pub mod table_structure_sql;

use crate::connection::{
    connection_url_for_endpoint, database_connection_config, task_client_session_id, uses_metadata_gate, AppState,
    MysqlMode, PoolKind, METADATA_POOL_ACQUIRE_TIMEOUT,
};
use crate::db;
use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::query::{should_discard_pool_after_error, QueryExecutionOptions};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[cfg(all(test, unix))]
mod external_table_filter_tests;

pub mod plugin_metadata;

macro_rules! extract_pool {
    ($pool:expr, $variant:ident) => {
        $pool.and_then(|v| match v {
            PoolKind::$variant(val) => Some(val.clone()),
            _ => None,
        })
    };
}

async fn clone_metadata_pool(state: &AppState, pool_key: &str) -> Option<PoolKind> {
    state.pool_handle(pool_key).await
}

struct EphemeralAgentMetadataSession {
    client_session_id: Option<String>,
    cleanup_guard: Option<crate::connection::ClientSessionPoolCleanupGuard>,
}

impl EphemeralAgentMetadataSession {
    async fn open(state: &AppState, connection_id: &str, database: Option<&str>, task_kind: &str) -> Self {
        let db_config = connection_config(state, connection_id).await;
        let client_session_id = ephemeral_agent_metadata_session_id(db_config.as_ref(), task_kind);
        let cleanup_guard = match client_session_id.as_deref() {
            Some(client_session_id) => {
                state.metadata_session_pool_cleanup_guard(connection_id, database, client_session_id).await
            }
            None => None,
        };
        Self { client_session_id, cleanup_guard }
    }

    fn client_session_id(&self) -> Option<&str> {
        self.client_session_id.as_deref()
    }

    async fn finish(mut self, state: &AppState, connection_id: &str, database: Option<&str>) {
        if close_ephemeral_agent_metadata_session(state, connection_id, database, self.client_session_id()).await {
            if let Some(cleanup_guard) = self.cleanup_guard.as_mut() {
                cleanup_guard.disarm();
            }
        }
    }
}

pub use dbx_types::metadata_filter::{
    sql_like_pattern_matches_case_insensitive, table_name_filter_matches, TableNameFilter,
};

fn mysql_database_list_timeout(config: Option<&ConnectionConfig>) -> Duration {
    config
        .map(|config| Duration::from_secs(config.effective_connect_timeout_secs()))
        .unwrap_or_else(db::connection_timeout)
}

pub async fn list_databases_core(state: &AppState, connection_id: &str) -> Result<Vec<db::DatabaseInfo>, String> {
    retry_metadata_connection(state, connection_id, None, || list_databases_once(state, connection_id)).await
}

/// Loads the more expensive database-level properties needed only by the
/// connection resource browser. General metadata paths keep using
/// `list_databases_core`, which only enumerates names.
pub async fn list_database_metadata_core(
    state: &AppState,
    connection_id: &str,
) -> Result<Vec<db::DatabaseInfo>, String> {
    retry_metadata_connection(state, connection_id, None, || list_database_metadata_once(state, connection_id)).await
}

pub async fn list_database_storage_core(
    state: &AppState,
    connection_id: &str,
    database_names: &[String],
) -> Result<Vec<db::DatabaseStorageInfo>, String> {
    retry_metadata_connection(state, connection_id, None, || {
        list_database_storage_once(state, connection_id, database_names)
    })
    .await
}

async fn list_database_storage_once(
    state: &AppState,
    connection_id: &str,
    database_names: &[String],
) -> Result<Vec<db::DatabaseStorageInfo>, String> {
    const DATABASE_STORAGE_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_DATABASE_STORAGE_NAMES: usize = 2048;

    if database_names.is_empty() {
        return Ok(Vec::new());
    }
    let config = connection_config(state, connection_id).await;
    {
        return Ok(Vec::new());
    }
}

async fn list_databases_once(state: &AppState, connection_id: &str) -> Result<Vec<db::DatabaseInfo>, String> {
    log::info!("[list_databases] connection_id={connection_id}");
    let db_config = connection_config(state, connection_id).await;
    {
        let pool_handle = state.pool_handle(connection_id).await;
        {}
        {}
        {}
        {}
        {}
        {}
        {};
        {}
    }

    let db_config = connection_config(state, connection_id).await;
    let mysql_database_list_timeout = mysql_database_list_timeout(db_config.as_ref());
    let pool = clone_metadata_pool(state, connection_id).await.ok_or("Connection not found")?;

    match &pool {
        PoolKind::Mysql(p, _) => db::mysql::list_databases_with_timeout(p, mysql_database_list_timeout).await,

        _ => Ok(vec![]),
    }
}

async fn list_database_metadata_once(state: &AppState, connection_id: &str) -> Result<Vec<db::DatabaseInfo>, String> {
    let config = connection_config(state, connection_id).await;
    {}
    let pool_handle = state.pool_handle(connection_id).await;
    {}
    if let Some(PoolKind::Mysql(pool, mode)) = pool_handle.as_ref() {
        let pool = pool.clone();
        let mode = *mode;
        return { db::mysql::list_database_metadata(&pool).await };
    }
    {}
    list_databases_once(state, connection_id).await
}

pub async fn list_schemas_core(state: &AppState, connection_id: &str, database: &str) -> Result<Vec<String>, String> {
    list_schemas_core_with_visible_filter(state, connection_id, database, false).await
}

pub async fn list_schemas_core_with_visible_filter(
    state: &AppState,
    connection_id: &str,
    database: &str,
    apply_visible_filter: bool,
) -> Result<Vec<String>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || {
        list_schemas_once(state, connection_id, database, apply_visible_filter)
    })
    .await
}

pub async fn list_schema_infos_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
) -> Result<Vec<db::SchemaInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || {
        list_schema_infos_once(state, connection_id, database)
    })
    .await
}

async fn list_schema_infos_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
) -> Result<Vec<db::SchemaInfo>, String> {
    let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
    let db_config = connection_config(state, connection_id).await;
    let show_system_schemas = db_config.as_ref().is_some_and(|config| config.show_system_schemas);
    {}

    let schemas = list_schemas_once(state, connection_id, database, false).await?;
    Ok(schemas.into_iter().map(|name| db::SchemaInfo { name, comment: None }).collect())
}

pub async fn list_data_types_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
) -> Result<Vec<String>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        {}
        Ok(Vec::new())
    })
    .await
}

async fn list_schemas_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    apply_visible_filter: bool,
) -> Result<Vec<String>, String> {
    let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
    let db_config = connection_config(state, connection_id).await;
    let show_system_schemas = db_config.as_ref().is_some_and(|config| config.show_system_schemas);
    let visible_schema_filter = visible_schema_filter(db_config.as_ref(), database, apply_visible_filter);

    {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        {};
        {}
    }

    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

    match &pool {
        _ => Ok(vec![]),
    }
}

fn visible_schema_filter(
    config: Option<&ConnectionConfig>,
    database: &str,
    apply_visible_filter: bool,
) -> Option<Vec<String>> {
    if !apply_visible_filter {
        return None;
    }
    config?.visible_schemas.as_ref()?.get(database).cloned()
}

fn filter_visible_schema_names(schemas: Vec<String>, visible: Option<&[String]>) -> Vec<String> {
    let Some(visible) = visible else {
        return schemas;
    };
    let visible: std::collections::HashSet<&str> = visible.iter().map(String::as_str).collect();
    schemas.into_iter().filter(|schema| visible.contains(schema.as_str())).collect()
}

pub async fn list_tables_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
) -> Result<Vec<db::TableInfo>, String> {
    let metadata_session = EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "tables").await;
    let result = retry_metadata_connection_for_session(
        state,
        connection_id,
        Some(database),
        metadata_session.client_session_id(),
        || {
            list_tables_once(
                state,
                connection_id,
                database,
                schema,
                filter,
                limit,
                offset,
                object_types,
                table_name_filter,
                metadata_session.client_session_id(),
            )
        },
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

pub async fn get_table_comment_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Option<String>, String> {
    {}

    let metadata_session =
        EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "table-comment").await;
    let result = get_table_comment_core_for_session(
        state,
        connection_id,
        database,
        schema,
        table,
        metadata_session.client_session_id(),
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

pub async fn get_mysql_table_auto_increment_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    table: &str,
) -> Result<Option<String>, String> {
    let db_config = connection_config(state, connection_id).await;
    let native_mysql = db_config.as_ref().is_some_and(|config| {
        true && config
            .driver_profile
            .as_deref()
            .map(str::trim)
            .is_none_or(|profile| profile.is_empty() || profile.eq_ignore_ascii_case("mysql"))
    });
    if !native_mysql {
        return Err("AUTO_INCREMENT metadata is supported only for native MySQL connections.".to_string());
    }

    retry_metadata_connection_for_session(state, connection_id, Some(database), None, || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
        match &pool {
            PoolKind::Mysql(pool, mode) if true => db::mysql::get_table_auto_increment(pool, database, table).await,
            _ => Err("AUTO_INCREMENT metadata is supported only for native MySQL connections.".to_string()),
        }
    })
    .await
}

async fn get_table_comment_core_for_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
) -> Result<Option<String>, String> {
    retry_metadata_connection_for_session(state, connection_id, Some(database), client_session_id, || async {
        let pool_key =
            state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
        let db_config = connection_config(state, connection_id).await;

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
            {}
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            PoolKind::Mysql(p, mode) if true => db::mysql::get_table_comment(p, database, table).await,

            _ => Err("Table comment lookup is not supported for this connection".to_string()),
        }
    })
    .await
}

fn table_comments_from_query_result(result: db::QueryResult) -> HashMap<String, String> {
    result
        .rows
        .into_iter()
        .filter_map(|row| {
            let name = row.first()?.as_str()?.to_string();
            let comment = row.get(1)?.as_str()?.trim().to_string();
            (!name.is_empty() && !comment.is_empty()).then_some((name, comment))
        })
        .collect()
}

const ORACLE_SYNONYM_MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct OracleObjectRef {
    owner: String,
    name: String,
}

struct OracleSynonymResolver {
    current: OracleObjectRef,
    visited: HashSet<OracleObjectRef>,
    depth: usize,
}

impl OracleSynonymResolver {
    fn new(owner: String, name: String) -> Self {
        let current = OracleObjectRef { owner, name };
        Self { current: current.clone(), visited: HashSet::from([current]), depth: 0 }
    }

    fn current(&self) -> &OracleObjectRef {
        &self.current
    }

    fn can_follow(&self) -> bool {
        self.depth < ORACLE_SYNONYM_MAX_DEPTH
    }

    fn follow(&mut self, target: OracleObjectRef) -> bool {
        if !self.can_follow() || !self.visited.insert(target.clone()) {
            return false;
        }
        self.current = target;
        self.depth += 1;
        true
    }
}

/// One attempt of an object-statistics fallback chain: a log label, the SQL to
/// run, and whether an empty (but successful) result should be accepted instead
/// of falling through to the next attempt.
type ObjectStatisticsAttempt = (&'static str, String, bool);

/// Vendors whose object statistics can be collected over a generic JDBC
/// connection (`PoolKind::ExternalDriver`).
///
/// The bundled JDBC plugin exposes no `listObjectStatistics` RPC, but the
/// vendor statistics SQL is plain SQL that runs fine through `executeQuery`, so
/// the only missing piece is recognising which database sits behind the JDBC
/// URL. The prefixes mirror `DbxJdbcPlugin.driverQuirks`, which picks its own
/// metadata dialect the same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExternalDriverStatisticsDialect {}

async fn list_tables_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
    client_session_id: Option<&str>,
) -> Result<Vec<db::TableInfo>, String> {
    let pool_key =
        state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
    let db_config = connection_config(state, connection_id).await;

    {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}

        {}
        {}
        {}
        {}
        {}
        if requests_table_objects_only(object_types) && table_name_filter.is_none_or(TableNameFilter::is_empty) {
            {}
        }
        if object_types.is_some() || table_name_filter.is_some_and(|filter| !filter.is_empty()) {
            {}
        }
        {};
        {}
    }

    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

    match &pool {
        PoolKind::Mysql(p, mode) => {
            if mysql_table_list_source_for_config(db_config.as_ref()) == MysqlTableListSource::ShowFullTables {
                db::mysql::list_logical_tables_show(p, mysql_table_metadata_catalog(database, schema))
                    .await
                    .map(|tables| filter_table_infos(tables, filter, limit, offset, object_types, table_name_filter))
            } else {
                db::mysql::list_tables_filtered(
                    p,
                    mysql_table_metadata_catalog(database, schema),
                    filter,
                    limit,
                    offset,
                    object_types,
                    table_name_filter,
                )
                .await
                .map(|tables| filter_table_infos(tables, None, None, None, object_types, None))
            }
        }

        _ => Ok(vec![]),
    }
}

