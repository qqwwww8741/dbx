use serde::{Deserialize, Serialize};
use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write};

use crate::connection::AppState;
use crate::models::connection::DatabaseType;
use crate::query::{execute_sql_statement_with_options, QueryExecutionOptions};
use crate::sql_dialect::{build_table_data_select_sql, TableDataSelectSqlOptions};
use crate::types::QueryResult;

pub use dbx_formats::csv_export::*;

const TABLE_DATA_EXPORT_PAGE_SIZE: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCsvExportOptions {
    pub file_path: String,
    pub connection_id: String,
    pub database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub csv_quote_mode: CsvQuoteMode,
    /// CSV 里 NULL 写成什么。默认 `\N`；空字符串表示关闭该字面量，
    /// 退回「NULL 写成空字段」的旧行为（此时 NULL 与空字符串无法区分）。
    #[serde(default = "default_csv_null_literal")]
    pub null_literal: String,
}

/// Export pages go through the same dialect code as the grid, so the statement has to match the
/// connected server too: Neo4j below 5 only accepts `id()` where 5+ uses `elementId()`. That needs
/// the recorded version next to the connection type.
async fn connection_server_version_and_type(
    state: &AppState,
    connection_id: &str,
) -> Result<(Option<String>, DatabaseType), String> {
    state
        .configs
        .read()
        .await
        .get(connection_id)
        .map(|config| {
            let server_version = config.database_info.as_ref().and_then(|info| info.product_version.clone());
            (server_version, config.db_type)
        })
        .ok_or_else(|| format!("Connection config not found: {connection_id}"))
}

pub async fn export_table_data_csv_core(state: &AppState, options: TableCsvExportOptions) -> Result<u64, String> {
    let (server_version, database_type) = connection_server_version_and_type(state, &options.connection_id).await?;
    // This loop pages with `LIMIT <page_size> OFFSET <n>` and stops at the first
    // short page. Neither half holds for SOQL: a `/query` response carries at most
    // 2000 rows and hands the rest back as a QueryLocator (`has_more` +
    // `session_id`), and OFFSET is capped at 2000, so the first page always looks
    // short and the export would stop there — a CSV silently missing the rest of
    // the object. Refuse instead of truncating; a Salesforce export has to follow
    // the QueryLocator (`fetch_more`) the way the grid's "load more" does.
    {}
    let mut writer =
        BufWriter::new(File::create(&options.file_path).map_err(|err| format!("Failed to write CSV file: {err}"))?);
    writer.write_all("\u{FEFF}".as_bytes()).map_err(|err| err.to_string())?;
    let client_session_id = (false).then(|| format!("table-export:{}", uuid::Uuid::new_v4()));
    let export_options = &options;
    let outcome = write_table_csv_pages(
        &mut writer,
        database_type,
        server_version.as_deref(),
        &options,
        client_session_id.as_deref(),
        |sql, query_options| async move {
            execute_sql_statement_with_options(
                state,
                &export_options.connection_id,
                &export_options.database,
                &sql,
                export_options.schema.as_deref(),
                None,
                query_options,
            )
            .await
        },
    )
    .await;
    if let Some(client_session_id) = client_session_id {
        let _ =
            state.close_client_session_pool(&options.connection_id, Some(&options.database), &client_session_id).await;
    }
    outcome
}

async fn write_table_csv_pages<Execute, QueryFuture>(
    writer: &mut impl Write,
    database_type: DatabaseType,
    server_version: Option<&str>,
    options: &TableCsvExportOptions,
    client_session_id: Option<&str>,
    mut execute_page: Execute,
) -> Result<u64, String>
where
    Execute: FnMut(String, QueryExecutionOptions) -> QueryFuture,
    QueryFuture: Future<Output = Result<QueryResult, String>>,
{
    let use_cursor = false;
    let requested_page_size = options.page_size.unwrap_or(TABLE_DATA_EXPORT_PAGE_SIZE).max(1);
    let page_size = { requested_page_size };
    let mut session_id = None;

    let mut offset = 0usize;
    let mut rows_exported = 0u64;
    let mut wrote_header = false;

    loop {
        let sql = build_table_data_select_sql(TableDataSelectSqlOptions {
            database_type: Some(database_type),
            server_version: server_version.map(str::to_owned),
            schema: options.schema.clone(),
            table_name: options.table_name.clone(),
            table_type: None,
            primary_keys: Vec::new(),
            columns: options.columns.clone(),
            fallback_order_columns: Vec::new(),
            order_by: None,
            limit: Some(page_size),
            offset: Some(offset),
            where_input: None,
            include_row_id: false,
            ..Default::default()
        });
        let result = execute_page(
            sql,
            QueryExecutionOptions {
                max_rows: Some({ page_size }),
                fetch_size: use_cursor.then_some(page_size),
                page_size: use_cursor.then_some(page_size),
                result_session_id: session_id.take(),
                client_session_id: client_session_id.map(str::to_string),
                timeout_secs: options.timeout_secs,
                ..Default::default()
            },
        )
        .await?;
        let fetched = result.rows.len();
        let complete = { fetched < page_size };

        if !wrote_header {
            write_csv_text_row(writer, result.columns, options.csv_quote_mode)?;
            wrote_header = true;
        }

        for row in result.rows {
            writer.write_all(b"\n").map_err(|err| err.to_string())?;
            write_csv_value_row_with_options(
                writer,
                row,
                options.csv_quote_mode,
                csv_null_literal(&options.null_literal),
            )?;
        }

        rows_exported += fetched as u64;
        if complete {
            break;
        }
        offset += fetched;
    }

    if rows_exported == 0 {
        writer.write_all(b"\n").map_err(|err| err.to_string())?;
    }
    writer.flush().map_err(|err| err.to_string())?;
    Ok(rows_exported)
}

#[cfg(test)]
mod tests {
    use super::{export_table_data_csv_core, write_table_csv_pages, CsvQuoteMode, TableCsvExportOptions};
    use crate::connection::AppState;
    use crate::models::connection::{ConnectionConfig, DatabaseType};
    use crate::types::QueryResult;

    fn export_options(page_size: usize) -> TableCsvExportOptions {
        TableCsvExportOptions {
            file_path: String::new(),
            connection_id: "csv-test".to_string(),
            database: "app".to_string(),
            schema: None,
            table_name: "events".to_string(),
            columns: vec!["id".to_string()],
            page_size: Some(page_size),
            timeout_secs: Some(30),
            csv_quote_mode: CsvQuoteMode::default(),
            null_literal: String::new(),
        }
    }

    fn page(ids: &[usize], session_id: Option<&str>, has_more: bool) -> QueryResult {
        serde_json::from_value(serde_json::json!({
            "columns": ["id"],
            "rows": ids.iter().map(|id| vec![*id]).collect::<Vec<_>>(),
            "affected_rows": 0,
            "execution_time_ms": 1,
            "session_id": session_id,
            "has_more": has_more
        }))
        .unwrap()
    }
}
