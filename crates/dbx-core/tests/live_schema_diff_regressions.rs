use dbx_core::models::connection::DatabaseType;
use dbx_core::schema_diff::*;
use dbx_core::sql_dialect::descriptor::DialectKind;
use dbx_core::types::*;

fn table_info(name: &str, table_type: &str) -> TableInfo {
    TableInfo {
        name: name.to_string(),
        table_type: table_type.to_string(),
        valid: None,
        comment: None,
        parent_schema: None,
        parent_name: None,
    }
}