fn collection_names_to_tables(names: Vec<String>, table_type: &str) -> Vec<db::TableInfo> {
    names
        .into_iter()
        .map(|name| db::TableInfo {
            name,
            table_type: table_type.to_string(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        })
        .collect()
}

fn filter_table_infos(
    tables: Vec<db::TableInfo>,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
) -> Vec<db::TableInfo> {
    let filter = filter.unwrap_or("");
    let limit = limit.unwrap_or(usize::MAX);
    let offset = offset.unwrap_or(0);
    tables
        .into_iter()
        .filter(|table| metadata_name_or_comment_matches(&table.name, table.comment.as_deref(), filter))
        .filter(|table| table_name_filter_matches(&table.name, table_name_filter))
        .filter(|table| table_info_matches_object_types(table, object_types))
        .skip(offset)
        .take(limit)
        .collect()
}

fn filter_object_infos(
    objects: Vec<db::ObjectInfo>,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
) -> Vec<db::ObjectInfo> {
    let filter = filter.unwrap_or("");
    let limit = limit.unwrap_or(usize::MAX);
    let offset = offset.unwrap_or(0);
    objects
        .into_iter()
        .filter(|object| metadata_name_or_comment_matches(&object.name, object.comment.as_deref(), filter))
        .filter(|object| table_name_filter_matches(&object.name, table_name_filter))
        .filter(|object| object_info_matches_object_types(object, object_types))
        .skip(offset)
        .take(limit)
        .collect()
}

fn metadata_name_or_comment_matches(name: &str, comment: Option<&str>, filter: &str) -> bool {
    if filter.trim().is_empty() {
        return true;
    }
    crate::sql::contains_or_fuzzy_match(name, filter)
        || comment.is_some_and(|comment| crate::sql::contains_or_fuzzy_match(comment, filter))
}

fn object_info_matches_object_types(object: &db::ObjectInfo, object_types: Option<&[String]>) -> bool {
    let Some(object_types) = object_types else {
        return true;
    };
    if object_types.is_empty() {
        return true;
    }
    let object_type = normalize_object_info_object_type(&object.object_type);
    object_types.iter().any(|expected| normalize_object_info_object_type(expected) == object_type)
}

fn normalize_object_info_object_type(value: &str) -> String {
    let upper = value.to_ascii_uppercase().replace(' ', "_");
    if upper.contains("MATERIALIZED") && upper.contains("VIEW") {
        return "MATERIALIZED_VIEW".to_string();
    }
    if upper == "BASE_TABLE" || upper.contains("TABLE") {
        return "TABLE".to_string();
    }
    if upper.contains("VIEW") {
        return "VIEW".to_string();
    }
    upper
}

fn table_info_matches_object_types(table: &db::TableInfo, object_types: Option<&[String]>) -> bool {
    let Some(object_types) = object_types else {
        return true;
    };
    if object_types.is_empty() {
        return true;
    }
    let table_type = normalize_table_info_object_type(&table.table_type);
    object_types.iter().any(|object_type| normalize_table_info_object_type(object_type) == table_type)
}

fn normalize_table_info_object_type(value: &str) -> String {
    let upper = value.to_ascii_uppercase().replace(' ', "_");
    if upper.contains("MATERIALIZED") && upper.contains("VIEW") {
        return "MATERIALIZED_VIEW".to_string();
    }
    if upper.contains("VIEW") {
        return "VIEW".to_string();
    }
    if upper.contains("COLLECTION") {
        return "COLLECTION".to_string();
    }
    if upper.contains("INDEX") {
        return "INDEX".to_string();
    }
    "TABLE".to_string()
}

fn requests_table_objects_only(object_types: Option<&[String]>) -> bool {
    object_types.is_some_and(|types| types.len() == 1 && normalize_table_info_object_type(&types[0]) == "TABLE")
}

fn query_result_cell_string(row: &[serde_json::Value], index: usize) -> Option<String> {
    let value = row.get(index)?;
    if value.is_null() {
        return None;
    }
    value.as_str().map(ToString::to_string).or_else(|| Some(value.to_string()))
}

fn normalize_information_schema_table_type(table_type: &str) -> String {
    match table_type.trim().to_ascii_uppercase().replace(' ', "_").as_str() {
        "BASE_TABLE" => "TABLE".to_string(),
        "VIEW" => "VIEW".to_string(),
        "MATERIALIZED_VIEW" => "MATERIALIZED_VIEW".to_string(),
        _ => table_type.to_string(),
    }
}

fn mysql_table_metadata_catalog<'a>(database: &'a str, schema: &'a str) -> &'a str {
    if schema.trim().is_empty() {
        database
    } else {
        schema
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MysqlTableListSource {
    InformationSchema,
    ShowFullTables,
}

fn is_shardingsphere_proxy_version(version: &str) -> bool {
    const MARKER: &[u8] = b"shardingsphere-proxy";
    version.as_bytes().windows(MARKER.len()).any(|window| window.eq_ignore_ascii_case(MARKER))
}

fn mysql_table_list_source_for_config(config: Option<&ConnectionConfig>) -> MysqlTableListSource {
    if false
        || config
            .and_then(|config| config.database_info.as_ref())
            .and_then(|info| info.product_version.as_deref())
            .is_some_and(is_shardingsphere_proxy_version)
    {
        MysqlTableListSource::ShowFullTables
    } else {
        MysqlTableListSource::InformationSchema
    }
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::connection::{AppState, PoolKind};
    use crate::models::connection::{ConnectionConfig, DatabaseConnectionInfo, DatabaseType};
    #[cfg(unix)]
    use crate::plugins::{
        InstalledPlugin, PluginDriverManifest, PluginDriverSession, PluginManifest, PluginRuntimeEnv,
    };
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn reference_keys_require_effective_unfiltered_plain_unique_columns() {
        let index = |name: &str, columns: &[&str], is_unique: bool, is_primary: bool| db::IndexInfo {
            name: name.to_string(),
            columns: columns.iter().map(|column| (*column).to_string()).collect(),
            is_unique,
            is_primary,
            filter: None,
            index_type: None,
            included_columns: None,
            comment: None,
            key_is_expression: Vec::new(),
            column_opclasses: vec![],
            key_options: Vec::new(),
            constraint_backed: false,
        };
        let mut filtered = index("uq_active_code", &["active_code"], true, false);
        filtered.filter = Some("active = true".to_string());
        let mut expression = index("uq_lower_email", &["lower(email)"], true, false);
        expression.key_is_expression = vec![true];
        let mut composite_expression = index("uq_tenant_lower_code", &["tenant_id", "lower(code)"], true, false);
        composite_expression.key_is_expression = vec![false, true];
        let indexes = vec![
            index("clickhouse_primary", &["event_id"], false, true),
            index("uq_code", &["code"], true, false),
            index("uq_Code", &["Code"], true, false),
            index("uq_tenant_code", &["tenant_id", "code"], true, false),
            index("uq_tenant_code_duplicate", &["tenant_id", "code"], true, false),
            index("uq_empty", &[""], true, false),
            index("uq_repeated", &["tenant_id", "tenant_id"], true, false),
            filtered,
            expression,
            composite_expression,
        ];

        assert_eq!(
            reference_keys_from_indexes(&indexes),
            vec![
                ReferenceKeyInfo { columns: vec!["code".to_string()] },
                ReferenceKeyInfo { columns: vec!["Code".to_string()] },
                ReferenceKeyInfo { columns: vec!["tenant_id".to_string(), "code".to_string()] },
            ]
        );
        assert_eq!(reference_key_columns_from_indexes(&indexes), vec!["code", "Code"]);
    }

    fn test_column(name: &str, comment: Option<&str>, is_primary_key: bool) -> super::db::ColumnInfo {
        super::db::ColumnInfo {
            name: name.to_string(),
            data_type: "VARCHAR".to_string(),
            is_nullable: true,
            column_default: None,
            is_primary_key,
            extra: None,
            comment: comment.map(|value| value.to_string()),
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            enum_values: None,
            ..Default::default()
        }
    }

    fn test_connection_config(db_type: DatabaseType) -> ConnectionConfig {
        ConnectionConfig {
            oracle_oci_nls_lang: None,
            oracle_oci_tns_admin: None,
            docs_notes_path: None,
            id: "test".to_string(),
            name: "test".to_string(),
            note: String::new(),
            db_type,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            agent_java_options: Vec::new(),
            host: "127.0.0.1".to_string(),
            port: 5432,
            username: "user".to_string(),
            password: "secret".to_string(),
            database: Some("demo".to_string()),
            default_schema: None,
            visible_databases: None,
            visible_database_patterns: None,
            visible_schemas: None,
            show_system_schemas: false,
            sidebar_auto_load_all_tables: false,
            attached_databases: Vec::new(),
            init_script: None,
            color: None,
            transport_layers: Vec::new(),
            connect_timeout_secs: 5,
            query_timeout_secs: 30,
            idle_timeout_secs: 60,
            keepalive_interval_secs: 0,
            ssl: false,
            ca_cert_path: String::new(),
            client_cert_path: String::new(),
            client_key_path: String::new(),
            sysdba: false,
            oracle_connection_type: None,
            connection_string: None,
            redis_connection_mode: None,
            redis_sentinel_master: String::new(),
            redis_sentinel_nodes: String::new(),
            redis_sentinel_username: String::new(),
            redis_sentinel_password: String::new(),
            redis_sentinel_tls: false,
            redis_cluster_nodes: String::new(),
            redis_key_separator: crate::models::connection::default_redis_key_separator(),
            redis_scan_page_size: None,
            redis_database_aliases: Default::default(),
            redis_key_templates: Vec::new(),
            redis_key_filter: None,
            redis_key_grouping: None,
            etcd_endpoints: String::new(),
            gbase_server: String::new(),
            informix_server: String::new(),
            external_config: None,
            plugin_id: None,
            plugin_connection_provider: None,
            plugin_connection_type: None,
            connection_secrets: HashMap::new(),
            jdbc_driver_class: None,
            jdbc_driver_paths: Vec::new(),
            one_time: false,
            save_password: true,
            read_only: false,
            is_production: false,
            production_databases: vec![],
            database_info: None,
        }
    }

    #[test]
    fn mysql_database_list_timeout_uses_configured_and_effective_bounds() {
        let mut config = test_connection_config(DatabaseType::Mysql);

        config.connect_timeout_secs = 10;
        assert_eq!(mysql_database_list_timeout(Some(&config)), Duration::from_secs(10));

        config.connect_timeout_secs = 0;
        assert_eq!(
            mysql_database_list_timeout(Some(&config)),
            Duration::from_secs(crate::models::connection::default_connect_timeout_secs())
        );

        config.connect_timeout_secs = 500;
        assert_eq!(mysql_database_list_timeout(Some(&config)), Duration::from_secs(300));
        assert_eq!(mysql_database_list_timeout(None), db::connection_timeout());
    }

    #[test]
    fn object_types_include_custom_types_only_when_unfiltered_or_type_requested() {
        assert!(object_types_include_custom_types(None));
        assert!(object_types_include_custom_types(Some(&["TYPE".to_string()])));
        assert!(object_types_include_custom_types(Some(&["type_body".to_string()])));
        assert!(object_types_include_custom_types(Some(&["table".to_string(), "type".to_string()])));
        assert!(!object_types_include_custom_types(Some(&["TABLE".to_string()])));
        assert!(!object_types_include_custom_types(Some(&["FUNCTION".to_string()])));
    }

    #[test]
    fn object_types_include_sequences_only_for_sequence_requests() {
        assert!(object_types_include_sequences(None));
        assert!(object_types_include_sequences(Some(&["SEQUENCE".to_string()])));
        assert!(object_types_include_sequences(Some(&["sequence".to_string()])));
        assert!(object_types_include_sequences(Some(&["TABLE".to_string(), "SEQUENCE".to_string()])));
        assert!(!object_types_include_sequences(Some(&["TABLE".to_string()])));
        assert!(!object_types_include_sequences(Some(&[])));
    }

    #[test]
    fn object_types_select_independent_catalog_branches() {
        assert!(object_types_include_relations(None));
        assert!(object_types_include_routines(None));
        assert!(object_types_include_custom_types(None));

        assert!(object_types_include_relations(Some(&["TABLE".to_string()])));
        assert!(object_types_include_relations(Some(&["VIEW".to_string()])));
        assert!(object_types_include_relations(Some(&["SEQUENCE".to_string()])));
        assert!(!object_types_include_routines(Some(&["TABLE".to_string()])));
        assert!(!object_types_include_custom_types(Some(&["TABLE".to_string()])));

        assert!(object_types_include_routines(Some(&["PROCEDURE".to_string()])));
        assert!(object_types_include_routines(Some(&["FUNCTION".to_string()])));
        assert!(!object_types_include_relations(Some(&["FUNCTION".to_string()])));
        assert!(!object_types_include_custom_types(Some(&["FUNCTION".to_string()])));

        assert!(object_types_include_custom_types(Some(&["TYPE".to_string()])));
        assert!(!object_types_include_relations(Some(&["TYPE".to_string()])));
        assert!(!object_types_include_routines(Some(&["TYPE".to_string()])));

        // The sidebar type group sends the TYPE_BODY companion kind as well;
        // it must select the type branch alone, never relations or routines.
        let type_group = ["TYPE".to_string(), "TYPE_BODY".to_string()];
        assert!(object_types_include_custom_types(Some(&type_group)));
        assert!(!object_types_include_relations(Some(&type_group)));
        assert!(!object_types_include_routines(Some(&type_group)));
    }

    #[test]
    fn object_types_only_custom_types_detects_dedicated_type_requests() {
        assert!(!object_types_only_custom_types(None));
        assert!(object_types_only_custom_types(Some(&["TYPE".to_string()])));
        assert!(object_types_only_custom_types(Some(&["TYPE".to_string(), "TYPE_BODY".to_string()])));
        assert!(object_types_only_custom_types(Some(&["type_body".to_string()])));
        assert!(!object_types_only_custom_types(Some(&["TYPE".to_string(), "TABLE".to_string()])));
        assert!(!object_types_only_custom_types(Some(&["TABLE".to_string()])));
        assert!(!object_types_only_custom_types(Some(&[])));
    }

    #[test]
    fn mysql_table_child_metadata_prefers_schema_when_present() {
        assert_eq!(mysql_table_metadata_catalog("app_db", ""), "app_db");
        assert_eq!(mysql_table_metadata_catalog("app_db", "tenant_db"), "tenant_db");
    }

    #[cfg(unix)]
    mod oracle_ddl_regression_tests {
        use super::*;
        use crate::types::ObjectSourceKind;
        use serde_json::{json, Value};
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        fn column_response() -> Value {
            json!({"result": [db::ColumnInfo {
                name: "ID".into(), comment: Some("Column's comment".into()), ..Default::default()
            }]})
        }

        fn comment_response() -> Value {
            json!({"result": {"columns": ["COMMENTS"], "rows": [["View's comment"]], "affected_rows": 0, "execution_time_ms": 0}})
        }

        fn driver_calls(dir: &std::path::Path) -> Vec<Value> {
            std::fs::read_to_string(dir.join("calls.log"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }

        #[tokio::test]
        async fn missing_or_failed_source_stops_before_comment_lookups() {
            for (object_type, response) in [
                (None, json!({"result": {}})),
                (None, json!({"result": {"source": "  "}})),
                (None, json!({"error": {"message": "source denied"}})),
                (Some(ObjectSourceKind::View), json!({"error": {"message": "source denied"}})),
            ] {
                let (state, dir) = scripted_oracle_driver(response, column_response(), comment_response()).await;
                let result = crate::schema::get_table_display_ddl_core(
                    &state,
                    "oracle-ddl",
                    "demo",
                    "hr",
                    "ORDERS",
                    object_type,
                )
                .await;
                assert!(result.is_err());
                assert!(driver_calls(&dir).iter().all(|call| call["method"] == "getObjectSource"));
                state.shutdown(Duration::from_secs(1)).await;
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }

    #[test]
    fn mysql_object_source_sql_qualifies_cross_database_objects() {
        assert_eq!(
            mysql_object_source_sql("tenant_db", "users_view", &db::ObjectSourceKind::View),
            "SHOW CREATE VIEW `tenant_db`.`users_view`"
        );
        assert_eq!(
            mysql_object_source_sql("tenant_db", "sync_users", &db::ObjectSourceKind::Procedure),
            "SHOW CREATE PROCEDURE `tenant_db`.`sync_users`"
        );
        assert_eq!(
            mysql_object_source_sql("tenant_db", "calc_score", &db::ObjectSourceKind::Function),
            "SHOW CREATE FUNCTION `tenant_db`.`calc_score`"
        );
        assert_eq!(
            mysql_object_source_sql("", "users_view", &db::ObjectSourceKind::View),
            "SHOW CREATE VIEW `users_view`"
        );
    }

    #[test]
    fn mysql_object_source_sql_emits_show_create_trigger() {
        assert_eq!(
            mysql_object_source_sql("tenant_db", "before_insert", &db::ObjectSourceKind::Trigger),
            "SHOW CREATE TRIGGER `tenant_db`.`before_insert`"
        );
    }

    #[test]
    fn mysql_object_source_sql_emits_show_create_event() {
        assert_eq!(
            mysql_object_source_sql("tenant_db", "event_daily_sync", &db::ObjectSourceKind::Event),
            "SHOW CREATE EVENT `tenant_db`.`event_daily_sync`"
        );
    }

    #[test]
    fn mysql_event_object_source_is_read_only() {
        let source = finalize_object_source(db::ObjectSource {
            name: "event_daily_sync".to_string(),
            object_type: db::ObjectSourceKind::Event,
            schema: None,
            source: "CREATE EVENT event_daily_sync ON SCHEDULE EVERY 1 DAY DO SELECT 1".to_string(),
            editable: None,
            routine_parameters: None,
        });

        assert_eq!(source.editable, Some(false));
    }

    #[test]
    fn mysql_object_source_sql_emits_show_create_materialized_view() {
        // Regression for the review comment: Doris / StarRocks ride on the MySQL
        // protocol, so the MV branch of mysql_object_source_sql must produce a
        // real statement (used at crates/dbx-core/src/schema/mod.rs:5395-5404 by
        // get_table_ddl_core). Returning an empty string silently broke the UI.
        assert_eq!(
            mysql_object_source_sql("shop", "daily_sales_mv", &db::ObjectSourceKind::MaterializedView),
            "SHOW CREATE MATERIALIZED VIEW `shop`.`daily_sales_mv`"
        );
        assert_eq!(
            mysql_object_source_sql("", "daily_sales_mv", &db::ObjectSourceKind::MaterializedView),
            "SHOW CREATE MATERIALIZED VIEW `daily_sales_mv`"
        );
    }

    #[test]
    fn mysql_object_source_ddl_column_index_matches_dialect_layout() {
        // VIEW and Doris/StarRocks MaterializedView return (Name, DDL).
        // PROCEDURE / FUNCTION return (Name, sql_mode, DDL, …).
        // Reading the wrong index returns the empty/no-op and surfaces as
        // "Failed to read object source" — regression-guarded here so we
        // don't have to spin up a real StarRocks to catch it.
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::View), 1);
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::MaterializedView), 1);
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::Procedure), 2);
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::Function), 2);
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::Trigger), 2);
        // SHOW CREATE EVENT returns (Event, sql_mode, time_zone, Create Event, …) —
        // one extra time_zone column before the DDL, verified against a real MySQL
        // 8.0 instance (`SHOW CREATE EVENT` for a live event).
        assert_eq!(mysql_object_source_ddl_column_index(&db::ObjectSourceKind::Event), 3);
    }

    #[test]
    fn metadata_retry_excludes_pool_saturation_from_reconnects() {
        assert!(!is_retryable_metadata_error("Pool not found"));
        assert!(!is_retryable_metadata_error(crate::query::METADATA_POOL_BUSY_ERROR));
        assert!(is_retryable_metadata_error("connection reset by peer"));
        assert!(!is_retryable_metadata_error("Unknown column 'email' in 'field list'"));
        assert!(!is_retryable_metadata_error("Access denied for user"));
    }

    #[tokio::test]
    async fn metadata_pool_races_retry_once_and_saturation_returns_busy() {
        let dir = std::env::temp_dir().join(format!("dbx-schema-metadata-pool-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state = crate::connection::AppState::new(storage);
        state.configs.write().await.insert("conn".to_string(), test_connection_config(DatabaseType::Mysql));

        let mut recovered_attempts = 0;
        let recovered = super::retry_metadata_connection_for_session(&state, "conn", Some("app"), None, || {
            recovered_attempts += 1;
            let attempt = recovered_attempts;
            async move {
                if attempt == 1 {
                    Err("Pool not found".to_string())
                } else {
                    Ok("loaded")
                }
            }
        })
        .await;
        assert_eq!(recovered, Ok("loaded"));
        assert_eq!(recovered_attempts, 2);

        let mut missing_attempts = 0;
        let missing = super::retry_metadata_connection_for_session(&state, "conn", Some("app"), None, || {
            missing_attempts += 1;
            async { Err::<(), _>("Pool not found".to_string()) }
        })
        .await;
        assert_eq!(missing.err().as_deref(), Some(crate::query::METADATA_POOL_BUSY_ERROR));
        assert_eq!(missing_attempts, 2);

        let mut saturation_attempts = 0;
        let saturated = super::retry_metadata_connection_for_session(&state, "conn", Some("app"), None, || {
            saturation_attempts += 1;
            async { Err::<(), _>("MySQL connection pool checkout timed out [stage=wait, timeout_ms=500]".to_string()) }
        })
        .await;
        assert_eq!(saturated.err().as_deref(), Some(crate::query::METADATA_POOL_BUSY_ERROR));
        assert_eq!(saturation_attempts, 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn filter_visible_schema_names_preserves_database_order() {
        let schemas = vec!["APP".to_string(), "SYS".to_string(), "REPORTING".to_string()];
        let visible = vec!["REPORTING".to_string(), "APP".to_string()];

        assert_eq!(filter_visible_schema_names(schemas, Some(&visible)), vec!["APP", "REPORTING"]);
    }

    fn test_table_info(name: &str) -> super::db::TableInfo {
        super::db::TableInfo {
            name: name.to_string(),
            table_type: "BASE TABLE".to_string(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        }
    }

    fn test_object_info(name: &str, object_type: &str) -> super::db::ObjectInfo {
        super::db::ObjectInfo {
            name: name.to_string(),
            object_type: object_type.to_string(),
            schema: Some("app".to_string()),
            valid: None,
            signature: None,
            custom_type_kind: None,
            has_members: None,
            comment: None,
            created_at: None,
            updated_at: None,
            parent_schema: None,
            parent_name: None,
            trigger: None,
            xugu_type_members_expandable: None,
        }
    }

    #[test]
    fn filter_table_infos_applies_filter_offset_and_limit() {
        let tables = vec![
            test_table_info("alpha"),
            test_table_info("audit_log"),
            test_table_info("audit_record"),
            test_table_info("users"),
        ];

        let filtered = filter_table_infos(tables, Some("audit"), Some(1), Some(1), None, None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "audit_record");
    }

    #[test]
    fn filter_table_infos_matches_fuzzy_subsequences() {
        let tables = vec![test_table_info("system_user"), test_table_info("user_order"), test_table_info("alpha")];

        let system_user = filter_table_infos(tables.clone(), Some("sysu"), None, None, None, None);
        assert_eq!(system_user.into_iter().map(|table| table.name).collect::<Vec<_>>(), vec!["system_user"]);

        let user_order = filter_table_infos(tables, Some("uo"), None, None, None, None);
        assert_eq!(user_order.into_iter().map(|table| table.name).collect::<Vec<_>>(), vec!["user_order"]);
    }

    #[test]
    fn filter_table_infos_matches_comments() {
        let mut orders = test_table_info("orders");
        orders.comment = Some("sales archive".to_string());
        let mut profile = test_table_info("profile");
        profile.comment = Some("customer account data".to_string());
        let tables = vec![orders, profile, test_table_info("logs")];

        let filtered = filter_table_infos(tables, Some("account"), None, None, None, None);

        assert_eq!(filtered.into_iter().map(|table| table.name).collect::<Vec<_>>(), vec!["profile"]);
    }

    #[test]
    fn filter_table_infos_skips_fuzzy_for_single_character_filters() {
        let tables = vec![test_table_info("orders"), test_table_info("user_order")];

        let filtered = filter_table_infos(tables, Some("u"), None, None, None, None);

        assert_eq!(filtered.into_iter().map(|table| table.name).collect::<Vec<_>>(), vec!["user_order"]);
    }

    #[test]
    fn filter_table_infos_keeps_special_filter_characters_literal() {
        let tables = vec![test_table_info("user_%"), test_table_info("user_account"), test_table_info("userXpercent")];

        let filtered = filter_table_infos(tables, Some("user_%"), None, None, None, None);

        assert_eq!(filtered.into_iter().map(|table| table.name).collect::<Vec<_>>(), vec!["user_%"]);
    }

    #[test]
    fn table_name_filter_uses_sql_like_without_fuzzy_subsequence() {
        let filter = TableNameFilter {
            include_patterns: vec!["ads_cp%".to_string()],
            exclude_patterns: vec!["%_bak".to_string()],
        };

        assert!(table_name_filter_matches("ads_cp_report", Some(&filter)));
        assert!(!table_name_filter_matches("ads_180d_creator_detail_report_di", Some(&filter)));
        assert!(!table_name_filter_matches("ads_cp_report_bak", Some(&filter)));
    }

    #[test]
    fn table_name_filter_supports_escaped_like_wildcards() {
        let filter = TableNameFilter { include_patterns: vec![r"order\_%".to_string()], exclude_patterns: vec![] };

        assert!(table_name_filter_matches("order_items", Some(&filter)));
        assert!(!table_name_filter_matches("orderXitems", Some(&filter)));
    }

    #[test]
    fn table_name_filter_handles_adversarial_failing_like_pattern() {
        let filter = TableNameFilter { include_patterns: vec!["%a".repeat(128)], exclude_patterns: vec![] };

        assert!(!table_name_filter_matches(&"a".repeat(127), Some(&filter)));
    }

    #[test]
    fn filter_table_infos_filters_object_type_before_offset_and_limit() {
        let tables = vec![
            test_table_info("orders"),
            super::db::TableInfo {
                name: "active_orders".to_string(),
                table_type: "VIEW".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
            test_table_info("users"),
            super::db::TableInfo {
                name: "active_users".to_string(),
                table_type: "VIEW".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
        ];
        let object_types = vec!["VIEW".to_string()];

        let filtered = filter_table_infos(tables, None, Some(1), Some(1), Some(&object_types), None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "active_users");
    }

    #[test]
    fn filter_object_infos_applies_sql_like_name_filter() {
        let objects = vec![
            test_object_info("fn_get_user", "FUNCTION"),
            test_object_info("fn_get_role", "FUNCTION"),
            test_object_info("fn_get_role_bak", "FUNCTION"),
            test_object_info("internal_hash", "FUNCTION"),
        ];
        let object_types = vec!["FUNCTION".to_string()];
        let name_filter =
            TableNameFilter { include_patterns: vec!["FN_%".to_string()], exclude_patterns: vec!["%_BAK".to_string()] };

        let filtered = filter_object_infos(objects, None, None, None, Some(&object_types), Some(&name_filter));

        assert_eq!(
            filtered.into_iter().map(|object| object.name).collect::<Vec<_>>(),
            vec!["fn_get_user", "fn_get_role"]
        );
    }

    #[test]
    fn filter_object_infos_filters_object_type_before_offset_and_limit() {
        let objects = vec![
            test_object_info("sync_user", "PROCEDURE"),
            test_object_info("find_user", "FUNCTION"),
            test_object_info("fetch_name", "FUNCTION"),
            test_object_info("orders", "TABLE"),
        ];
        let object_types = vec!["FUNCTION".to_string()];

        let filtered = filter_object_infos(objects, Some("fn"), Some(1), Some(1), Some(&object_types), None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "fetch_name");
    }

    #[test]
    fn filter_object_infos_matches_comments() {
        let mut order_view = test_object_info("order_view", "VIEW");
        order_view.comment = Some("monthly revenue summary".to_string());
        let mut sync_user = test_object_info("sync_user", "PROCEDURE");
        sync_user.comment = Some("sync account records".to_string());
        let objects = vec![order_view, sync_user, test_object_info("audit_log", "TABLE")];

        let object_types = vec!["VIEW".to_string()];
        let filtered = filter_object_infos(objects, Some("revenue"), None, None, Some(&object_types), None);

        assert_eq!(filtered.into_iter().map(|object| object.name).collect::<Vec<_>>(), vec!["order_view"]);
    }

    #[test]
    fn detects_unsupported_agent_completion_assistant_errors() {
        assert!(super::is_agent_completion_assistant_unsupported(
            "Agent RPC error (-1): Unknown method: completion_assistant_search_v1"
        ));
        assert!(super::is_agent_completion_assistant_unsupported(
            "Agent RPC error (-1): unknown method: completion_assistant_search_v1"
        ));
        assert!(super::is_agent_completion_assistant_unsupported(
            "Agent RPC error (-1): Completion assistant search is not supported by this agent"
        ));
        assert!(!super::is_agent_completion_assistant_unsupported("Agent RPC error (-1): Connection failed"));
    }

    #[test]
    fn detects_unsupported_agent_partition_method_errors() {
        assert!(super::is_agent_partition_method_unsupported(
            "Agent RPC error (-1): unknown method: get_table_partitioning",
            "get_table_partitioning",
        ));
        assert!(super::is_agent_partition_method_unsupported(
            "Agent RPC error (-32601): Method not found: get_table_partition_status",
            "get_table_partition_status",
        ));
        // A different method in the same error must not match.
        assert!(!super::is_agent_partition_method_unsupported(
            "Agent RPC error (-1): unknown method: get_table_partitioning",
            "get_table_partition_status",
        ));
        assert!(!super::is_agent_partition_method_unsupported(
            "Agent RPC error (-1): Connection failed",
            "get_table_partitioning",
        ));
    }

    #[test]
    fn deduplicates_columns_and_preserves_later_comment() {
        let columns = deduplicate_column_infos(vec![
            test_column("ID", None, false),
            test_column("ID", Some("源主键"), true),
            test_column("TFBH", Some(""), false),
            test_column("TFBH", Some("台账编号"), false),
        ]);

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].name, "ID");
        assert_eq!(columns[0].comment.as_deref(), Some("源主键"));
        assert!(columns[0].is_primary_key);
        assert_eq!(columns[1].name, "TFBH");
        assert_eq!(columns[1].comment.as_deref(), Some("台账编号"));
    }

    #[test]
    fn table_comments_from_query_result_maps_non_blank_comments() {
        let result = db::QueryResult {
            columns: vec!["TABLE_NAME".to_string(), "COMMENTS".to_string()],
            column_types: Vec::new(),
            column_sortables: Vec::new(),
            spatial_columns: vec![],
            spatial_values: vec![],
            rows: vec![
                vec![serde_json::json!("ORDERS"), serde_json::json!("Orders table")],
                vec![serde_json::json!("PRODUCTS"), serde_json::json!(" ")],
            ],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let comments = table_comments_from_query_result(result);
        assert_eq!(comments.get("ORDERS").map(String::as_str), Some("Orders table"));
        assert!(!comments.contains_key("PRODUCTS"));
    }
}

pub async fn list_objects_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
) -> Result<Vec<db::ObjectInfo>, String> {
    let db_config = connection_config(state, connection_id).await;
    let filter_locally_after_oracle_comments = false;
    let force_local_table_name_filter = table_name_filter.is_some_and(|filter| !filter.is_empty());
    let use_oracle_agent_paging = false;
    let metadata_session = EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "objects").await;
    let result = retry_metadata_connection_for_session(
        state,
        connection_id,
        Some(database),
        metadata_session.client_session_id(),
        || async {
            let objects = list_objects_once(
                state,
                connection_id,
                database,
                schema,
                filter,
                limit,
                offset,
                object_types,
                table_name_filter,
                metadata_session.client_session_id(),
            )
            .await
            .map(|outcome| {
                let final_offset = if outcome.paging_applied || false { Some(0) } else { offset };
                filter_object_infos(outcome.objects, filter, limit, final_offset, object_types, table_name_filter)
            })?;
            Ok(objects)
        },
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

pub async fn list_object_statistics_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::ObjectStatistics>, String> {
    let metadata_session =
        EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "object-statistics").await;
    let result = retry_metadata_connection_for_session(
        state,
        connection_id,
        Some(database),
        metadata_session.client_session_id(),
        || list_object_statistics_once(state, connection_id, database, schema, metadata_session.client_session_id()),
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

pub async fn list_completion_objects_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::ObjectInfo>, String> {
    let metadata_session =
        EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "completion-objects").await;
    let result = retry_metadata_connection_for_session(
        state,
        connection_id,
        Some(database),
        metadata_session.client_session_id(),
        || list_completion_objects_once(state, connection_id, database, schema, metadata_session.client_session_id()),
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

fn ephemeral_agent_metadata_session_id(config: Option<&ConnectionConfig>, task_kind: &str) -> Option<String> {
    config.filter(|config| false).map(|_| task_client_session_id(task_kind, &uuid::Uuid::new_v4().to_string()))
}

async fn close_ephemeral_agent_metadata_session(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    client_session_id: Option<&str>,
) -> bool {
    let Some(client_session_id) = client_session_id else {
        return true;
    };
    match state.close_metadata_session_pool(connection_id, database, client_session_id).await {
        Ok(_) => true,
        Err(error) => {
            log::warn!(
                "Failed to close ephemeral Agent metadata session '{client_session_id}' for '{connection_id}': {error}"
            );
            false
        }
    }
}

pub async fn completion_assistant_search_core(
    state: &AppState,
    request: db::CompletionAssistantRequest,
) -> Result<db::CompletionAssistantResponse, String> {
    let started_at = Instant::now();
    let request_summary = format!(
        "connection_id={} database={} schema={:?} kinds={:?} mask={} limit={:?}",
        request.connection_id,
        request.database,
        request.schema,
        request.object_kinds,
        request.mask,
        request.max_results
    );
    retry_metadata_connection(state, &request.connection_id, Some(&request.database), || async {
        let pool_key = state
            .get_or_create_metadata_pool_for_session(&request.connection_id, Some(&request.database), None)
            .await?;
        log::debug!("[schema][completion_assistant:start] {request_summary}");
        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
        }

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {}
        }

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {}
        }

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            if let Some(pool) = pool_handle.as_ref().and_then(|pool| match pool {
                PoolKind::Mysql(pool, mode) if true => Some(pool.clone()),
                _ => None,
            }) {
                return db::mysql::completion_assistant_search(&pool, &request).await;
            }
        }

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {}
        }

        let response = completion_assistant_fallback_core(state, &request).await;
        if let Ok(response) = &response {
            log::debug!(
                "[schema][completion_assistant:done] {} elapsed_ms={} candidates={} fallback_used={}",
                request_summary,
                started_at.elapsed().as_millis(),
                response.candidates.len(),
                response.fallback_used
            );
        }
        response
    })
    .await
}

fn is_agent_completion_assistant_unsupported(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("unknown method: completion_assistant_search_v1")
        || error.contains("method not found: completion_assistant_search_v1")
        || error.contains("completion assistant search is not supported")
}

/// True when an agent built against an older protocol does not implement a
/// table-partition RPC. A mixed-version deployment (new core, old agent) must
/// hide the Partitions tab rather than fail every probe, so callers degrade to
/// the default value instead of surfacing the error.
fn is_agent_partition_method_unsupported(error: &str, method: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains(method) && (error.contains("unknown method") || error.contains("method not found"))
}

async fn completion_assistant_fallback_core(
    state: &AppState,
    request: &db::CompletionAssistantRequest,
) -> Result<db::CompletionAssistantResponse, String> {
    let limit = request.max_results.unwrap_or(100).clamp(1, 1000);
    let kinds = if request.object_kinds.is_empty() {
        vec![db::CompletionAssistantObjectKind::Table, db::CompletionAssistantObjectKind::View]
    } else {
        request.object_kinds.clone()
    };
    let mut candidates = Vec::new();
    let schema = request.parent_schema.as_deref().or(request.schema.as_deref()).unwrap_or("");
    let filter = request.mask.trim().trim_matches('%');

    if kinds.iter().any(|kind| matches!(kind, db::CompletionAssistantObjectKind::Schema)) {
        let schemas = list_schemas_core(state, &request.connection_id, &request.database).await?;
        for schema_name in schemas {
            if completion_name_matches(&schema_name, filter, request.match_mode.as_ref()) {
                candidates.push(db::CompletionAssistantCandidate {
                    name: schema_name.clone(),
                    kind: db::CompletionAssistantCandidateKind::Schema,
                    database: Some(request.database.clone()),
                    schema: Some(schema_name),
                    parent_schema: None,
                    parent_name: None,
                    comment: None,
                    data_type: None,
                    signature: None,
                });
            }
            if candidates.len() >= limit {
                return Ok(db::CompletionAssistantResponse { candidates, incomplete: true, fallback_used: true });
            }
        }
    }

    if kinds.iter().any(db::CompletionAssistantObjectKind::is_table_like) {
        let object_types = completion_table_object_types(&kinds);
        let tables = list_tables_core(
            state,
            &request.connection_id,
            &request.database,
            schema,
            if filter.is_empty() { None } else { Some(filter) },
            Some(limit),
            None,
            object_types.as_deref(),
            None,
        )
        .await?;
        for table in tables {
            let kind = if table.table_type.to_uppercase().contains("VIEW") {
                db::CompletionAssistantCandidateKind::View
            } else {
                db::CompletionAssistantCandidateKind::Table
            };
            candidates.push(db::CompletionAssistantCandidate {
                name: table.name,
                kind,
                database: Some(request.database.clone()),
                schema: if schema.is_empty() { None } else { Some(schema.to_string()) },
                parent_schema: table.parent_schema,
                parent_name: table.parent_name,
                comment: table.comment,
                data_type: None,
                signature: None,
            });
            if candidates.len() >= limit {
                return Ok(db::CompletionAssistantResponse { candidates, incomplete: true, fallback_used: true });
            }
        }

        let completion_config = connection_config(state, &request.connection_id).await;
        {}
    }

    if kinds.iter().any(|kind| matches!(kind, db::CompletionAssistantObjectKind::Column)) {
        if let Some(table) = request.parent_name.as_deref().filter(|table| !table.trim().is_empty()) {
            let columns = get_columns_core(state, &request.connection_id, &request.database, schema, table).await?;
            for column in columns {
                if completion_name_matches(&column.name, filter, request.match_mode.as_ref()) {
                    candidates.push(db::CompletionAssistantCandidate {
                        name: column.name,
                        kind: db::CompletionAssistantCandidateKind::Column,
                        database: Some(request.database.clone()),
                        schema: if schema.is_empty() { None } else { Some(schema.to_string()) },
                        parent_schema: if schema.is_empty() { None } else { Some(schema.to_string()) },
                        parent_name: Some(table.to_string()),
                        comment: column.comment,
                        data_type: Some(column.data_type),
                        signature: None,
                    });
                }
                if candidates.len() >= limit {
                    return Ok(db::CompletionAssistantResponse { candidates, incomplete: true, fallback_used: true });
                }
            }
        }
    }

    Ok(db::CompletionAssistantResponse { candidates, incomplete: false, fallback_used: true })
}

fn completion_table_object_types(kinds: &[db::CompletionAssistantObjectKind]) -> Option<Vec<String>> {
    let mut object_types = Vec::new();
    if kinds.iter().any(|kind| matches!(kind, db::CompletionAssistantObjectKind::Table)) {
        object_types.push("table".to_string());
    }
    if kinds.iter().any(|kind| matches!(kind, db::CompletionAssistantObjectKind::View)) {
        object_types.push("view".to_string());
    }
    if object_types.is_empty() {
        None
    } else {
        Some(object_types)
    }
}

fn completion_name_matches(name: &str, filter: &str, mode: Option<&db::CompletionAssistantMatchMode>) -> bool {
    if filter.is_empty() {
        return true;
    }
    let name = name.to_lowercase();
    let filter = filter.to_lowercase();
    match mode.unwrap_or(&db::CompletionAssistantMatchMode::Prefix) {
        db::CompletionAssistantMatchMode::Prefix => name.starts_with(&filter),
        db::CompletionAssistantMatchMode::Contains => name.contains(&filter),
    }
}

async fn list_object_statistics_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    client_session_id: Option<&str>,
) -> Result<Vec<db::ObjectStatistics>, String> {
    let pool_key =
        state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
    let db_config = connection_config(state, connection_id).await;
    let pool_handle = state.pool_handle(&pool_key).await;
    {};
    {}
    {}
    {}
    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
    match &pool {
        PoolKind::Mysql(p, mode) => {
            let include_mysql_details = db_config.as_ref().is_some_and(|config| true);
            db::mysql::list_object_statistics(p, database, include_mysql_details).await
        }

        _ => Ok(vec![]),
    }
}

struct ObjectListOutcome {
    objects: Vec<db::ObjectInfo>,
    paging_applied: bool,
}

fn unpaged_object_list(objects: Vec<db::ObjectInfo>) -> ObjectListOutcome {
    ObjectListOutcome { objects, paging_applied: false }
}

async fn list_objects_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    filter: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<&[String]>,
    table_name_filter: Option<&TableNameFilter>,
    client_session_id: Option<&str>,
) -> Result<ObjectListOutcome, String> {
    let pool_key =
        state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
    let db_config = connection_config(state, connection_id).await;
    let force_local_table_name_filter = table_name_filter.is_some_and(|filter| !filter.is_empty());
    let (mysql_limit, mysql_offset) = if filter.is_none_or(|value| value.trim().is_empty())
        && table_name_filter.is_none_or(|filter| filter.is_empty())
    {
        (limit, offset)
    } else {
        (None, None)
    };

    {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        {}
        {}
    }

    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

    match &pool {
        PoolKind::Mysql(p, mode) => {
            // Note: mysql and ob_oracle take different second args (database vs schema)
            if mysql_table_list_source_for_config(db_config.as_ref()) == MysqlTableListSource::ShowFullTables {
                db::mysql::list_objects_with_logical_tables(p, database, object_types, mysql_limit, mysql_offset)
                    .await
                    .map(|result| ObjectListOutcome { objects: result.objects, paging_applied: result.paging_applied })
            } else {
                db::mysql::list_objects(p, database, object_types, mysql_limit, mysql_offset)
                    .await
                    .map(|result| ObjectListOutcome { objects: result.objects, paging_applied: result.paging_applied })
            }
        }

        _ => Ok(unpaged_object_list(
            list_tables_core(state, connection_id, database, schema, None, None, None, None, None)
                .await?
                .into_iter()
                .map(|table| db::ObjectInfo {
                    name: table.name,
                    object_type: table.table_type,
                    schema: if schema.is_empty() { None } else { Some(schema.to_string()) },
                    valid: None,
                    signature: None,
                    custom_type_kind: None,
                    has_members: None,
                    comment: table.comment,
                    created_at: None,
                    updated_at: None,
                    parent_schema: table.parent_schema,
                    parent_name: table.parent_name,
                    trigger: None,
                    xugu_type_members_expandable: None,
                })
                .collect(),
        )),
    }
}

