//! Prepare target CREATE TABLE and deferred foreign keys without executing DDL.
//!
//! The transfer pass and the structure-only SQL preview share this planning helper, so both
//! always agree on the DDL that will run and the names it will use. Nothing here executes
//! DDL: execution stays with `execute_transfer_create_table_ddl_on_pool` and the caller. The
//! preview is still a plan, not a frozen script — the transfer pass re-reads source and
//! target metadata (and re-runs its fail-closed checks) when it starts.

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PreparedTableDdl {
    pub ddl: String,
    pub reused_source_ddl: bool,
    pub deferred_fk_alters: Vec<String>,
    pub deferred_fk_names: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare_table_ddl(
    state: &AppState,
    request: &TransferRequest,
    table: &str,
    target_table: &str,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    columns: &[db::ColumnInfo],
    table_comment: Option<&str>,
    known_foreign_keys: &HashMap<String, Vec<db::ForeignKeyInfo>>,
) -> Result<PreparedTableDdl, String> {
    let rebuild = request.drop_target_before_create;

    let foreign_keys = if let Some(foreign_keys) = known_foreign_keys.get(table) {
        foreign_keys.clone()
    } else if rebuild {
        // Strict for rebuilds: silently dropping foreign keys the inspection failed on
        // would produce a rebuilt table that enforces fewer constraints than the source.
        crate::schema::list_foreign_keys_core(
            state,
            &request.source_connection_id,
            &request.source_database,
            &request.source_schema,
            table,
        )
        .await
        .map_err(|e| format!("Failed to inspect source foreign keys for table '{table}' before rebuilding: {e}"))?
    } else if supports_deferred_mysql_foreign_keys(target_db_type) {
        // Ordinary MySQL-family targets keep the legacy best-effort behavior: fall back
        // to the generated DDL without foreign keys when the inspection fails. Other
        // targets never use the foreign keys, so no query is issued at all.
        match crate::schema::list_foreign_keys_core(
            state,
            &request.source_connection_id,
            &request.source_database,
            &request.source_schema,
            table,
        )
        .await
        {
            Ok(foreign_keys) => foreign_keys,
            Err(e) => {
                log::warn!("[transfer] failed to inspect source foreign keys for {table}: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let (source_driver_profile, target_driver_profile) = {
        let configs = state.configs.read().await;
        (
            configs.get(&request.source_connection_id).and_then(|config| config.driver_profile.clone()),
            configs.get(&request.target_connection_id).and_then(|config| config.driver_profile.clone()),
        )
    };
    let can_reuse = can_reuse_source_table_ddl(
        source_db_type,
        target_db_type,
        source_driver_profile.as_deref(),
        target_driver_profile.as_deref(),
        target_table == table,
    ) && (true);

    // A rebuild recreates the source structure exactly. When the structure comes from
    // the source DDL (the `can_reuse` path) the column list is only used for data
    // mapping, so an empty list is fine — the DDL is read directly from the source. Only
    // when the DDL must be *generated* from the column list does incomplete metadata
    // become fatal, because there would be nothing to build the CREATE TABLE from.
    if rebuild && !can_reuse {
        let incomplete = columns.is_empty()
            || columns.iter().any(|column| column.name.trim().is_empty() || column.data_type.trim().is_empty());
        if incomplete {
            return Err(format!(
                "Cannot rebuild table '{target_table}': the source returned incomplete column metadata, and the \
                 target cannot reuse the source DDL."
            ));
        }
    }

    let mut reused_source_ddl = false;
    let ddl = if can_reuse {
        let (source_ddl, source_ddl_was_read) = {
            match crate::schema::get_table_ddl_core(
                state,
                &request.source_connection_id,
                &request.source_database,
                &request.source_schema,
                table,
                None,
            )
            .await
            {
                Ok(ddl) => (ddl, true),
                Err(e) if rebuild || false => {
                    // Rebuilds and H2-to-H2 transfers must not silently discard source
                    // constraints or generated columns when native DDL cannot be read.
                    return Err(format!("Failed to read source DDL for table '{table}' before creating target: {e}"));
                }
                Err(_) => (
                    generate_create_table_ddl_with_column_quoting(
                        columns,
                        target_table,
                        &request.source_schema,
                        &request.target_schema,
                        target_db_type,
                        source_db_type,
                        table_comment,
                        request.target_catalog.as_deref(),
                        request.quote_target_column_names,
                    ),
                    false,
                ),
            }
        };
        if let Some(rewritten) = rewrite_transfer_source_table_ddl(
            &source_ddl,
            &request.source_schema,
            &request.target_schema,
            source_db_type,
            target_db_type,
            table,
            target_table,
        ) {
            reused_source_ddl = source_ddl_was_read;
            rewritten
        } else {
            // The reused DDL's CREATE TABLE head could not be renamed safely; fall back
            // to the generated DDL rather than creating the table under the wrong name.
            generate_create_table_ddl_with_column_quoting(
                columns,
                target_table,
                &request.source_schema,
                &request.target_schema,
                target_db_type,
                source_db_type,
                table_comment,
                request.target_catalog.as_deref(),
                request.quote_target_column_names,
            )
        }
    } else {
        generate_create_table_ddl_with_column_quoting(
            columns,
            target_table,
            &request.source_schema,
            &request.target_schema,
            target_db_type,
            source_db_type,
            table_comment,
            request.target_catalog.as_deref(),
            request.quote_target_column_names,
        )
    };

    // MySQL-family targets: defer the foreign keys to ALTER statements so the table
    // creation order never has to satisfy foreign key dependencies (a foreign key cycle
    // has no valid CREATE TABLE order at all). The rebuild path above guarantees the
    // metadata behind `foreign_keys` is trustworthy before anything is stripped.
    let mut ddl = ddl;
    let mut deferred_fk_alters = Vec::new();
    let mut deferred_fk_names = Vec::new();
    if supports_deferred_mysql_foreign_keys(target_db_type) && !foreign_keys.is_empty() {
        ddl = strip_inline_foreign_key_constraint_lines(&ddl);
        deferred_fk_names = group_foreign_keys_by_constraint_name(&foreign_keys)
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect();
        deferred_fk_alters =
            generate_mysql_foreign_key_alter_statements(&foreign_keys, request, target_table, target_db_type);
    }

    Ok(PreparedTableDdl { ddl, reused_source_ddl, deferred_fk_alters, deferred_fk_names })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(rebuild: bool) -> TransferRequest {
        serde_json::from_value(json!({
            "transferId": "ddl-plan-test",
            "sourceConnectionId": "source", "sourceDatabase": "main", "sourceSchema": "main",
            "targetConnectionId": "target", "targetDatabase": "main", "targetSchema": "main",
            "tables": ["child", "Parent"], "createTable": true, "content": "structureAndData",
            "mode": "append", "batchSize": 10, "dropTargetBeforeCreate": rebuild,
            "dropTargetConfirmed": true
        }))
        .unwrap()
    }

    fn columns() -> Vec<db::ColumnInfo> {
        vec![
            db::ColumnInfo {
                name: "id".into(),
                data_type: "INTEGER".into(),
                is_primary_key: true,
                ..Default::default()
            },
            db::ColumnInfo { name: "parent_id".into(), data_type: "INTEGER".into(), ..Default::default() },
        ]
    }
}
