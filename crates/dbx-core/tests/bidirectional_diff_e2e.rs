use dbx_core::models::connection::DatabaseType;
use dbx_core::schema_diff::{prepare_schema_diff, RollbackGraph, SchemaDiffPreparationOptions, TableSchemaDetail};
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

/// RollbackGraph validate_consistency directly
#[test]
fn rollback_graph_direct_consistency_check() {
    use dbx_core::schema_diff::DependencyGraph;

    let forward_diff = dbx_core::schema_diff::TableDiff {
        diff_type: "added".to_string(),
        object_type: None,
        name: "users".to_string(),
        target_name: None,
        columns: Some(vec![]),
        indexes: None,
        foreign_keys: None,
        triggers: None,
        ddl: Some("CREATE TABLE users (id INT)".to_string()),
        target_ddl: None,
        source_table_comment: None,
        target_table_comment: None,
        sync_sql: None,
    };

    let dep_graph =
        DependencyGraph { nodes: std::collections::HashMap::new(), topological_order: vec!["users".to_string()] };

    let mut graph = RollbackGraph::from_forward_diffs(&[forward_diff], &[], &dep_graph);
    graph.validate_consistency();
    assert!(graph.is_consistent, "Simple add should produce consistent rollback graph");
    assert!(graph.consistency_issues.is_empty());
}