async fn list_completion_objects_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    client_session_id: Option<&str>,
) -> Result<Vec<db::ObjectInfo>, String> {
    let pool_key =
        state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
    let db_config = connection_config(state, connection_id).await;

    let pool_handle = state.pool_handle(&pool_key).await;
    {}
    {}

    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
    match &pool {
        PoolKind::Mysql(p, mode) if true => db::mysql::list_completion_objects(p, database).await,

        _ => Ok(Vec::new()),
    }
}

fn filter_completion_objects(objects: Vec<db::ObjectInfo>) -> Vec<db::ObjectInfo> {
    objects
        .into_iter()
        .filter(|object| {
            let object_type = object.object_type.to_ascii_uppercase();
            object_type.contains("PROCEDURE") || object_type.contains("FUNCTION") || object_type.contains("TRIGGER")
        })
        .collect()
}

async fn retry_metadata_connection<T, F, Fut>(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    retry_metadata_connection_for_session(state, connection_id, database, None, operation).await
}

async fn retry_metadata_connection_for_session<T, F, Fut>(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    client_session_id: Option<&str>,
    operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    Box::pin(run_metadata_connection_for_session(state, connection_id, database, client_session_id, true, operation))
        .await
}

async fn run_metadata_connection_for_session<T, F, Fut>(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    client_session_id: Option<&str>,
    allow_recovery: bool,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let db_type = {
        let configs = state.configs.read().await;
        configs.get(connection_id).map(|config| config.db_type)
    };
    let _metadata_permit = match db_type.filter(|db_type| uses_metadata_gate(*db_type)) {
        Some(db_type) => {
            Some(state.acquire_metadata_permit(connection_id, database, db_type, client_session_id).await?)
        }
        None => None,
    };
    if !allow_recovery {
        return operation().await;
    }
    let mut retried = false;
    let mut missing_pool_retry = false;
    loop {
        let result = operation().await;
        if result.as_ref().err().is_some_and(|error| error == "Pool not found") {
            if !missing_pool_retry {
                missing_pool_retry = true;
                log::debug!(
                    "[metadata:pool:missing-retry] connection_id={} database={}",
                    connection_id,
                    database.unwrap_or_default()
                );
                continue;
            }
            log::warn!(
                "[metadata:pool:missing] connection_id={} database={}",
                connection_id,
                database.unwrap_or_default()
            );
            return Err(crate::query::METADATA_POOL_BUSY_ERROR.to_string());
        }
        if result.as_ref().err().is_some_and(|error| crate::query::is_pool_saturation_error(error)) {
            log::warn!(
                "[metadata:pool:saturation] connection_id={} database={} error={}",
                connection_id,
                database.unwrap_or_default(),
                result.as_ref().err().map(String::as_str).unwrap_or_default()
            );
            return Err(crate::query::METADATA_POOL_BUSY_ERROR.to_string());
        }
        let recovery =
            result.as_ref().err().map(|error| metadata_recovery(db_type, error, retried)).unwrap_or_default();
        match recovery.action {
            MetadataErrorAction::ReplaceRuntime => {
                state
                    .detach_metadata_pool_after_recovery(
                        connection_id,
                        database,
                        client_session_id,
                        recovery.agent_session_id.as_deref(),
                        true,
                    )
                    .await;
                return result;
            }
            MetadataErrorAction::Discard => {
                state
                    .detach_metadata_pool_after_recovery(
                        connection_id,
                        database,
                        client_session_id,
                        recovery.agent_session_id.as_deref(),
                        false,
                    )
                    .await;
                return result;
            }
            MetadataErrorAction::Retry => {
                retried = true;
                if let Err(error) =
                    state.reconnect_metadata_pool_for_session(connection_id, database, client_session_id).await
                {
                    let reconnect_recovery = metadata_recovery(db_type, &error, true);
                    match reconnect_recovery.action {
                        MetadataErrorAction::ReplaceRuntime => {
                            state
                                .detach_metadata_pool_after_recovery(
                                    connection_id,
                                    database,
                                    client_session_id,
                                    reconnect_recovery.agent_session_id.as_deref(),
                                    true,
                                )
                                .await;
                        }
                        MetadataErrorAction::Retry | MetadataErrorAction::Discard => {
                            state
                                .detach_metadata_pool_after_recovery(
                                    connection_id,
                                    database,
                                    client_session_id,
                                    reconnect_recovery.agent_session_id.as_deref(),
                                    false,
                                )
                                .await;
                        }
                        MetadataErrorAction::Return => {}
                    }
                    return Err(error);
                }
            }
            MetadataErrorAction::Return => return result,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MetadataErrorAction {
    Retry,
    Discard,
    ReplaceRuntime,
    #[default]
    Return,
}

#[derive(Debug, Default)]
struct MetadataRecovery {
    action: MetadataErrorAction,
    agent_session_id: Option<String>,
}

#[cfg(test)]
fn metadata_error_action(db_type: Option<DatabaseType>, error: &str, retried: bool) -> MetadataErrorAction {
    metadata_recovery(db_type, error, retried).action
}

fn metadata_recovery(db_type: Option<DatabaseType>, error: &str, retried: bool) -> MetadataRecovery {
    {}

    let action = if !retried && is_retryable_metadata_error(error) {
        MetadataErrorAction::Retry
    } else if should_discard_pool_after_error(db_type, error) {
        MetadataErrorAction::Discard
    } else {
        MetadataErrorAction::Return
    };
    MetadataRecovery { action, agent_session_id: None }
}

#[cfg(test)]
async fn replace_metadata_runtime(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    client_session_id: Option<&str>,
) {
    state.replace_runtime_for_metadata_pool(connection_id, database, client_session_id).await;
}

fn is_retryable_metadata_error(error: &str) -> bool {
    crate::query::is_connection_error(error)
}

pub async fn get_columns_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::ColumnInfo>, String> {
    get_columns_core_for_session(state, connection_id, database, schema, table, None).await
}

pub async fn get_columns_core_for_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
) -> Result<Vec<db::ColumnInfo>, String> {
    {}
    if client_session_id.is_none() {
        let metadata_session =
            EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "columns").await;
        if metadata_session.client_session_id().is_some() {
            let result = get_columns_core_for_session_inner(
                state,
                connection_id,
                database,
                schema,
                table,
                metadata_session.client_session_id(),
                false,
            )
            .await;
            metadata_session.finish(state, connection_id, Some(database)).await;
            return result;
        }
    }
    get_columns_core_for_session_inner(state, connection_id, database, schema, table, client_session_id, true).await
}

async fn get_columns_core_for_session_inner(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
    use_client_session_context: bool,
) -> Result<Vec<db::ColumnInfo>, String> {
    get_columns_core_for_session_inner_with_pool(
        state,
        connection_id,
        database,
        schema,
        table,
        client_session_id,
        use_client_session_context,
        None,
        true,
    )
    .await
}

async fn get_columns_core_for_existing_pool(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    pool_key: &str,
) -> Result<Vec<db::ColumnInfo>, String> {
    get_columns_core_for_session_inner_with_pool(
        state,
        connection_id,
        database,
        schema,
        table,
        None,
        true,
        Some(pool_key),
        false,
    )
    .await
}

async fn get_columns_core_for_session_inner_with_pool(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
    use_client_session_context: bool,
    existing_pool_key: Option<&str>,
    allow_recovery: bool,
) -> Result<Vec<db::ColumnInfo>, String> {
    let context_session_id = if use_client_session_context { client_session_id } else { None };
    let existing_pool_key = existing_pool_key.map(str::to_owned);
    let operation = || async {
        let pool_key = if let Some(pool_key) = existing_pool_key.as_deref() {
            pool_key.to_string()
        } else {
            state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?
        };
        let db_config = connection_config(state, connection_id).await;

        {}

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {}

            {}
            {}
            {}
            {}
            {}
            {};
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            PoolKind::Mysql(p, mode) => {
                let effective_db = mysql_table_metadata_catalog(database, schema);
                db::mysql::get_columns(p, effective_db, table).await.map(deduplicate_column_infos)
            }

            _ => Ok(vec![]),
        }
    };
    Box::pin(run_metadata_connection_for_session(
        state,
        connection_id,
        Some(database),
        client_session_id,
        allow_recovery,
        operation,
    ))
    .await
}

