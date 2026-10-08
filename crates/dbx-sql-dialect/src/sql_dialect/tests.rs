use super::*;
use crate::models::connection::DatabaseType;

#[test]
fn builds_mysql_table_data_large_value_previews_without_truncating_keys() {
    let sql = build_table_data_select_sql(TableDataSelectSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        table_name: "large_rows".to_string(),
        primary_keys: vec!["id".to_string()],
        columns: vec!["id".to_string(), "payload".to_string(), "raw_value".to_string(), "metadata".to_string()],
        column_types: vec!["bigint".to_string(), "longtext".to_string(), "longblob".to_string(), "json".to_string()],
        large_value_preview_size: Some(4096),
        limit: Some(100),
        ..Default::default()
    });

    assert!(sql.starts_with("SELECT `id`, LEFT(`payload`, 4097) AS `payload`"));
    assert!(sql.contains("CONCAT('T:4096:', LENGTH(`payload`)) AS `__DBX_LARGE_VALUE_BYTES_T_1`"));
    assert!(sql.contains("LEFT(`raw_value`, 4097) AS `raw_value`"));
    assert!(sql.contains("CONCAT('B:4096:', LENGTH(`raw_value`)) AS `__DBX_LARGE_VALUE_BYTES_B_2`"));
    assert!(sql.contains("LEFT(`metadata`, 4097) AS `metadata`"));
    assert!(sql.contains("CONCAT('T:4096:', LENGTH(`metadata`)) AS `__DBX_LARGE_VALUE_BYTES_J_3`"));
    assert!(!sql.contains("OCTET_LENGTH"));
    assert!(!sql.contains("__DBX_LARGE_VALUE_BYTES_0"));
}

#[test]
fn previews_mysql_bounded_string_columns_only_above_the_active_budget() {
    let sql = build_table_data_select_sql(TableDataSelectSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        table_name: "t_0001".to_string(),
        primary_keys: vec!["id".to_string()],
        columns: vec![
            "id".to_string(),
            "image_mime".to_string(),
            "image_data".to_string(),
            "image_url".to_string(),
            "large_note".to_string(),
            "large_binary".to_string(),
        ],
        column_types: vec![
            "int".to_string(),
            "varchar(64)".to_string(),
            "longblob".to_string(),
            "varchar(512)".to_string(),
            "varchar(10000)".to_string(),
            "varbinary(10000)".to_string(),
        ],
        large_value_preview_size: Some(419),
        limit: Some(10_000),
        ..Default::default()
    });

    assert!(sql.starts_with("SELECT `id`, `image_mime`, LEFT(`image_data`, 420) AS `image_data`"));
    assert!(sql.contains("CONCAT('B:419:', LENGTH(`image_data`)) AS `__DBX_LARGE_VALUE_BYTES_B_2`, LEFT(`image_url`, 420) AS `image_url`"));
    assert!(sql.contains("CONCAT('T:419:', LENGTH(`image_url`)) AS `__DBX_LARGE_VALUE_BYTES_T_3`"));
    assert!(sql.contains("LEFT(`large_note`, 420) AS `large_note`"));
    assert!(sql.contains("CONCAT('T:419:', LENGTH(`large_note`)) AS `__DBX_LARGE_VALUE_BYTES_T_4`"));
    assert!(sql.contains("LEFT(`large_binary`, 420) AS `large_binary`"));
    assert!(!sql.contains("LEFT(`image_mime`"));
}

#[test]
fn normalizes_where_input_with_multibyte_identifier_prefix() {
    assert_eq!(normalize_where_input(Some("`客户名称` = '示例客户'")), "`客户名称` = '示例客户'");
    assert_eq!(normalize_where_input(Some("WHERE `客户名称` = '示例客户';")), "`客户名称` = '示例客户'");
}
