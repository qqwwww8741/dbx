use dbx_core::models::connection::DatabaseType;
use dbx_core::schema_diff::{
    generate_schema_sync_sql, prepare_schema_diff, SchemaDiffPreparationOptions, TableSchemaDetail,
};
use dbx_core::sql_dialect::descriptor::DialectKind;
use dbx_core::types::{ColumnInfo, TableInfo};

fn table(name: &str) -> TableInfo {
    TableInfo {
        name: name.to_string(),
        table_type: "BASE TABLE".to_string(),
        valid: None,
        comment: None,
        parent_schema: None,
        parent_name: None,
    }
}

fn col(name: &str, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        data_type: data_type.to_string(),
        resolved_schema: None,
        is_nullable: false,
        column_default: None,
        is_primary_key: false,
        is_unique: false,
        extra: None,
        comment: None,
        numeric_precision: None,
        numeric_scale: None,
        character_maximum_length: None,
        metadata_capabilities: None,
        enum_values: None,
        character_set: None,
        collation: None,
    }
}

fn detail(name: &str, columns: Vec<ColumnInfo>) -> TableSchemaDetail {
    TableSchemaDetail {
        name: name.to_string(),
        columns,
        indexes: vec![],
        foreign_keys: vec![],
        triggers: vec![],
        ddl: None,
    }
}

// ============================================================================
// 12.2 — Same-dialect consistency: diff should be empty when schemas match
// ============================================================================

#[test]
fn identical_mysql_schemas_produce_no_diff() {
    let options = SchemaDiffPreparationOptions {
        source_tables: vec![table("users")],
        target_tables: vec![table("users")],
        source_details: vec![detail("users", vec![col("id", "int"), col("name", "varchar(64)")])],
        target_details: vec![detail("users", vec![col("id", "int"), col("name", "varchar(64)")])],
        database_type: DatabaseType::Mysql,
        ..Default::default()
    };

    let result = prepare_schema_diff(options);
    assert!(result.diffs.is_empty(), "Identical schemas should produce no diffs");
    assert!(result.sync_sql.is_empty(), "Identical schemas should produce no SQL");
}

// ============================================================================
// 12.2 — Cross-dialect generate_schema_sync_sql verification
// ============================================================================

#[test]
fn generate_schema_sync_sql_mysql_output_format() {
    let diffs = vec![dbx_core::schema_diff::TableDiff {
        diff_type: "added".to_string(),
        object_type: None,
        name: "users".to_string(),
        target_name: None,
        columns: Some(vec![]),
        indexes: None,
        foreign_keys: None,
        triggers: None,
        ddl: Some("CREATE TABLE `users` (`id` INT NOT NULL, `name` VARCHAR(64) NOT NULL)".to_string()),
        target_ddl: None,
        source_table_comment: None,
        target_table_comment: None,
        sync_sql: None,
    }];

    let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

    assert!(sql.contains("CREATE TABLE"), "MySQL SQL should contain CREATE TABLE");
    assert!(sql.contains('`'), "MySQL SQL should use backtick identifiers");
}