fn deduplicate_column_infos(columns: Vec<db::ColumnInfo>) -> Vec<db::ColumnInfo> {
    let mut result: Vec<db::ColumnInfo> = Vec::with_capacity(columns.len());
    for column in columns {
        if let Some(existing) = result.iter_mut().find(|existing| existing.name == column.name) {
            existing.is_primary_key |= column.is_primary_key;
            existing.is_unique |= column.is_unique;
            existing.is_nullable &= column.is_nullable;
            merge_optional_string(&mut existing.column_default, column.column_default);
            merge_optional_string(&mut existing.extra, column.extra);
            merge_optional_string(&mut existing.comment, column.comment);
            if existing.numeric_precision.is_none() {
                existing.numeric_precision = column.numeric_precision;
            }
            if existing.numeric_scale.is_none() {
                existing.numeric_scale = column.numeric_scale;
            }
            if existing.character_maximum_length.is_none() {
                existing.character_maximum_length = column.character_maximum_length;
            }
            if existing.data_type.trim().is_empty() && !column.data_type.trim().is_empty() {
                existing.data_type = column.data_type;
            }
            if existing.metadata_capabilities != column.metadata_capabilities {
                existing.metadata_capabilities = None;
            }
        } else {
            result.push(column);
        }
    }
    result
}

