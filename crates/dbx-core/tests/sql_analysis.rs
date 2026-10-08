use dbx_core::sql_analysis::analyze_sql_references;

#[test]
fn extracts_tables_aliases_and_qualified_columns() {
    let analysis = analyze_sql_references("select u.missing from users u where u.id = 1", Some("postgres")).unwrap();

    assert_eq!(analysis.tables.len(), 1);
    assert_eq!(analysis.tables[0].name, "users");
    assert_eq!(analysis.tables[0].alias.as_deref(), Some("u"));

    let columns: Vec<_> =
        analysis.columns.iter().map(|column| (column.qualifier.as_deref(), column.name.as_str())).collect();
    assert_eq!(columns, vec![(Some("u"), "missing"), (Some("u"), "id")]);
}

#[test]
fn extracts_nested_query_scopes_for_correlated_subqueries() {
    let sql = "select aa.house_id from mds_base_house aa where exists (select 1 from mds_base_owner where HOUSE_ID = aa.HOUSE_ID)";
    let analysis = analyze_sql_references(sql, Some("mysql")).unwrap();

    let tables: Vec<_> =
        analysis.tables.iter().map(|table| (table.name.as_str(), table.alias.as_deref(), table.scope_id)).collect();
    assert_eq!(tables, vec![("mds_base_house", Some("aa"), 0), ("mds_base_owner", None, 1)]);

    let scopes: Vec<_> = analysis.scopes.iter().map(|scope| (scope.id, scope.parent_id)).collect();
    assert_eq!(scopes, vec![(0, None), (1, Some(0))]);

    let columns: Vec<_> = analysis
        .columns
        .iter()
        .map(|column| (column.qualifier.as_deref(), column.name.as_str(), column.scope_id))
        .collect();
    assert_eq!(columns, vec![(Some("aa"), "house_id", 0), (None, "HOUSE_ID", 1), (Some("aa"), "HOUSE_ID", 1)]);
}

#[test]
fn extracts_in_subquery_in_a_child_scope() {
    let sql = "select u.id from users u where u.id in (select o.user_id from orders o)";
    let analysis = analyze_sql_references(sql, Some("sqlserver")).unwrap();

    let tables: Vec<_> = analysis.tables.iter().map(|table| (table.name.as_str(), table.scope_id)).collect();
    assert_eq!(tables, vec![("users", 0), ("orders", 1)]);

    let scopes: Vec<_> = analysis.scopes.iter().map(|scope| (scope.id, scope.parent_id)).collect();
    assert_eq!(scopes, vec![(0, None), (1, Some(0))]);
}

#[test]
fn nested_queries_inherit_and_shadow_cte_names() {
    let sql = "WITH source AS (SELECT * FROM dbo.outer_source) SELECT * FROM source WHERE EXISTS (WITH source AS (SELECT * FROM dbo.inner_source) SELECT * FROM source) AND EXISTS (SELECT * FROM source)";
    let analysis = analyze_sql_references(sql, Some("sqlserver")).unwrap();

    let tables: Vec<_> =
        analysis.tables.iter().map(|table| (table.schema.as_deref(), table.name.as_str(), table.scope_id)).collect();
    assert_eq!(tables, vec![(Some("dbo"), "outer_source", 1), (Some("dbo"), "inner_source", 3)]);

    let scopes: Vec<_> = analysis.scopes.iter().map(|scope| (scope.id, scope.parent_id)).collect();
    assert_eq!(scopes, vec![(0, None), (1, Some(0)), (2, Some(0)), (3, Some(2)), (4, Some(0))]);
}

#[test]
fn extracts_unqualified_columns_from_single_table_select() {
    let analysis = analyze_sql_references("select missing, id from users", Some("postgres")).unwrap();

    let columns: Vec<_> =
        analysis.columns.iter().map(|column| (column.qualifier.as_deref(), column.name.as_str())).collect();
    assert_eq!(columns, vec![(None, "missing"), (None, "id")]);
}

#[test]
fn extracts_mysql_quoted_table_references() {
    let analysis = analyze_sql_references("SELECT * FROM `t_19991` LIMIT 100", Some("mysql")).unwrap();

    assert_eq!(analysis.tables.len(), 1);
    assert_eq!(analysis.tables[0].name, "t_19991");
    assert_eq!(analysis.tables[0].schema, None);
    assert_eq!(analysis.tables[0].span.start_line, 1);
    assert_eq!(analysis.tables[0].span.start_column, 15);
    assert_eq!(analysis.tables[0].span.end_line, 1);
    assert_eq!(analysis.tables[0].span.end_column, 24);
}

#[test]
fn extracts_mysql_qualified_backtick_table_references() {
    let analysis = analyze_sql_references("SELECT * FROM `core`.`products` LIMIT 100;", Some("mysql")).unwrap();

    assert_eq!(analysis.tables.len(), 1);
    assert_eq!(analysis.tables[0].schema.as_deref(), Some("core"));
    assert_eq!(analysis.tables[0].name, "products");
}

#[test]
fn extracts_mysql_single_quoted_table_references() {
    let analysis = analyze_sql_references("SELECT * FROM 't_10001' LIMIT 100", Some("mysql")).unwrap();

    assert_eq!(analysis.tables.len(), 1);
    assert_eq!(analysis.tables[0].name, "t_10001");
    assert_eq!(analysis.tables[0].schema, None);
}