pub async fn get_all_columns_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::TableColumnsResult>, String> {
    let tables = list_tables_core(state, connection_id, database, schema, None, None, None, None, None).await?;

    let mut result: Vec<db::TableColumnsResult> = Vec::with_capacity(tables.len());
    for table in tables {
        match get_columns_core(state, connection_id, database, schema, &table.name).await {
            Ok(columns) => {
                result.push(db::TableColumnsResult { table_name: table.name, columns, error: None });
            }
            Err(e) => {
                log::warn!(
                    "[schema][get_all_columns] connection_id={} database={} schema={} table={} error={}",
                    connection_id,
                    database,
                    schema,
                    table.name,
                    e
                );
                result.push(db::TableColumnsResult { table_name: table.name, columns: Vec::new(), error: Some(e) });
            }
        }
    }

    Ok(result)
}

fn merge_optional_string(target: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    if candidate.trim().is_empty() {
        if target.is_none() {
            *target = Some(candidate);
        }
        return;
    }
    if target.as_ref().is_none_or(|value| value.trim().is_empty()) {
        *target = Some(candidate);
    }
}

pub async fn list_indexes_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::IndexInfo>, String> {
    {}
    let metadata_session = EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "indexes").await;
    let result = list_indexes_core_for_session(
        state,
        connection_id,
        database,
        schema,
        table,
        metadata_session.client_session_id(),
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceKeyInfo {
    pub columns: Vec<String>,
}

pub fn reference_keys_from_indexes(indexes: &[db::IndexInfo]) -> Vec<ReferenceKeyInfo> {
    let mut keys = Vec::new();
    for index in indexes {
        if !index.is_unique
            || index.filter.as_deref().is_some_and(|filter| !filter.trim().is_empty())
            || index.columns.is_empty()
            || index.key_is_expression.iter().any(|is_expression| *is_expression)
        {
            continue;
        }
        let columns = index.columns.iter().map(|column| column.trim().to_string()).collect::<Vec<_>>();
        let mut seen = HashSet::new();
        if columns.iter().any(|column| column.is_empty() || !seen.insert(column.as_str())) {
            continue;
        }
        let key = ReferenceKeyInfo { columns };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

pub fn reference_key_columns_from_indexes(indexes: &[db::IndexInfo]) -> Vec<String> {
    reference_keys_from_indexes(indexes)
        .into_iter()
        .filter_map(|key| (key.columns.len() == 1).then(|| key.columns[0].clone()))
        .collect()
}

pub async fn list_reference_keys_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<ReferenceKeyInfo>, String> {
    let indexes = list_indexes_core(state, connection_id, database, schema, table).await?;
    Ok(reference_keys_from_indexes(&indexes))
}

pub async fn list_reference_key_columns_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, String> {
    let indexes = list_indexes_core(state, connection_id, database, schema, table).await?;
    Ok(reference_key_columns_from_indexes(&indexes))
}

async fn list_indexes_core_for_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
) -> Result<Vec<db::IndexInfo>, String> {
    retry_metadata_connection_for_session(state, connection_id, Some(database), client_session_id, || async {
        let pool_key =
            state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
        let db_config = connection_config(state, connection_id).await;

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
            {}
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            PoolKind::Mysql(p, mode) => {
                {}
                {
                    db::mysql::list_indexes(p, mysql_table_metadata_catalog(database, schema), table).await
                }
            }

            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_foreign_keys_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::ForeignKeyInfo>, String> {
    {}
    let metadata_session =
        EphemeralAgentMetadataSession::open(state, connection_id, Some(database), "foreign-keys").await;
    let result = list_foreign_keys_core_for_session(
        state,
        connection_id,
        database,
        schema,
        table,
        metadata_session.client_session_id(),
    )
    .await;
    metadata_session.finish(state, connection_id, Some(database)).await;
    result
}

pub async fn list_foreign_keys_for_database_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<HashMap<String, Vec<db::ForeignKeyInfo>>, String> {
    {}
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
        match &pool {
            PoolKind::Mysql(p, mode) if true => {
                db::mysql::list_foreign_keys_for_database(p, mysql_table_metadata_catalog(database, schema)).await
            }
            PoolKind::Mysql(_, _) => {
                Err("Database-wide foreign-key metadata is not supported for this MySQL variant".to_string())
            }
            _ => Err("Database-wide foreign-key metadata requires a native MySQL connection".to_string()),
        }
    })
    .await
}

async fn list_foreign_keys_core_for_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    client_session_id: Option<&str>,
) -> Result<Vec<db::ForeignKeyInfo>, String> {
    retry_metadata_connection_for_session(state, connection_id, Some(database), client_session_id, || async {
        let pool_key =
            state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
        let db_config = connection_config(state, connection_id).await;

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            PoolKind::Mysql(p, mode) => {
                db::mysql::list_foreign_keys(p, mysql_table_metadata_catalog(database, schema), table).await
            }

            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_triggers_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::TriggerInfo>, String> {
    {}
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            PoolKind::Mysql(p, mode) => {
                db::mysql::list_triggers(p, mysql_table_metadata_catalog(database, schema), table).await
            }

            _ => Ok(vec![]),
        }
    })
    .await
}

/// Lists structured constraints for a relation. Exposed generically so the
/// agent protocol and native drivers share one route; the built-in drivers
/// that implement it today are PostgreSQL, OpenGauss, SQL Server, and the
/// Xugu agent.
pub async fn list_constraints_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::ConstraintInfo>, String> {
    {}
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;

        {
            let pool_handle = state.pool_handle(&pool_key).await;
            {};
            {}
        }

        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_partitions_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::PartitionInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        Ok(vec![])
    })
    .await
}

/// PostgreSQL partition classification of a single table, used by the table
/// structure editor to decide whether `CREATE INDEX CONCURRENTLY` applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePartitionStatus {
    /// The table is a partitioned parent (`pg_class.relkind = 'p'`); PostgreSQL
    /// rejects `CREATE INDEX CONCURRENTLY` directly on it — the supported
    /// approach is building child indexes concurrently and attaching them.
    pub is_partitioned_parent: bool,
    /// The table is itself a partition of a parent (`pg_class.relispartition`).
    pub is_partition: bool,
}

pub async fn table_partition_status_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<TablePartitionStatus, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool_handle = state.pool_handle(&pool_key).await;
        match pool_handle.as_ref() {
            _ => Ok(TablePartitionStatus::default()),
        }
    })
    .await
}

/// Structured declarative partitioning view used by the table structure
/// editor. Native PostgreSQL pools and compatible agents (such as Kingbase)
/// provide the same response shape; unsupported pools return the default
/// all-false/empty value.
pub async fn get_table_partitioning_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<db::PgTablePartitioning, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool_handle = state.pool_handle(&pool_key).await;
        match pool_handle.as_ref() {
            _ => Ok(db::PgTablePartitioning::default()),
        }
    })
    .await
}

/// Same-table index names whose `pg_index.indisvalid` is `false` (left behind
/// by a cancelled `CREATE INDEX CONCURRENTLY`). Empty for non-PostgreSQL pools.
pub async fn list_invalid_indexes_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool_handle = state.pool_handle(&pool_key).await;
        match pool_handle.as_ref() {
            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_subpartitions_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<db::SubpartitionInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        Ok(vec![])
    })
    .await
}

pub async fn list_functions_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::FunctionInfo>, String> {
    let postgres_functions = retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(None),
        }
    })
    .await?;

    if let Some(functions) = postgres_functions {
        return Ok(functions);
    }

    // Non-Postgres: reuse sidebar list_objects + get_object_source paths.
    list_functions_via_objects(state, connection_id, database, schema).await
}

/// Build FunctionInfo for non-Postgres pools by reusing list_objects + get_object_source
/// (same paths the sidebar uses for PROCEDURE/FUNCTION).
async fn list_functions_via_objects(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::FunctionInfo>, String> {
    let object_types = ["PROCEDURE".to_string(), "FUNCTION".to_string()];
    let objects =
        list_objects_core(state, connection_id, database, schema, None, None, None, Some(&object_types), None).await?;

    // Bound concurrent get_object_source calls (N+1) without requiring AppState: Clone.
    const CONCURRENCY: usize = 8;
    let mut functions = Vec::with_capacity(objects.len());
    for chunk in objects.chunks(CONCURRENCY) {
        let chunk_results = futures::future::join_all(
            chunk
                .iter()
                .map(|object| load_function_info_via_object(state, connection_id, database, schema, object.clone())),
        )
        .await;
        functions.extend(chunk_results.into_iter().flatten());
    }

    Ok(functions)
}

fn schema_diff_routine_kind(object_type: &str) -> Option<(&'static str, db::ObjectSourceKind)> {
    let object_type_upper = object_type.to_ascii_uppercase();
    if object_type_upper.contains("PROC") {
        Some(("PROCEDURE", db::ObjectSourceKind::Procedure))
    } else if object_type_upper.contains("FUNC") {
        Some(("FUNCTION", db::ObjectSourceKind::Function))
    } else {
        None
    }
}

async fn load_function_info_via_object(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    object: db::ObjectInfo,
) -> Option<db::FunctionInfo> {
    let (function_type, source_kind) = schema_diff_routine_kind(&object.object_type)?;

    let definition = match get_object_source_core(
        state,
        connection_id,
        database,
        schema,
        &object.name,
        source_kind.clone(),
        object.signature.as_deref(),
        None,
    )
    .await
    {
        Ok(source) if !source.source.trim().is_empty() => source.source,
        Ok(_) | Err(_) => {
            // Retry the alternate routine kind when the primary getter is empty/fails.
            let alternate = match source_kind {
                db::ObjectSourceKind::Procedure => db::ObjectSourceKind::Function,
                db::ObjectSourceKind::Function => db::ObjectSourceKind::Procedure,
                other => other,
            };
            match get_object_source_core(
                state,
                connection_id,
                database,
                schema,
                &object.name,
                alternate,
                object.signature.as_deref(),
                None,
            )
            .await
            {
                Ok(source) if !source.source.trim().is_empty() => source.source,
                // Skip objects with no readable source so empty definitions are not treated as loaded.
                _ => return None,
            }
        }
    };

    Some(db::FunctionInfo {
        name: object.name,
        function_type: function_type.to_string(),
        data_type: String::new(),
        definition: strip_routine_definer_clause(&definition),
        arguments: object.signature.unwrap_or_default(),
    })
}

/// MySQL's SHOW CREATE PROCEDURE/FUNCTION prefixes `CREATE DEFINER=`user`@`host``.
/// The definer account typically differs across same-structure databases on
/// different servers while the routine body is identical, so drop the clause
/// before schema-diff comparison (same spirit as DBeaver's removeDefiner
/// option). Definitions without the clause pass through unchanged; the
/// `^CREATE DEFINER` anchor keeps definer mentions inside a routine body alone.
fn strip_routine_definer_clause(definition: &str) -> String {
    static DEFINER_PREFIX: OnceLock<Regex> = OnceLock::new();
    let definer_prefix = DEFINER_PREFIX.get_or_init(|| {
        Regex::new(r#"(?is)^\s*CREATE\s+DEFINER\s*=\s*(`(?:[^`]|``)*`|"(?:[^"]|"")*"|[A-Za-z0-9_$]+)@(`(?:[^`]|``)*`|"(?:[^"]|"")*"|[A-Za-z0-9_$.%*-]+)"#).unwrap()
    });
    match definer_prefix.find(definition) {
        Some(found) => format!("CREATE {}", definition[found.end()..].trim_start()),
        None => definition.to_string(),
    }
}

#[cfg(test)]
mod schema_diff_routine_kind_tests {
    use super::schema_diff_routine_kind;
    use crate::db::ObjectSourceKind;

    #[test]
    fn classifies_procedure_and_function_object_types() {
        assert_eq!(schema_diff_routine_kind("PROCEDURE"), Some(("PROCEDURE", ObjectSourceKind::Procedure)));
        assert_eq!(schema_diff_routine_kind("StoredProc"), Some(("PROCEDURE", ObjectSourceKind::Procedure)));
        assert_eq!(schema_diff_routine_kind("FUNCTION"), Some(("FUNCTION", ObjectSourceKind::Function)));
        assert_eq!(schema_diff_routine_kind("user_function"), Some(("FUNCTION", ObjectSourceKind::Function)));
        assert!(schema_diff_routine_kind("TABLE").is_none());
        assert!(schema_diff_routine_kind("VIEW").is_none());
    }
}

#[cfg(test)]
mod strip_routine_definer_clause_tests {
    use super::strip_routine_definer_clause;

    #[test]
    fn strips_backquoted_definer_prefix() {
        assert_eq!(
            strip_routine_definer_clause("CREATE DEFINER=`root`@`localhost` PROCEDURE `p`() BEGIN SELECT 1; END"),
            "CREATE PROCEDURE `p`() BEGIN SELECT 1; END"
        );
    }

    #[test]
    fn strips_bare_definer_prefix() {
        assert_eq!(
            strip_routine_definer_clause("CREATE DEFINER=app_user@10.0.0.% FUNCTION `f`() RETURNS int RETURN 1"),
            "CREATE FUNCTION `f`() RETURNS int RETURN 1"
        );
    }

    #[test]
    fn keeps_definitions_without_definer() {
        let def = "CREATE PROCEDURE `p`() BEGIN SELECT 1; END";
        assert_eq!(strip_routine_definer_clause(def), def);
    }

    #[test]
    fn keeps_definer_mentions_inside_the_body() {
        let def = "CREATE PROCEDURE `p`() BEGIN -- CREATE DEFINER=`x`@`y` stays\nSELECT 1; END";
        assert_eq!(strip_routine_definer_clause(def), def);
    }

    #[test]
    fn strips_definer_after_leading_whitespace() {
        assert_eq!(
            strip_routine_definer_clause("  CREATE DEFINER=`root`@`%` PROCEDURE `p`() BEGIN END"),
            "CREATE PROCEDURE `p`() BEGIN END"
        );
    }
}

pub async fn list_sequences_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    with_last_values: bool,
) -> Result<Vec<db::SequenceInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let db_config = connection_config(state, connection_id).await;
        if db_config.as_ref().is_some_and(is_agent_pg_sequence_config) {
            let pool_handle = state.pool_handle(&pool_key).await;
            {}
        }
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_rules_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::RuleInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn list_owners_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
) -> Result<Vec<db::OwnerInfo>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(vec![]),
        }
    })
    .await
}

pub async fn get_table_owner_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Option<String>, String> {
    retry_metadata_connection(state, connection_id, Some(database), || async {
        let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
        let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

        match &pool {
            _ => Ok(None),
        }
    })
    .await
}

/// Whether to widen or normalize a single-table DDL fetch for its caller.
///
/// Database export and table transfer render one relation at a time because
/// they already iterate every relation themselves. A selected table structure
/// export and interactive display both recurse through the PostgreSQL
/// partition tree, while only display includes access statements. Oracle
/// exports additionally request portable DDL normalization.
#[derive(Clone, Copy)]
struct TableDdlOptions {
    include_postgres_access: bool,
    include_partitions: bool,
    portable_oracle: bool,
    include_sqlserver_temporal: bool,
}

impl TableDdlOptions {
    const SINGLE_RELATION: Self = Self {
        include_postgres_access: false,
        include_partitions: false,
        portable_oracle: false,
        include_sqlserver_temporal: false,
    };
    const RELATION_EXPORT: Self = Self {
        include_postgres_access: false,
        include_partitions: false,
        portable_oracle: true,
        include_sqlserver_temporal: false,
    };
    const EXPORT: Self = Self {
        include_postgres_access: false,
        include_partitions: true,
        portable_oracle: true,
        include_sqlserver_temporal: false,
    };
    const DISPLAY: Self = Self {
        include_postgres_access: true,
        include_partitions: true,
        portable_oracle: false,
        include_sqlserver_temporal: true,
    };
}

pub async fn get_table_ddl_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    object_type: Option<db::ObjectSourceKind>,
) -> Result<String, String> {
    get_table_ddl_core_with_options(
        state,
        connection_id,
        database,
        schema,
        table,
        object_type,
        TableDdlOptions::SINGLE_RELATION,
        None,
    )
    .await
}

pub async fn get_table_export_ddl_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    object_type: Option<db::ObjectSourceKind>,
) -> Result<String, String> {
    get_table_ddl_core_with_options(
        state,
        connection_id,
        database,
        schema,
        table,
        object_type,
        TableDdlOptions::EXPORT,
        None,
    )
    .await
}

pub(crate) async fn get_table_relation_export_ddl_core_for_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    object_type: Option<db::ObjectSourceKind>,
    client_session_id: Option<&str>,
) -> Result<String, String> {
    get_table_ddl_core_with_options(
        state,
        connection_id,
        database,
        schema,
        table,
        object_type,
        TableDdlOptions::RELATION_EXPORT,
        client_session_id,
    )
    .await
}

pub async fn get_table_display_ddl_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    object_type: Option<db::ObjectSourceKind>,
) -> Result<String, String> {
    get_table_ddl_core_with_options(
        state,
        connection_id,
        database,
        schema,
        table,
        object_type,
        TableDdlOptions::DISPLAY,
        None,
    )
    .await
}

async fn get_table_ddl_core_with_options(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    object_type: Option<db::ObjectSourceKind>,
    options: TableDdlOptions,
    client_session_id: Option<&str>,
) -> Result<String, String> {
    {}
    if matches!(object_type, Some(db::ObjectSourceKind::View)) {
        let source = get_object_source_core(
            state,
            connection_id,
            database,
            schema,
            table,
            db::ObjectSourceKind::View,
            None,
            None,
        )
        .await?;
        let db_config = connection_config(state, connection_id).await;
        let append_oracle_comments = false;
        let schema = { schema };
        let database_type = db_config.as_ref().map(|config| config.db_type);
        // Kingbase MySQL compatibility mode reports a backtick identifier
        // quote; thread it through so the view DDL wraps hyphenated schema
        // names in backticks instead of double quotes the server rejects.
        let identifier_quote = state.connection_identifier_quote(connection_id, Some(database)).await.ok().flatten();
        let ddl = crate::object_source_sql::build_view_ddl_sql(crate::object_source_sql::BuildViewDdlInput {
            database_type,
            schema: if schema.trim().is_empty() { None } else { Some(schema.to_string()) },
            name: table.to_string(),
            source: source.source,
            identifier_quote,
        });
        // Oracle-family comments live in dictionary tables, not inside CREATE VIEW.
        // Append COMMENT ON so table-properties DDL / hover match the structure editor.
        {}
        return Ok(ddl);
    }
    if matches!(object_type, Some(db::ObjectSourceKind::MaterializedView)) {
        let source = get_object_source_core(
            state,
            connection_id,
            database,
            schema,
            table,
            db::ObjectSourceKind::MaterializedView,
            None,
            None,
        )
        .await?;
        return Ok(source.source);
    }
    if let Some(kind) = object_type.clone().filter(ddl_kind_uses_object_source) {
        // Routines, packages, triggers, types and the other schema objects have no
        // table DDL: `SHOW CREATE TABLE` or the columns/indexes renderer can only
        // fabricate `CREATE TABLE <name> ()` for them. Ask for the definition
        // instead, and keep the previous behaviour when the driver cannot produce
        // one (engine without a source query, empty definition, …).
        match get_object_source_core(state, connection_id, database, schema, table, kind, None, None).await {
            Ok(source) if !source.source.trim().is_empty() => return Ok(source.source),
            Ok(_) => {}
            Err(error) => {
                log::debug!(
                    "[schema][get_table_ddl:object-source-kind-fallback-failed] connection_id={connection_id} database={database} schema={schema} table={table} error={error}"
                );
            }
        }
    }

    retry_metadata_connection_for_session(state, connection_id, Some(database), client_session_id, || {
        get_table_ddl_once(state, connection_id, database, schema, table, options, client_session_id)
    })
    .await
}

/// Whether the DDL of this object kind is its own definition, rather than the
/// table DDL built from columns and indexes.
///
/// Views and materialized views are excluded because they have dedicated
/// branches in [`get_table_ddl_core_with_options`]: a view is re-wrapped into a
/// `CREATE ... VIEW` statement, and a materialized view returns the raw source.
fn ddl_kind_uses_object_source(kind: &db::ObjectSourceKind) -> bool {
    !matches!(kind, db::ObjectSourceKind::View | db::ObjectSourceKind::MaterializedView)
}

async fn get_table_ddl_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    options: TableDdlOptions,
    client_session_id: Option<&str>,
) -> Result<String, String> {
    let pool_key =
        state.get_or_create_metadata_pool_for_session(connection_id, Some(database), client_session_id).await?;
    let db_config = connection_config(state, connection_id).await;

    {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}

        {}
        {}
        {}
    }

    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;

    match &pool {
        PoolKind::Mysql(p, _) => mysql_ddl(p, mysql_table_metadata_catalog(database, schema), table).await,

        _ => Err("DDL not supported for this database type".to_string()),
    }
}

async fn connection_config(state: &AppState, connection_id: &str) -> Option<ConnectionConfig> {
    state.configs.read().await.get(connection_id).cloned()
}

/// KingbaseES and Vastbase run through Agent pools but expose PostgreSQL's
/// sequence catalogs, so their sequence metadata is queried over the agent
/// connection (t8y2/dbx#9016). HighGo/UXDB stay on their existing paths.
fn is_agent_pg_sequence_config(config: &ConnectionConfig) -> bool {
    false
}

fn with_agent_pg_sequence_objects(
    mut objects: Vec<db::ObjectInfo>,
    sequences: &[db::ObjectInfo],
) -> Vec<db::ObjectInfo> {
    for sequence in sequences {
        let already_listed = objects.iter().any(|object| {
            object.name == sequence.name
                && normalize_object_info_object_type(&object.object_type)
                    == normalize_object_info_object_type(&sequence.object_type)
        });
        if !already_listed {
            objects.push(sequence.clone());
        }
    }
    objects
}

fn is_cloudberry_config(config: &ConnectionConfig) -> bool {
    matches!(config.driver_profile.as_deref(), Some("cloudberry"))
}

/// Whether a native PostgreSQL connection should list user-defined types.
///
/// Only databases with a verified `pg_type` catalog contract are enabled.
/// Other PG-protocol connections (Redshift, QuestDB, Cloudberry, KWDB, ...)
/// keep the legacy object list even though they share `PoolKind::Postgres`.
fn supports_pg_custom_type_objects(config: &ConnectionConfig) -> bool {
    false
}

/// Whether a typed object-list request needs the pg_class relation branch.
///
/// `None` means the caller wants the full object list (object browser “all
/// objects” view), so every branch is selected. Group loads only request their
/// own kinds (e.g. `["TABLE"]`), which skips the other catalog scans entirely.
fn object_types_include_relations(object_types: Option<&[String]>) -> bool {
    object_types.is_none_or(|types| {
        types.iter().any(|t| {
            matches!(
                t.to_ascii_uppercase().as_str(),
                "TABLE" | "VIEW" | "MATERIALIZED_VIEW" | "SEQUENCE" | "FOREIGN_TABLE" | "PARTITIONED_TABLE"
            )
        })
    })
}

fn object_types_include_sequences(object_types: Option<&[String]>) -> bool {
    object_types.is_none_or(|types| types.iter().any(|t| t.eq_ignore_ascii_case("SEQUENCE")))
}

fn object_types_include_routines(object_types: Option<&[String]>) -> bool {
    object_types
        .is_none_or(|types| types.iter().any(|t| matches!(t.to_ascii_uppercase().as_str(), "PROCEDURE" | "FUNCTION")))
}

fn object_types_include_custom_types(object_types: Option<&[String]>) -> bool {
    object_types
        .is_none_or(|types| types.iter().any(|t| t.eq_ignore_ascii_case("TYPE") || t.eq_ignore_ascii_case("TYPE_BODY")))
}

/// Whether the object-type filter exclusively asks for user-defined types.
///
/// Used to keep agent errors visible: the native PostgreSQL fallback never
/// lists custom types, so running it for a dedicated type request would mask a
/// real catalog failure as an empty type group.
fn object_types_only_custom_types(object_types: Option<&[String]>) -> bool {
    object_types.is_some_and(|types| {
        !types.is_empty() && types.iter().all(|t| t.eq_ignore_ascii_case("TYPE") || t.eq_ignore_ascii_case("TYPE_BODY"))
    })
}

fn mysql_show_metadata_database_for_config<'a>(config: Option<&ConnectionConfig>, database: &'a str) -> &'a str {
    {
        database
    }
}

fn filter_mysql_system_databases_for_config(
    databases: Vec<db::DatabaseInfo>,
    config: Option<&ConnectionConfig>,
) -> Vec<db::DatabaseInfo> {
    {
        return databases;
    }
}

fn is_mysql_system_database(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "information_schema" | "mysql" | "performance_schema" | "sys")
}

fn is_questdb_config(config: &ConnectionConfig) -> bool {
    false
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn mysql_ident(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn mysql_qualified_name(database: &str, name: &str) -> String {
    if database.trim().is_empty() {
        mysql_ident(name)
    } else {
        format!("{}.{}", mysql_ident(database), mysql_ident(name))
    }
}

pub fn mysql_object_source_sql(database: &str, name: &str, kind: &db::ObjectSourceKind) -> String {
    let qualified_name = mysql_qualified_name(database, name);
    match kind {
        db::ObjectSourceKind::View => format!("SHOW CREATE VIEW {qualified_name}"),
        db::ObjectSourceKind::Procedure => format!("SHOW CREATE PROCEDURE {qualified_name}"),
        db::ObjectSourceKind::Function => format!("SHOW CREATE FUNCTION {qualified_name}"),
        db::ObjectSourceKind::Trigger => format!("SHOW CREATE TRIGGER {qualified_name}"),
        db::ObjectSourceKind::Event => format!("SHOW CREATE EVENT {qualified_name}"),
        db::ObjectSourceKind::Sequence
        | db::ObjectSourceKind::Synonym
        | db::ObjectSourceKind::Job
        | db::ObjectSourceKind::Package
        | db::ObjectSourceKind::PackageBody
        | db::ObjectSourceKind::Type
        | db::ObjectSourceKind::TypeBody => String::new(),
        // Doris and StarRocks expose materialized views via `SHOW CREATE MATERIALIZED VIEW`.
        // MySQL itself never reaches this arm in normal use: the desktop capabilities map at
        // apps/desktop/src/lib/database/databaseObjectCapabilities.ts has no "mysql" entry,
        // so the UI never sends MaterializedView for a real MySQL connection. If something
        // else forces the kind through, MySQL 8.x will surface a syntax error instead of
        // silently returning empty, which is the desired fail-loud behaviour.
        db::ObjectSourceKind::MaterializedView => {
            format!("SHOW CREATE MATERIALIZED VIEW {qualified_name}")
        }
    }
}

/// Column index of the DDL text in the row returned by the statements generated
/// by [`mysql_object_source_sql`].
///
/// The shape of the result is dialect-dependent:
/// - `SHOW CREATE VIEW`, Doris/StarRocks `SHOW CREATE MATERIALIZED VIEW` →
///   `(Name, DDL)` → DDL at index `1`.
/// - `SHOW CREATE PROCEDURE`, `SHOW CREATE FUNCTION`, `SHOW CREATE TRIGGER` →
///   `(Name, sql_mode, DDL, …)` → DDL at index `2`.
/// - `SHOW CREATE EVENT` → `(Event, sql_mode, time_zone, Create Event, …)` →
///   DDL at index `3` (it has an extra `time_zone` column before the DDL).
///
/// Encoded as a function so the index can be unit-tested without a live DB.
pub(crate) fn mysql_object_source_ddl_column_index(kind: &db::ObjectSourceKind) -> usize {
    match kind {
        db::ObjectSourceKind::View | db::ObjectSourceKind::MaterializedView => 1,
        db::ObjectSourceKind::Procedure
        | db::ObjectSourceKind::Function
        | db::ObjectSourceKind::Trigger
        | db::ObjectSourceKind::Sequence
        | db::ObjectSourceKind::Synonym
        | db::ObjectSourceKind::Job
        | db::ObjectSourceKind::Package
        | db::ObjectSourceKind::PackageBody
        | db::ObjectSourceKind::Type
        | db::ObjectSourceKind::TypeBody => 2,
        db::ObjectSourceKind::Event => 3,
    }
}

fn normalize_routine_object_source(source: String) -> String {
    source
}

async fn mysql_object_source(
    pool: &db::mysql::MySqlPool,
    database: &str,
    name: &str,
    kind: &db::ObjectSourceKind,
) -> Result<String, String> {
    let primary_sql = mysql_object_source_sql(database, name, kind);
    let primary_column_index = mysql_object_source_ddl_column_index(kind);
    let mut conn = db::mysql::get_conn_with_timeout(pool, db::connection_timeout()).await?;

    match read_mysql_object_source_row(&mut conn, &primary_sql, primary_column_index).await {
        Ok(source) => Ok(source),

        Err(e) => Err(e),
    }
}

async fn read_mysql_object_source_row(
    conn: &mut mysql_async::Conn,
    sql: &str,
    ddl_column_index: usize,
) -> Result<String, String> {
    use mysql_async::prelude::*;
    let result = conn.query_iter(sql).await.map_err(|e| e.to_string())?;
    let rows: Vec<mysql_async::Row> = result.collect_and_drop().await.map_err(|e| e.to_string())?;
    let row = rows.first().ok_or("Object source not found")?;
    row.get_opt::<String, usize>(ddl_column_index)
        .and_then(|result| result.ok())
        .or_else(|| {
            row.get_opt::<Vec<u8>, usize>(ddl_column_index)
                .and_then(|result| result.ok())
                .map(|b| String::from_utf8_lossy(&b).to_string())
        })
        .ok_or_else(|| "Failed to read object source".to_string())
}

/// Whether a connection may serve custom type details (phase 2). Kept
/// separate from listing support so a future per-kind DDL capability can be
/// toggled independently.
fn supports_custom_type_details(config: &ConnectionConfig) -> bool {
    false
}

pub async fn get_custom_type_details_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    name: &str,
) -> Result<db::CustomTypeDetails, String> {
    retry_metadata_connection(state, connection_id, Some(database), || {
        get_custom_type_details_once(state, connection_id, database, schema, name)
    })
    .await
}

async fn get_custom_type_details_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    name: &str,
) -> Result<db::CustomTypeDetails, String> {
    let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
    let db_config = connection_config(state, connection_id).await;
    let Some(config) = db_config.as_ref() else {
        return Err("connection not found".to_string());
    };
    if !supports_custom_type_details(config) {
        return Err(format!("custom type details are not supported for {:?} connections", config.db_type));
    }
    {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
    }
    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
    match &pool {
        _ => Err("custom type details are not supported for this connection type".to_string()),
    }
}

pub async fn get_object_source_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    name: &str,
    object_type: db::ObjectSourceKind,
    signature: Option<&str>,
    relation_name: Option<&str>,
) -> Result<db::ObjectSource, String> {
    let source = retry_metadata_connection(state, connection_id, Some(database), || {
        get_object_source_once(
            state,
            connection_id,
            database,
            schema,
            name,
            object_type.clone(),
            signature,
            relation_name,
        )
    })
    .await?;
    Ok(finalize_object_source(source))
}

pub async fn get_event_info_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    _schema: &str,
    name: &str,
) -> Result<db::MysqlEventInfo, String> {
    let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
    let pool = clone_metadata_pool(state, &pool_key).await.ok_or("Pool not found")?;
    match pool {
        PoolKind::Mysql(pool, _) => db::mysql::get_event_info(&pool, database, name).await,

        _ => Err("MySQL event details are only supported for MySQL connections".into()),
    }
}

fn finalize_object_source(mut source: db::ObjectSource) -> db::ObjectSource {
    if matches!(source.object_type, db::ObjectSourceKind::Procedure | db::ObjectSourceKind::Function) {
        source.source = normalize_routine_object_source(source.source);
    }
    if matches!(source.object_type, db::ObjectSourceKind::Event) {
        source.editable = Some(false);
    }
    source
}

async fn get_object_source_once(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    name: &str,
    object_type: db::ObjectSourceKind,
    signature: Option<&str>,
    relation_name: Option<&str>,
) -> Result<db::ObjectSource, String> {
    let pool_key = state.get_or_create_metadata_pool_for_session(connection_id, Some(database), None).await?;
    let db_config = connection_config(state, connection_id).await;
    let source = {
        let pool_handle = state.pool_handle(&pool_key).await;
        {}
        {
            match pool_handle.as_ref().ok_or("Pool not found")? {
                PoolKind::Mysql(pool, _) => {
                    mysql_object_source(pool, mysql_table_metadata_catalog(database, schema), name, &object_type)
                        .await?
                }

                _ => return Err("Object source is not supported for this database type".to_string()),
            }
        }
    };

    let editable = { None };

    Ok(db::ObjectSource {
        name: name.to_string(),
        object_type,
        schema: if schema.is_empty() { None } else { Some(schema.to_string()) },
        source,
        editable,
        routine_parameters: None,
    })
}

/// Catalog features the routine/sequence object-source queries branch on.
/// Probing the catalog (rather than gating on error messages or versions)
/// keeps the object-source queries correct regardless of the locale the
/// server reports errors in (#11161 — the old error-message gate missed
/// localized servers).
#[derive(Clone, Copy)]
struct PostgresCatalogCaps {
    /// `pg_proc.prokind` exists from PostgreSQL 11 onwards; legacy servers
    /// filter routines with `proisagg`/`proiswindow` instead (#11161).
    has_proc_prokind: bool,
    /// The `pg_sequence` catalog view (and `pg_sequence_last_value`) are also
    /// PostgreSQL 10 additions; the pre-10 sequence DDL must not join them.
    has_pg_sequence: bool,
}

impl Default for PostgresCatalogCaps {
    fn default() -> Self {
        Self { has_proc_prokind: true, has_pg_sequence: true }
    }
}

#[cfg(test)]
mod object_source_tests {
    use super::*;
    use crate::types::ObjectSourceKind;
}

#[cfg(test)]
mod ddl_tests {
    use super::*;

    /// Ctrl/Cmd+click on a routine, trigger or package sends its object kind to
    /// the DDL endpoint. Those requests must be answered from the object source:
    /// the table renderer can only fabricate `CREATE TABLE <name> ()` for them.
    #[test]
    fn ddl_object_kinds_that_need_the_object_source() {
        for kind in [
            db::ObjectSourceKind::Procedure,
            db::ObjectSourceKind::Function,
            db::ObjectSourceKind::Trigger,
            db::ObjectSourceKind::Event,
            db::ObjectSourceKind::Sequence,
            db::ObjectSourceKind::Synonym,
            db::ObjectSourceKind::Job,
            db::ObjectSourceKind::Package,
            db::ObjectSourceKind::PackageBody,
            db::ObjectSourceKind::Type,
            db::ObjectSourceKind::TypeBody,
        ] {
            assert!(ddl_kind_uses_object_source(&kind), "{kind:?} must use the object source");
        }
        // Views and materialized views keep their dedicated branches.
        assert!(!ddl_kind_uses_object_source(&db::ObjectSourceKind::View));
        assert!(!ddl_kind_uses_object_source(&db::ObjectSourceKind::MaterializedView));
    }

    fn column(name: &str, data_type: &str) -> db::ColumnInfo {
        db::ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: true,
            column_default: None,
            is_primary_key: false,
            extra: None,
            comment: None,
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            enum_values: None,
            ..Default::default()
        }
    }

    fn assert_table_ddl_options(
        options: TableDdlOptions,
        include_partitions: bool,
        portable_oracle: bool,
        include_postgres_access: bool,
    ) {
        assert_eq!(options.include_partitions, include_partitions);
        assert_eq!(options.portable_oracle, portable_oracle);
        assert_eq!(options.include_postgres_access, include_postgres_access);
    }

    #[test]
    fn table_structure_export_includes_partition_tree() {
        assert_table_ddl_options(TableDdlOptions::EXPORT, true, true, false);
        assert_table_ddl_options(TableDdlOptions::RELATION_EXPORT, false, true, false);
        assert_table_ddl_options(TableDdlOptions::DISPLAY, true, false, true);
    }

    #[test]
    fn mysql_display_ddl_gets_statement_terminator() {
        let ddl = "CREATE TABLE `users` (\n  `id` int NOT NULL\n) ENGINE=InnoDB";

        assert_eq!(
            ensure_display_ddl_terminated(ddl.to_string()),
            "CREATE TABLE `users` (\n  `id` int NOT NULL\n) ENGINE=InnoDB;"
        );
    }

    #[test]
    fn mysql_display_ddl_does_not_duplicate_existing_terminator() {
        let ddl = "CREATE TABLE `users` (`id` int);\n";

        assert_eq!(ensure_display_ddl_terminated(ddl.to_string()), ddl);
    }

    #[test]
    fn mysql_display_ddl_repairs_double_encoded_comments() {
        let ddl = "CREATE TABLE `订单` (\n  `id` bigint COMMENT 'è®¢åID',\n  `reviewed_at` datetime COMMENT 'å®¡æ ¸æ¶é´'\n) COMMENT='订单表'";

        assert_eq!(
            normalize_mysql_display_ddl(ddl.to_string()),
            "CREATE TABLE `订单` (\n  `id` bigint COMMENT '订单ID',\n  `reviewed_at` datetime COMMENT '审核时间'\n) COMMENT='订单表';"
        );
    }

    #[test]
    fn mysql_display_ddl_preserves_valid_text() {
        let ddl = "CREATE TABLE `orders` (`id` bigint COMMENT '订单ID') ENGINE=InnoDB";

        assert_eq!(
            normalize_mysql_display_ddl(ddl.to_string()),
            "CREATE TABLE `orders` (`id` bigint COMMENT '订单ID') ENGINE=InnoDB;"
        );
    }

    #[test]
    fn mysql_display_ddl_only_repairs_comment_clauses() {
        let ddl = "CREATE TABLE `comment` (\n  `comment` varchar(64) DEFAULT 'è®¢åID',\n  `kind` enum('comment', 'å®¡æ ¸æ¶é´') COMMENT 'å®¡æ ¸æ¶é´'\n) /* COMMENT 'è®¢åID' */";

        assert_eq!(
            normalize_mysql_display_ddl(ddl.to_string()),
            "CREATE TABLE `comment` (\n  `comment` varchar(64) DEFAULT 'è®¢åID',\n  `kind` enum('comment', 'å®¡æ ¸æ¶é´') COMMENT '审核时间'\n) /* COMMENT 'è®¢åID' */;"
        );
    }

    #[test]
    fn mysql_display_ddl_preserves_unterminated_literals() {
        let ddl = "CREATE TABLE `orders` (`note` varchar(64) DEFAULT 'unfinished COMMENT 'è®¢åID'";

        assert_eq!(normalize_mysql_display_ddl(ddl.to_string()), format!("{ddl};"));
    }

    struct FakeMysqlDdlExecutor {
        outcomes: std::collections::VecDeque<Result<String, MysqlDdlQueryError>>,
        executed: Vec<String>,
    }

    impl MysqlDdlQueryExecutor for FakeMysqlDdlExecutor {
        async fn execute(&mut self, sql: &str) -> Result<String, MysqlDdlQueryError> {
            self.executed.push(sql.to_string());
            self.outcomes.pop_front().expect("test outcome for DDL query")
        }
    }

    #[tokio::test]
    async fn mysql_ddl_uses_one_qualified_query_on_success() {
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Ok("CREATE TABLE `users` (`id` int)".to_string())].into(),
            executed: Vec::new(),
        };

        let ddl = mysql_ddl_with_executor(&mut executor, "app", "users").await.unwrap();

        assert_eq!(ddl, "CREATE TABLE `users` (`id` int);");
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `app`.`users`"]);
    }

    #[tokio::test]
    async fn mysql_ddl_retries_unqualified_after_no_such_table() {
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [
                Err(mysql_server_error(1146, "Table 'retail`fas.account`details' doesn't exist")),
                Ok("CREATE TABLE `account``details` (`id` int)".to_string()),
            ]
            .into(),
            executed: Vec::new(),
        };

        let ddl = mysql_ddl_with_executor(&mut executor, "retail`fas", "account`details").await.unwrap();

        assert_eq!(ddl, "CREATE TABLE `account``details` (`id` int);");
        assert_eq!(
            executor.executed,
            ["SHOW CREATE TABLE `retail``fas`.`account``details`", "SHOW CREATE TABLE `account``details`",]
        );
    }

    /// Without a database the materialized view statement stays unqualified,
    /// matching the qualifier used for the table probe.
    #[tokio::test]
    async fn mysql_ddl_falls_back_to_materialized_view_without_a_database() {
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [
                Err(mysql_server_error(
                    1105,
                    "not support async materialized view, please use `show create materialized view`",
                )),
                Ok("CREATE MATERIALIZED VIEW `mv_daily` (id)".to_string()),
            ]
            .into(),
            executed: Vec::new(),
        };

        let ddl = mysql_ddl_with_executor(&mut executor, "", "mv_daily").await.unwrap();

        assert_eq!(ddl, "CREATE MATERIALIZED VIEW `mv_daily` (id);");
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `mv_daily`", "SHOW CREATE MATERIALIZED VIEW `mv_daily`"]);
    }

    /// Engines without `SHOW CREATE MATERIALIZED VIEW` answer the retry with a
    /// syntax error; the original table error is what the user should still see.
    #[tokio::test]
    async fn mysql_ddl_preserves_the_table_error_when_the_materialized_view_probe_fails() {
        let refusal = mysql_server_error(
            1105,
            "errCode = 2, detailMessage = not support async materialized view, please use `show create materialized view`",
        );
        let expected = refusal.to_string();
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Err(refusal), Err(mysql_server_error(1064, "You have an error in your SQL syntax"))].into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "app", "missing").await.unwrap_err();

        assert_eq!(error, expected);
        assert_eq!(
            executor.executed,
            ["SHOW CREATE TABLE `app`.`missing`", "SHOW CREATE MATERIALIZED VIEW `app`.`missing`"]
        );
    }

    /// Unrelated failures must not trigger an extra probe, even when the server
    /// reports them with the same generic error code Doris uses.
    #[tokio::test]
    async fn mysql_ddl_does_not_probe_materialized_views_for_unrelated_failures() {
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Err(mysql_server_error(1105, "errCode = 2, detailMessage = table is broken"))].into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "app", "missing").await.unwrap_err();

        assert_eq!(error, "Server error: `ERROR 1105 (HY000): errCode = 2, detailMessage = table is broken'");
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `app`.`missing`"]);
    }

    #[tokio::test]
    async fn mysql_ddl_preserves_qualified_error_when_fallback_fails() {
        let first_error = mysql_server_error(1146, "qualified table doesn't exist");
        let expected = first_error.to_string();
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Err(first_error), Err(mysql_server_error(1146, "unqualified table doesn't exist"))].into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "app", "missing").await.unwrap_err();

        assert_eq!(error, expected);
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `app`.`missing`", "SHOW CREATE TABLE `missing`"]);
    }

    #[tokio::test]
    async fn mysql_ddl_does_not_retry_other_server_errors() {
        let first_error = mysql_server_error(1044, "access denied");
        let expected = first_error.to_string();
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Err(first_error), Ok("unexpected fallback".to_string())].into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "app", "users").await.unwrap_err();

        assert_eq!(error, expected);
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `app`.`users`"]);
    }

    #[tokio::test]
    async fn mysql_ddl_does_not_retry_without_a_database() {
        let first_error = mysql_server_error(1146, "table doesn't exist");
        let expected = first_error.to_string();
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [Err(first_error), Ok("unexpected fallback".to_string())].into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "", "missing").await.unwrap_err();

        assert_eq!(error, expected);
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `missing`"]);
    }

    #[tokio::test]
    async fn mysql_ddl_does_not_retry_result_parsing_errors() {
        let mut executor = FakeMysqlDdlExecutor {
            outcomes: [
                Err(MysqlDdlQueryError::Result("DDL not found".to_string())),
                Ok("unexpected fallback".to_string()),
            ]
            .into(),
            executed: Vec::new(),
        };

        let error = mysql_ddl_with_executor(&mut executor, "app", "users").await.unwrap_err();

        assert_eq!(error, "DDL not found");
        assert_eq!(executor.executed, ["SHOW CREATE TABLE `app`.`users`"]);
    }
}

#[derive(Debug)]
enum MysqlDdlQueryError {
    Query(mysql_async::Error),
    Result(String),
}

impl MysqlDdlQueryError {
    fn is_no_such_table(&self) -> bool {
        matches!(self, Self::Query(mysql_async::Error::Server(error)) if error.code == 1146)
    }

    /// Doris exposes asynchronous materialized views as base tables and then
    /// refuses `SHOW CREATE TABLE` / `SHOW CREATE VIEW` on them with a pointer
    /// to another statement:
    /// `ERROR 1105 (HY000): errCode = 2, detailMessage = not support async
    /// materialized view, please use `show create materialized view``.
    /// Recognize that refusal so the DDL request can be retried with the
    /// statement the server asked for instead of failing outright.
    fn is_materialized_view_ddl_refusal(&self) -> bool {
        matches!(
            self,
            Self::Query(mysql_async::Error::Server(error))
                if error.message.to_ascii_lowercase().contains("materialized view")
        )
    }
}

impl std::fmt::Display for MysqlDdlQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query(error) => error.fmt(formatter),
            Self::Result(error) => error.fmt(formatter),
        }
    }
}

trait MysqlDdlQueryExecutor {
    async fn execute(&mut self, sql: &str) -> Result<String, MysqlDdlQueryError>;
}

struct MysqlDdlConnection<'a> {
    conn: &'a mut mysql_async::Conn,
}

impl MysqlDdlQueryExecutor for MysqlDdlConnection<'_> {
    async fn execute(&mut self, sql: &str) -> Result<String, MysqlDdlQueryError> {
        use mysql_async::prelude::*;

        let result = self.conn.query_iter(sql).await.map_err(MysqlDdlQueryError::Query)?;
        let rows: Vec<mysql_async::Row> = result.collect_and_drop().await.map_err(MysqlDdlQueryError::Query)?;
        let row = rows.first().ok_or_else(|| MysqlDdlQueryError::Result("DDL not found".to_string()))?;
        row.get_opt::<String, usize>(1)
            .and_then(|result| result.ok())
            .or_else(|| {
                row.get_opt::<Vec<u8>, usize>(1)
                    .and_then(|result| result.ok())
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            })
            .ok_or_else(|| MysqlDdlQueryError::Result("Failed to read DDL".to_string()))
    }
}

async fn mysql_ddl_with_executor(
    executor: &mut impl MysqlDdlQueryExecutor,
    database: &str,
    table: &str,
) -> Result<String, String> {
    let sql = format!("SHOW CREATE TABLE {}", mysql_qualified_name(database, table));
    let qualified_error = match executor.execute(&sql).await {
        Ok(ddl) => return Ok(normalize_mysql_display_ddl(ddl)),
        Err(error) => error,
    };
    if qualified_error.is_materialized_view_ddl_refusal() {
        let name = mysql_qualified_name(database, table);
        return mysql_materialized_view_ddl(executor, &name, qualified_error).await;
    }
    if database.trim().is_empty() || !qualified_error.is_no_such_table() {
        return Err(qualified_error.to_string());
    }

    // Mycat 1.x routes by the logical qualifier but forwards it unchanged to a
    // physical schema; the metadata pool has already selected the logical database.
    let fallback_sql = format!("SHOW CREATE TABLE {}", mysql_ident(table));
    match executor.execute(&fallback_sql).await {
        Ok(ddl) => Ok(normalize_mysql_display_ddl(ddl)),
        Err(_) => Err(qualified_error.to_string()),
    }
}

/// Reads the definition of a materialized view that the server refused to
/// describe as a table.
///
/// The fallback only replaces the original error when the statement actually
/// returns a definition: engines without `SHOW CREATE MATERIALIZED VIEW`
/// (plain MySQL, MariaDB) answer with a syntax error, so the caller still sees
/// the error it would have seen before this fallback existed.
async fn mysql_materialized_view_ddl(
    executor: &mut impl MysqlDdlQueryExecutor,
    qualified_name: &str,
    original_error: MysqlDdlQueryError,
) -> Result<String, String> {
    let sql = format!("SHOW CREATE MATERIALIZED VIEW {qualified_name}");
    match executor.execute(&sql).await {
        Ok(ddl) => Ok(normalize_mysql_display_ddl(ddl)),
        Err(_) => Err(original_error.to_string()),
    }
}

pub async fn mysql_ddl(pool: &db::mysql::MySqlPool, database: &str, table: &str) -> Result<String, String> {
    // Use the health-checked getter so a stale pooled connection (server closed
    // it after an idle timeout, NAT/firewall dropped the TCP state, etc.) is
    // detected and replaced before issuing the query. Without this, the first
    // DDL request after a period of inactivity could surface a low-level
    // connection error that a manual refresh would have masked.
    let mut conn = db::mysql::get_conn_with_health_check(pool).await?;
    mysql_ddl_with_executor(&mut MysqlDdlConnection { conn: &mut conn }, database, table).await
}

fn normalize_mysql_display_ddl(sql: String) -> String {
    ensure_display_ddl_terminated(repair_mysql_ddl_comments(&sql))
}

fn repair_mysql_ddl_comments(sql: &str) -> String {
    let mut repaired = String::with_capacity(sql.len());
    let mut cursor = 0;

    while let Some((comment_start, value_start, value_end)) = next_mysql_ddl_comment_literal(sql, cursor) {
        repaired.push_str(&sql[cursor..comment_start]);
        repaired.push_str(&sql[comment_start..value_start]);
        repaired.push_str(&db::mysql::fix_potential_double_encoding(&sql[value_start..value_end]));
        repaired.push('\'');
        cursor = value_end + 1;
    }

    repaired.push_str(&sql[cursor..]);
    repaired
}

fn next_mysql_ddl_comment_literal(sql: &str, from: usize) -> Option<(usize, usize, usize)> {
    let bytes = sql.as_bytes();
    let mut index = from;
    while index + 7 <= bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                index = mysql_quoted_value_end(bytes, index)?;
                continue;
            }
            b'#' => {
                index = mysql_line_comment_end(bytes, index + 1);
                continue;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index = mysql_line_comment_end(bytes, index + 2);
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = mysql_block_comment_end(bytes, index + 2)?;
                continue;
            }
            _ => {}
        }

        if bytes[index..index + 7].eq_ignore_ascii_case(b"COMMENT")
            && (index == 0 || !is_mysql_identifier_byte(bytes[index - 1]))
            && (index + 7 == bytes.len() || !is_mysql_identifier_byte(bytes[index + 7]))
        {
            let comment_start = index;
            let mut quote = index + 7;
            while quote < bytes.len() && (bytes[quote].is_ascii_whitespace() || bytes[quote] == b'=') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'\'') {
                let value_end = mysql_quoted_value_end(bytes, quote)?.saturating_sub(1);
                return Some((comment_start, quote + 1, value_end));
            }
            index += 7;
            continue;
        }
        index += 1;
    }
    None
}

fn is_mysql_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn mysql_quoted_value_end(bytes: &[u8], quote: usize) -> Option<usize> {
    let delimiter = *bytes.get(quote)?;
    let mut index = quote + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            value if value == delimiter && bytes.get(index + 1) == Some(&delimiter) => index += 2,
            value if value == delimiter => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

fn mysql_line_comment_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..].iter().position(|byte| *byte == b'\n').map_or(bytes.len(), |offset| from + offset + 1)
}

fn mysql_block_comment_end(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..].windows(2).position(|window| window == b"*/").map(|offset| from + offset + 2)
}

fn ensure_display_ddl_terminated(sql: String) -> String {
    let trimmed = sql.trim_end();
    // SHOW CREATE TABLE returns a table definition, not a runnable script; DBX
    // displays/copies it as SQL, so include the default statement terminator.
    if trimmed.ends_with(';') {
        sql
    } else {
        format!("{trimmed};")
    }
}

/// 在一个 PostgreSQL 池上批量取元数据：多连接池并发、单连接池顺序执行。
///
/// 两个分支返回同一组结果的元组，调用方无需关心池的形状。单连接池上顺序执行
/// 与并发执行的端到端耗时相同（一条连接本来也只能串行处理），但不会产生排队
/// 导致的 checkout 超时。
macro_rules! postgres_metadata_batch {
    ($pool:expr, $($call:expr),+ $(,)?) => {{
        if postgres_pool_serves_one_request_at_a_time($pool) {
            Ok(($($call.await?,)+))
        } else {
            tokio::try_join!($($call),+)
        }
    }};
}

#[cfg(test)]
fn mysql_server_error(code: u16, message: &str) -> MysqlDdlQueryError {
    MysqlDdlQueryError::Query(mysql_async::Error::Server(mysql_async::ServerError {
        code,
        message: message.to_string(),
        state: "HY000".to_string(),
    }))
}
