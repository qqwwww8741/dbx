use std::collections::HashSet;
use std::ops::ControlFlow;

use serde::{Deserialize, Serialize};

use crate::models::connection::DatabaseType;
use crate::sql::{find_statement_at_cursor, find_statement_at_cursor_for_database};
use crate::sql_dialect::{
    firebird_rows_clause, pagination_strategy, quote_table_identifier, PaginationContext, TablePaginationStrategy,
};
use sqlparser::ast::{
    visit_expressions, Expr, GroupByExpr, LimitClause, ObjectNamePart, OrderByKind, Select, SelectItem,
    SelectModifiers, SetExpr, Statement, TableFactor, Value, ValueWithSpan,
};
use sqlparser::dialect::{ClickHouseDialect, GenericDialect, MsSqlDialect, MySqlDialect};
use sqlparser::parser::Parser;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySqlBuildResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPagination {
    pub limit: usize,
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPaginationExecutionPlanOptions {
    pub sql: String,
    pub query_base_sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub pagination: QueryPagination,
    pub use_agent_cursor: bool,
    #[serde(default)]
    pub first_page_uses_actual_sql: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPaginationExecutionPlan {
    pub sql_to_execute: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_sql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_sql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_query_row_bound: Option<usize>,
    pub use_agent_result_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination_error: Option<String>,
    /// Trailing helper column added by DBX's ROWNUM pagination wrapper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination_row_number_column: Option<String>,
    /// True when the statement cannot be paginated server-side and must be
    /// executed once with the whole result streamed back (single execution).
    /// Only meaningful to in-process callers (query-result export); never
    /// serialized to the frontend.
    #[serde(skip)]
    pub single_execution: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedQuerySqlOptions {
    pub original_sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountQuerySqlOptions {
    pub original_sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuerySortDirection {
    Asc,
    Desc,
}

impl QuerySortDirection {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortedQuerySqlOptions {
    pub original_sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default)]
    pub result_columns: Vec<String>,
    pub column_index: usize,
    pub column: String,
    pub direction: QuerySortDirection,
}

pub fn build_query_pagination_execution_plan(
    options: QueryPaginationExecutionPlanOptions,
) -> QueryPaginationExecutionPlan {
    // Every page DBX generates for this query is derived from the same
    // user-written statement, so a literal LIMIT/TOP the user already wrote
    // bounds the query's total row count regardless of how large the
    // underlying table is — a cheap, exact upper bound that needs no
    // COUNT(*) execution. SQL Server's TOP is covered separately from the
    // standard LIMIT/OFFSET dialects (MySQL, Postgres, etc.) since they use
    // different clause syntax.
    let exact_query_row_bound = match pagination_strategy(options.database_type, PaginationContext::UserQuery) {
        TablePaginationStrategy::LimitOffset => top_level_limit_row_count(&options.query_base_sql),
        _ => None,
    };
    let mut plan = QueryPaginationExecutionPlan {
        sql_to_execute: options.sql.clone(),
        page_sql: None,
        page_limit: None,
        page_offset: None,
        count_sql: None,
        exact_query_row_bound,
        use_agent_result_session: false,
        pagination_error: None,
        pagination_row_number_column: None,
        single_execution: false,
    };

    let sql_server_cte = false;
    {
        let counted = build_count_query_sql(CountQuerySqlOptions {
            original_sql: options.query_base_sql.clone(),
            database_type: options.database_type,
        });
        if counted.ok {
            plan.count_sql = counted.sql;
        }
    }

    if options.pagination.session_id.is_some() {
        plan.page_limit = Some(options.pagination.limit);
        plan.page_offset = Some(options.pagination.offset);
        plan.use_agent_result_session = true;
        return plan;
    }

    {}

    {}

    let can_use_first_page_cursor = options.use_agent_cursor && options.pagination.offset == 0;
    // DB2, HighGo, OceanBase Oracle, and Xugu can spend substantially more time
    // executing an unbounded query before the Agent exposes its first cursor
    // page. Prefer a bounded SQL query whenever it can be rewritten safely.
    // Independent pages of an unordered query do not have a stable row order;
    // callers should add ORDER BY when that matters.
    // Kingbase keeps the cursor for unordered queries to preserve its behavior.
    let prefer_server_pagination = match options.database_type {
        _ => false,
    };
    if can_use_first_page_cursor && !prefer_server_pagination {
        if !options.first_page_uses_actual_sql && options.sql == options.query_base_sql {
            plan.sql_to_execute = options.query_base_sql;
        }
        plan.page_limit = Some(options.pagination.limit);
        plan.page_offset = Some(options.pagination.offset);
        plan.use_agent_result_session = true;
        return plan;
    }

    let paginated = build_paginated_query_sql(PaginatedQuerySqlOptions {
        original_sql: options.sql.clone(),
        database_type: options.database_type,
        limit: options.pagination.limit,
        offset: options.pagination.offset,
    });
    if paginated.ok {
        plan.sql_to_execute = paginated.sql.clone().unwrap_or_default();
        plan.page_sql = paginated.sql;
        plan.page_limit = Some(options.pagination.limit);
        plan.page_offset = Some(options.pagination.offset);
        {}
        {}
    } else if can_use_first_page_cursor && true {
        // Kingbase JDBC may buffer an entire result in auto-commit mode, so use
        // LIMIT/OFFSET whenever the statement can be rewritten safely. Keep the
        // Agent cursor as a bounded fallback for multi-statement or dialect-
        // specific SQL that the pagination parser cannot transform. HighGo does
        // not take this fallback: its JDBC driver may materialize the unbounded
        // result before cursor paging starts. Leaving the page metadata unset
        // routes it through regular execution and the configured JDBC maxRows
        // safeguard instead.
        if !options.first_page_uses_actual_sql && options.sql == options.query_base_sql {
            plan.sql_to_execute = options.query_base_sql;
        }
        plan.page_limit = Some(options.pagination.limit);
        plan.page_offset = Some(options.pagination.offset);
        plan.use_agent_result_session = true;
    } else {
    }
    plan
}

pub fn build_paginated_query_sql(options: PaginatedQuerySqlOptions) -> QuerySqlBuildResult {
    let Ok(statement) = single_selectable_statement(&options.original_sql, options.database_type) else {
        return err(single_statement_error_reason(&options.original_sql));
    };
    if unsupported_pagination_type(options.database_type) {
        return err("unsupported");
    }
    let safe_limit = options.limit.max(1);
    let safe_offset = options.offset;

    {}

    match pagination_strategy(options.database_type, PaginationContext::UserQuery) {
        TablePaginationStrategy::LimitOffset => {
            // Kingbase SQL Server compatibility mode and Xugu accept TOP as a real clause.
            // Appending LIMIT/OFFSET alongside a top-level TOP would be rejected by
            // the server ("multiple TOP/LIMIT clauses not allowed"), so fall back to
            // the Agent cursor / client-side row cap for such statements.
            {}
            let dedup_order_by = dedup_projection_count_without_order_by(&options.original_sql);
            ok(add_standard_limit(&statement, options.database_type, safe_limit, safe_offset, dedup_order_by))
        }
    }
}

pub fn build_count_query_sql(options: CountQuerySqlOptions) -> QuerySqlBuildResult {
    let Ok(statement) = single_selectable_statement(&options.original_sql, options.database_type) else {
        return err(single_statement_error_reason(&options.original_sql));
    };
    if unsupported_pagination_type(options.database_type) {
        return err("unsupported");
    }
    {}
    let (execution_hint, statement) = split_leading_execution_hint(&statement, options.database_type);
    // A locking clause does not affect cardinality and cannot appear inside
    // every dialect's derived-table count query. PostgreSQL permits pagination
    // after the lock clause; decline counting that uncommon order rather than
    // accidentally dropping the user's explicit LIMIT/OFFSET.
    let tokens = top_level_sql_tokens(statement);
    let statement = if let Some(index) = locking_clause_index(&tokens) {
        if has_pagination_clause_after(&tokens, index) {
            return err("locking");
        }
        statement[..index].trim_end().to_string()
    } else {
        statement.to_string()
    };
    // ES SQL can't wrap a SELECT in `SELECT COUNT(*) FROM (...)` — the
    // driver already reports the true match count via affected_rows.
    {}
    {}
    if options.database_type == Some(DatabaseType::Mysql) {
        return mysql_count_sql(&statement)
            .map(|sql| ok(format!("{execution_hint}{sql}")))
            .unwrap_or_else(|| err("unsupported"));
    }
    {}

    {}

    let alias = { quote_table_identifier(options.database_type, "dbx_count") };
    {}
    let wrapped_sql = match options.database_type {
        _ => statement,
    };
    ok(format!(
        "{execution_hint}{}",
        derived_table_sql("SELECT COUNT(*) AS dbx_total_rows FROM", &wrapped_sql, &format!("{alias};"))
    ))
}

pub fn build_sorted_query_sql(options: SortedQuerySqlOptions) -> QuerySqlBuildResult {
    let base_sql = options.original_sql.trim();
    if base_sql.is_empty() {
        return err("empty");
    }

    let mut statement = find_query_result_statement_at_cursor(base_sql, 0, options.database_type)
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    if statement.is_empty() {
        return err("empty");
    }
    let normalized_base_len = base_sql.trim_end_matches(';').trim().len();
    if statement.len() != normalized_base_len {
        if options.database_type != Some(DatabaseType::Mysql) {
            return err("multi");
        }
        statement = restore_leading_execution_hint(base_sql, &statement, options.database_type);
        if statement.len() != normalized_base_len {
            return err("multi");
        }
    }
    let (execution_hint, statement) = if options.database_type == Some(DatabaseType::Mysql) {
        split_leading_execution_hint(&statement, options.database_type)
    } else {
        ("", statement.as_str())
    };
    if statement.trim_start().to_ascii_uppercase().starts_with("WITH") {
        return err("with");
    }
    if !statement.trim_start().to_ascii_uppercase().starts_with("SELECT") {
        return err("not_select");
    }

    {}

    let uses_hive_subquery_syntax = false;
    {}

    let aliases = build_derived_column_aliases(&options.result_columns);
    // Caché/IRIS rejects derived-table column alias lists (`t(col, col)`)
    // outright (SQLCODE -25), regardless of delimited-identifier support.
    let use_derived_column_aliases = !uses_hive_subquery_syntax
        && options.database_type != Some(DatabaseType::Mysql)
        && true
        // Doris accepts the derived-table alias but not its column-name list.
        && true
        && true
        && true
        && true
        && true
        && true
        && true
        && true;
    let sort_alias = if use_derived_column_aliases {
        aliases
            .get(options.column_index)
            .or_else(|| {
                options
                    .result_columns
                    .iter()
                    .position(|column| column == &options.column)
                    .and_then(|index| aliases.get(index))
            })
            .cloned()
            .unwrap_or_else(|| fallback_alias(options.column_index))
    } else {
        options.result_columns.get(options.column_index).cloned().unwrap_or_else(|| options.column.clone())
    };
    // Oracle-compatible derived tables do not accept a PostgreSQL-style
    // column alias list. Use the selected column position when duplicate
    // labels would otherwise make ORDER BY ambiguous.
    let use_sort_ordinal = false;
    let sort_reference = { quote_table_identifier(options.database_type, &sort_alias) };
    let wrapped_statement = { statement.to_string() };

    if use_derived_column_aliases {
        let alias_list = aliases
            .iter()
            .map(|alias| quote_table_identifier(options.database_type, alias))
            .collect::<Vec<_>>()
            .join(", ");
        ok(format!(
            "{execution_hint}SELECT * FROM ({wrapped_statement}) t({alias_list}) ORDER BY {sort_reference} {};",
            options.direction.as_sql()
        ))
    } else {
        ok(format!(
            "{execution_hint}SELECT * FROM ({wrapped_statement}) t ORDER BY {sort_reference} {};",
            options.direction.as_sql()
        ))
    }
}

fn ok(sql: String) -> QuerySqlBuildResult {
    QuerySqlBuildResult { ok: true, sql: Some(sql), reason: None }
}

fn err(reason: &str) -> QuerySqlBuildResult {
    QuerySqlBuildResult { ok: false, sql: None, reason: Some(reason.to_string()) }
}

fn unsupported_pagination_type(database_type: Option<DatabaseType>) -> bool {
    // SOQL has no derived tables and no OFFSET wrapping; the Salesforce driver
    // pages through QueryLocator cursors (session_id) instead.
    false
}

fn find_query_result_statement_at_cursor(sql: &str, cursor_pos: usize, database_type: Option<DatabaseType>) -> String {
    if database_type == Some(DatabaseType::Mysql) {
        find_statement_at_cursor_for_database(sql, cursor_pos, DatabaseType::Mysql)
    } else {
        find_statement_at_cursor(sql, cursor_pos)
    }
}

fn single_selectable_statement(original_sql: &str, database_type: Option<DatabaseType>) -> Result<String, ()> {
    let base_sql = original_sql.trim();
    if base_sql.is_empty() {
        return Err(());
    }

    let extracted = find_query_result_statement_at_cursor(base_sql, 0, database_type)
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    if extracted.is_empty() {
        return Err(());
    }
    if !single_statement_matches_base_sql(&extracted, base_sql) {
        return Err(());
    }
    let statement = restore_leading_execution_hint(base_sql, &extracted, database_type);
    let statement_without_leading_comments =
        strip_leading_statement_comments(statement.trim_start_matches(';').trim_start());
    let upper = statement_without_leading_comments.to_ascii_uppercase();
    if upper.starts_with("WITH") {
        if !cte_main_statement_is_select(&statement) {
            return Err(());
        }
    } else if !upper.starts_with("SELECT") {
        return Err(());
    }
    if has_top_level_select_into(&statement) {
        return Err(());
    }

    Ok(statement)
}

fn single_statement_matches_base_sql(statement: &str, base_sql: &str) -> bool {
    let normalized_statement = statement.trim().trim_end_matches(';').trim();
    let normalized_base = base_sql.trim().trim_end_matches(';').trim();
    if normalized_statement.len() == normalized_base.len() {
        return true;
    }
    let base_without_leading_comments =
        strip_leading_statement_comments(normalized_base).trim().trim_end_matches(';').trim();
    normalized_statement == base_without_leading_comments
}

fn cte_main_statement_is_select(sql: &str) -> bool {
    let tokens = top_level_sql_tokens(sql);
    let mut index = match tokens.iter().position(|token| token.text == "WITH") {
        Some(index) => index + 1,
        None => return false,
    };

    if tokens.get(index).is_some_and(|token| token.text == "RECURSIVE") {
        index += 1;
    }

    while let Some(token) = tokens.get(index) {
        if is_with_main_statement_keyword(&token.text) {
            return token.text == "SELECT";
        }
        index += 1;
    }
    false
}

fn is_with_main_statement_keyword(token: &str) -> bool {
    matches!(token, "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "MERGE")
}

fn single_statement_error_reason(original_sql: &str) -> &'static str {
    let base_sql = original_sql.trim();
    if base_sql.is_empty() {
        return "empty";
    }
    let statement = find_statement_at_cursor(base_sql, 0).trim().trim_end_matches(';').trim().to_string();
    if statement.is_empty() {
        return "empty";
    }
    if statement.len() != base_sql.trim_end_matches(';').trim().len() {
        return "multi";
    }
    "not_select"
}

fn has_top_level_select_into(sql: &str) -> bool {
    let mut saw_select = false;
    for token in top_level_sql_tokens(sql) {
        if !saw_select {
            saw_select = token.text == "SELECT";
            continue;
        }
        if token.text == "INTO" {
            return true;
        }
    }
    false
}

fn skip_leading_sql_comments(sql: &str, mut index: usize) -> usize {
    loop {
        index = skip_sql_whitespace(sql, index);
        if sql[index..].starts_with("--") {
            index += 2;
            while index < sql.len() && next_char(sql, index) != '\n' {
                index += next_char(sql, index).len_utf8();
            }
            continue;
        }
        if sql[index..].starts_with("/*") {
            index += 2;
            while index < sql.len() {
                let ch = next_char(sql, index);
                let next = next_char_at(sql, index + ch.len_utf8());
                index += ch.len_utf8();
                if ch == '*' && next == Some('/') {
                    index += 1;
                    break;
                }
            }
            continue;
        }
        return index;
    }
}

fn restore_leading_execution_hint(original_sql: &str, statement: &str, database_type: Option<DatabaseType>) -> String {
    if leading_execution_hint_prefix_for_database(statement, database_type).is_some() {
        return statement.to_string();
    }
    let normalized = original_sql.trim().trim_end_matches(';').trim();
    match leading_execution_hint_prefix_for_database(normalized, database_type) {
        Some(prefix) => format!("{prefix}{statement}"),
        None => statement.to_string(),
    }
}

fn split_leading_execution_hint(sql: &str, database_type: Option<DatabaseType>) -> (&str, &str) {
    match leading_execution_hint_prefix_for_database(sql, database_type) {
        Some(prefix) => (prefix, sql[prefix.len()..].trim_start()),
        None => ("", sql),
    }
}

fn leading_execution_hint_prefix_for_database(sql: &str, database_type: Option<DatabaseType>) -> Option<&str> {
    if let Some(prefix) = leading_execution_hint_prefix(sql) {
        return Some(prefix);
    }
    if database_type != Some(DatabaseType::Mysql) {
        return None;
    }

    let executable_start = skip_leading_sql_comments(sql, 0);
    let directive_start = crate::tdsql_mysql::leading_directive_start(sql, executable_start)?;
    (directive_start < executable_start).then(|| &sql[directive_start..executable_start])
}

fn leading_execution_hint_prefix(sql: &str) -> Option<&str> {
    let mut index = 0;
    let mut hint_start = None;
    loop {
        index = skip_sql_whitespace(sql, index);
        let rest = &sql[index..];
        if rest.starts_with("--") {
            index += 2;
            while index < sql.len() && next_char(sql, index) != '\n' {
                index += next_char(sql, index).len_utf8();
            }
            continue;
        }
        if !rest.starts_with("/*") {
            return hint_start.map(|start| &sql[start..index]);
        }
        if hint_start.is_none() && matches!(next_char_at(sql, index + 2), Some('+' | '@' | '&')) {
            hint_start = Some(index);
        }
        index += 2;
        while index < sql.len() {
            let ch = next_char(sql, index);
            let next = next_char_at(sql, index + ch.len_utf8());
            index += ch.len_utf8();
            if ch == '*' && next == Some('/') {
                index += 1;
                break;
            }
        }
    }
}

fn strip_leading_statement_comments(sql: &str) -> &str {
    &sql[skip_leading_sql_comments(sql, 0)..]
}

fn skip_sql_whitespace(sql: &str, mut index: usize) -> usize {
    while index < sql.len() && next_char(sql, index).is_whitespace() {
        index += next_char(sql, index).len_utf8();
    }
    index
}

fn sql_keyword_at(sql: &str, index: usize, keyword: &str) -> bool {
    let Some(candidate) = sql.get(index..index + keyword.len()) else {
        return false;
    };
    if !candidate.eq_ignore_ascii_case(keyword) {
        return false;
    }
    let before_ok = index == 0 || !is_sql_token_part(next_char_before(sql, index));
    let after = index + keyword.len();
    let after_ok = after >= sql.len() || !is_sql_token_part(next_char(sql, after));
    before_ok && after_ok
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MysqlDerivedProjectionSafety {
    Safe,
    Ambiguous,
    Unknown,
}

fn mysql_count_sql(statement: &str) -> Option<String> {
    let dialect = MySqlDialect {};
    let Ok(mut statements) = Parser::parse_sql(&dialect, statement) else {
        return Some(mysql_wrapped_count_sql(statement));
    };
    let projection_safety = {
        let [Statement::Query(query)] = statements.as_slice() else {
            return None;
        };
        mysql_derived_table_set_projection_safety(query.body.as_ref())
    };
    if projection_safety != MysqlDerivedProjectionSafety::Ambiguous {
        return Some(mysql_wrapped_count_sql(statement));
    }

    let replacement_projection = match Parser::parse_sql(&dialect, "SELECT 1 AS dbx_count_value").ok()?.pop()? {
        Statement::Query(query) => match query.body.as_ref() {
            SetExpr::Select(select) => select.projection.clone(),
            _ => return None,
        },
        _ => return None,
    };
    {
        let [Statement::Query(query)] = statements.as_mut_slice() else {
            return None;
        };
        let SetExpr::Select(select) = query.body.as_mut() else {
            return None;
        };
        let group_by_is_empty = matches!(&select.group_by, GroupByExpr::Expressions(expressions, modifiers) if expressions.is_empty() && modifiers.is_empty());
        if select.distinct.is_some()
            || select.into.is_some()
            || !group_by_is_empty
            || select.having.is_some()
            || !select.projection.iter().all(mysql_projection_item_is_row_preserving)
        {
            return None;
        }

        select.projection = replacement_projection;
        // An ORDER BY can refer to a removed output alias; ordering never
        // changes how many rows survive MySQL LIMIT/OFFSET.
        query.order_by = None;
    }

    Some(mysql_wrapped_count_sql(&statements.pop()?.to_string()))
}

fn mysql_wrapped_count_sql(statement: &str) -> String {
    let alias = quote_table_identifier(Some(DatabaseType::Mysql), "dbx_count");
    derived_table_sql("SELECT COUNT(*) AS dbx_total_rows FROM", statement, &format!("{alias};"))
}

fn mysql_derived_table_set_projection_safety(set_expr: &SetExpr) -> MysqlDerivedProjectionSafety {
    match set_expr {
        SetExpr::Select(select) => mysql_derived_table_select_projection_safety(select),
        SetExpr::Query(query) => mysql_derived_table_set_projection_safety(query.body.as_ref()),
        SetExpr::SetOperation { left, .. } => mysql_derived_table_set_projection_safety(left),
        SetExpr::Values(_) | SetExpr::Table(_) => MysqlDerivedProjectionSafety::Safe,
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => {
            MysqlDerivedProjectionSafety::Unknown
        }
    }
}

fn mysql_derived_table_select_projection_safety(select: &Select) -> MysqlDerivedProjectionSafety {
    if select.projection.len() == 1 {
        return if matches!(select.projection.as_slice(), [SelectItem::Wildcard(_)])
            && (select.from.len() != 1 || !select.from[0].joins.is_empty())
        {
            MysqlDerivedProjectionSafety::Ambiguous
        } else {
            MysqlDerivedProjectionSafety::Safe
        };
    }
    if select
        .projection
        .iter()
        .any(|item| matches!(item, SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _)))
    {
        return MysqlDerivedProjectionSafety::Ambiguous;
    }

    let mut column_names = HashSet::with_capacity(select.projection.len());
    let mut unknown_name = false;
    for item in &select.projection {
        let Some(name) = derived_projection_name(item) else {
            unknown_name = true;
            continue;
        };
        if !column_names.insert(name.to_lowercase()) {
            return MysqlDerivedProjectionSafety::Ambiguous;
        }
    }
    if unknown_name {
        MysqlDerivedProjectionSafety::Unknown
    } else {
        MysqlDerivedProjectionSafety::Safe
    }
}

fn mysql_projection_item_is_row_preserving(item: &SelectItem) -> bool {
    match item {
        SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) => true,
        SelectItem::UnnamedExpr(Expr::Identifier(_) | Expr::CompoundIdentifier(_)) => true,
        SelectItem::ExprWithAlias { expr: Expr::Identifier(_) | Expr::CompoundIdentifier(_), .. } => true,
        SelectItem::UnnamedExpr(_) | SelectItem::ExprWithAlias { .. } | SelectItem::ExprWithAliases { .. } => false,
    }
}

fn derived_projection_name(item: &SelectItem) -> Option<&str> {
    match item {
        SelectItem::ExprWithAlias { alias, .. } => Some(&alias.value),
        SelectItem::UnnamedExpr(Expr::Identifier(identifier)) if !identifier.value.starts_with('@') => {
            Some(&identifier.value)
        }
        SelectItem::UnnamedExpr(Expr::CompoundIdentifier(identifiers)) => {
            identifiers.last().map(|identifier| identifier.value.as_str())
        }
        SelectItem::UnnamedExpr(_)
        | SelectItem::ExprWithAliases { .. }
        | SelectItem::QualifiedWildcard(_, _)
        | SelectItem::Wildcard(_) => None,
    }
}

fn has_top_level_limit(sql: &str) -> bool {
    top_level_sql_tokens(sql).iter().any(|token| token.text == "LIMIT")
}

/// True when the statement has a top-level `TOP` clause (SQL Server dialect).
/// Kingbase's SQL Server compatibility mode treats TOP as a real clause, so a
/// statement that already bounds rows with TOP must not receive a sibling LIMIT.
pub fn has_top_level_top(sql: &str) -> bool {
    top_level_sql_tokens(sql).iter().any(|token| token.text == "TOP")
}

/// Concrete row-count bound of a top-level `TOP` clause when written as a
/// literal (`TOP n`, `TOP(n)`, `TOP (n)`). Returns `None` for percentage TOP
/// (`TOP n PERCENT`), `WITH TIES` (the server may return more than `n` rows),
/// parenthesized expressions (`TOP (100 + 1)`, `TOP (100 * 2)`), or when the
/// TOP clause has no literal at all. A parenthesized form is only accepted when
/// it is exactly one integer literal followed by `)`, so the returned bound is
/// always exact — never a silent under-count.
pub fn top_level_top_row_count(sql: &str) -> Option<usize> {
    let tokens = top_level_sql_tokens(sql);
    let top_index = tokens.iter().position(|token| token.text == "TOP")?;
    let top_token = &tokens[top_index];
    // A modifier keyword directly after TOP (ALL / DISTINCT) means the following
    // literal is not a plain row-count bound. PERCENT / WITH TIES come after the
    // literal and are handled by the check below.
    if tokens.get(top_index + 1).is_some_and(|token| matches!(token.text.as_str(), "ALL" | "DISTINCT")) {
        return None;
    }
    let mut cursor = skip_sql_whitespace(sql, top_token.start + top_token.text.len());
    let parenthesized = sql.get(cursor..)?.starts_with('(');
    if parenthesized {
        cursor = skip_sql_whitespace(sql, cursor + 1);
    }
    let count = parse_usize_literal(sql, &mut cursor)?;
    let after = skip_sql_whitespace(sql, cursor);
    if parenthesized {
        // The parenthesized form must be exactly one integer literal followed by
        // `)`. Anything else is an expression whose real bound we cannot know
        // (e.g. TOP (100 + 1) returns 101 rows, not 100), so refuse to treat it
        // as a bound.
        if !sql.get(after..)?.starts_with(')') {
            return None;
        }
    }
    // `TOP n PERCENT` and `TOP n WITH TIES` (parenthesized or not) do not bound
    // the row count to the literal, so reject those adjacent modifiers. A later
    // table hint such as `FROM events WITH (NOLOCK)` is unrelated to TOP.
    let after_paren = if parenthesized { skip_sql_whitespace(sql, after + 1) } else { after };
    let modifier_index = tokens.iter().position(|token| token.start >= after_paren);
    if modifier_index.is_some_and(|index| {
        tokens[index].text == "PERCENT"
            || (tokens[index].text == "WITH" && tokens.get(index + 1).is_some_and(|token| token.text == "TIES"))
    }) {
        return None;
    }
    Some(count)
}

fn top_level_limit_row_count(sql: &str) -> Option<usize> {
    let tokens = top_level_sql_tokens(sql);
    let limit_index = tokens.iter().position(|token| token.text == "LIMIT")?;
    let token = &tokens[limit_index];
    let (count, suffix_start) = parse_standard_limit_row_count(sql, token.start + token.text.len())?;
    let suffix_start = skip_sql_whitespace(sql, suffix_start);
    if sql.get(suffix_start..)?.starts_with('%') {
        return None;
    }
    if tokens[limit_index + 1..].iter().enumerate().any(|(offset, token)| {
        token.text == "BY"
            || token.text == "PERCENT"
            || (token.text == "WITH" && tokens.get(limit_index + offset + 2).is_some_and(|next| next.text == "TIES"))
    }) {
        return None;
    }
    Some(count)
}

fn parse_standard_limit_row_count(sql: &str, start: usize) -> Option<(usize, usize)> {
    let mut cursor = skip_sql_whitespace(sql, start);
    let first = parse_usize_literal(sql, &mut cursor)?;
    cursor = skip_sql_whitespace(sql, cursor);
    if sql.get(cursor..)?.starts_with(',') {
        cursor = skip_sql_whitespace(sql, cursor + 1);
        let count = parse_usize_literal(sql, &mut cursor)?;
        return Some((count, cursor));
    }
    Some((first, cursor))
}

fn parse_usize_literal(sql: &str, cursor: &mut usize) -> Option<usize> {
    let start = *cursor;
    while *cursor < sql.len() && sql.as_bytes()[*cursor].is_ascii_digit() {
        *cursor += 1;
    }
    if *cursor == start {
        return None;
    }
    sql[start..*cursor].parse().ok()
}

fn add_standard_limit(
    statement: &str,
    database_type: Option<DatabaseType>,
    limit: usize,
    offset: usize,
    dedup_order_by: Option<Vec<usize>>,
) -> String {
    let order_sql = dedup_order_by.as_deref().map_or(String::new(), format_positional_order_by);

    if has_top_level_limit(statement) {
        if !order_sql.is_empty() {
            // For dedup queries (DISTINCT / GROUP BY) without user ORDER BY,
            // wrap the query to guarantee deterministic LIMIT/OFFSET pagination.
            // The inner query preserves DISTINCT semantics; the outer query
            // adds ORDER BY on positional columns to ensure consistent row
            // ordering across pages in distributed databases like Doris.
            return add_outer_standard_limit(statement, database_type, limit, offset, &order_sql);
        }
        // A user/top-level LIMIT can still be wider than the selected grid page size.
        // Wrap it so the first page respects the UI page limit while preserving the user's cap.
        if offset > 0 || top_level_limit_row_count(statement).is_some_and(|row_count| row_count > limit) {
            return add_outer_standard_limit(statement, database_type, limit, offset, "");
        }
        return format!("{statement};");
    }
    let offset_sql = if offset > 0 { format!(" OFFSET {offset}") } else { String::new() };
    let limit_sql = format!("{order_sql} LIMIT {limit}{offset_sql}");
    {}
    append_or_insert_before_locking(statement, &limit_sql)
}

fn add_outer_standard_limit(
    statement: &str,
    database_type: Option<DatabaseType>,
    limit: usize,
    offset: usize,
    order_sql: &str,
) -> String {
    let alias = quote_table_identifier(database_type, "dbx_page");
    derived_table_sql("SELECT * FROM", statement, &format!("{alias}{order_sql} LIMIT {limit} OFFSET {offset};"))
}

/// Insert pagination before a top-level locking clause; SQL dialects require
/// LIMIT/FETCH to precede FOR UPDATE, FOR SHARE, or LOCK IN SHARE MODE.
fn append_or_insert_before_locking(statement: &str, clause: &str) -> String {
    let clause = clause.trim();
    if let Some(index) = locking_clause_index(&top_level_sql_tokens(statement)) {
        let before = statement[..index].trim_end();
        let after = statement[index..].trim_start();
        let separator = if sql_suffix_needs_newline(before) { "\n" } else { " " };
        return format!("{before}{separator}{clause} {after};");
    }
    append_sql_suffix(statement, &format!("{clause};"))
}

const LOCKING_CLAUSE_PATTERNS: &[&[&str]] = &[
    &["FOR", "UPDATE"],
    &["FOR", "SHARE"],
    &["FOR", "KEY", "SHARE"],
    &["FOR", "NO", "KEY", "UPDATE"],
    &["LOCK", "IN", "SHARE", "MODE"],
];

fn locking_clause_index(tokens: &[SqlToken]) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        LOCKING_CLAUSE_PATTERNS
            .iter()
            .any(|pattern| token_sequence_matches(&tokens[index..], pattern))
            .then_some(token.start)
    })
}

fn token_sequence_matches(tokens: &[SqlToken], expected: &[&str]) -> bool {
    tokens.len() >= expected.len() && tokens.iter().zip(expected).all(|(token, expected)| token.text == *expected)
}

fn has_pagination_clause_after(tokens: &[SqlToken], index: usize) -> bool {
    tokens.iter().any(|token| token.start > index && matches!(token.text.as_str(), "LIMIT" | "OFFSET" | "FETCH"))
}

fn derived_table_sql(prefix: &str, statement: &str, suffix: &str) -> String {
    format!("{prefix} ({}) {suffix}", statement_for_sql_suffix(statement))
}

fn append_sql_suffix(statement: &str, suffix: &str) -> String {
    let separator = if sql_suffix_needs_newline(statement) { "\n" } else { " " };
    format!("{}{separator}{}", statement.trim_end(), suffix.trim_start())
}

fn statement_for_sql_suffix(statement: &str) -> String {
    let trimmed = statement.trim_end();
    if sql_suffix_needs_newline(trimmed) {
        format!("{trimmed}\n")
    } else {
        trimmed.to_string()
    }
}

fn sql_suffix_needs_newline(sql: &str) -> bool {
    let Some(last_line_start) = sql.rfind(['\n', '\r']).map(|index| index + 1) else {
        return line_has_open_line_comment(sql);
    };
    line_has_open_line_comment(&sql[last_line_start..])
}

fn line_has_open_line_comment(line: &str) -> bool {
    let mut index = 0;
    while index < line.len() {
        let ch = next_char(line, index);
        let next = next_char_at(line, index + ch.len_utf8());
        if matches!(ch, '\'' | '"' | '`') {
            index = skip_sql_quoted(line, index, ch);
            continue;
        }
        if ch == '[' {
            index = skip_sql_bracket_identifier(line, index);
            continue;
        }
        if ch == '/' && next == Some('*') {
            index += 2;
            while index < line.len() {
                let current = next_char(line, index);
                let following = next_char_at(line, index + current.len_utf8());
                index += current.len_utf8();
                if current == '*' && following == Some('/') {
                    index += 1;
                    break;
                }
            }
            continue;
        }
        if ch == '-' && next == Some('-') {
            return true;
        }
        if ch == '#' {
            return true;
        }
        index += ch.len_utf8();
    }
    false
}

/// For dedup queries (DISTINCT / GROUP BY) without an ORDER BY clause, generate
/// a positional `ORDER BY 1, 2, ..., N` clause so that LIMIT/OFFSET pagination
/// returns deterministic results across pages.  This is especially important for
/// distributed databases (e.g. Doris, StarRocks) where tablet scan order varies
/// between independent query executions.
fn format_positional_order_by(positions: &[usize]) -> String {
    if positions.is_empty() {
        return String::new();
    }
    let cols: Vec<String> = positions.iter().map(|position| position.to_string()).collect();
    format!(" ORDER BY {}", cols.join(", "))
}

/// Detect dedup queries (SELECT DISTINCT, GROUP BY, HAVING) that lack a
/// top-level ORDER BY clause.  Returns the 1-based projection positions to sort
/// on so that a positional ORDER BY can be injected for deterministic pagination.
///
/// A GROUP BY query sorts on its grouping keys alone: the keys already identify
/// an output row uniquely, and some engines reject sorting on aggregate outputs.
///
/// Returns `None` for:
///   - Non-SELECT queries
///   - Queries without dedup semantics
///   - Queries that already specify ORDER BY
///   - Wildcard projections (`SELECT *`)
///   - Parse failures
fn dedup_projection_count_without_order_by(sql: &str) -> Option<Vec<usize>> {
    let dialect = GenericDialect {};
    let statements = Parser::parse_sql(&dialect, sql).ok()?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return None;
    };
    // Reject if the query already has an ORDER BY clause.
    if query.order_by.is_some() {
        return None;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    let has_distinct = select.distinct.is_some();
    let has_group_by = !matches!(&select.group_by, GroupByExpr::Expressions(exprs, _) if exprs.is_empty());
    let has_having = select.having.is_some();
    if !has_distinct && !has_group_by && !has_having {
        return None;
    }
    // Wildcard projections cannot be used with positional ORDER BY.
    if select.projection.len() == 1 && matches!(select.projection.first(), Some(SelectItem::Wildcard(_))) {
        return None;
    }
    let all_positions: Vec<usize> = (1..=select.projection.len()).collect();
    if has_distinct {
        return Some(all_positions);
    }
    if let GroupByExpr::Expressions(exprs, _) = &select.group_by {
        if let Some(positions) = group_by_key_positions(exprs, &select.projection) {
            return Some(positions);
        }
    }
    Some(all_positions)
}

/// Map every GROUP BY key onto the 1-based position of the output column that
/// exposes it.  Returns `None` when a key is not projected (sorting on the keys
/// alone is then impossible) or when a wildcard hides which position is which,
/// so callers fall back to sorting on the full projection.
fn group_by_key_positions(group_by: &[Expr], projection: &[SelectItem]) -> Option<Vec<usize>> {
    if group_by.is_empty() {
        return None;
    }
    if !projection.iter().all(|item| matches!(item, SelectItem::UnnamedExpr(_) | SelectItem::ExprWithAlias { .. })) {
        return None;
    }
    let mut positions: Vec<usize> = Vec::with_capacity(group_by.len());
    for key in group_by {
        let position = group_by_key_position(key, projection)?;
        if !positions.contains(&position) {
            positions.push(position);
        }
    }
    positions.sort_unstable();
    Some(positions)
}

fn group_by_key_position(key: &Expr, projection: &[SelectItem]) -> Option<usize> {
    // `GROUP BY 1` is already a projection position.
    if let Expr::Value(ValueWithSpan { value: Value::Number(number, _), .. }) = key {
        let position = number.parse::<usize>().ok()?;
        return (1..=projection.len()).contains(&position).then_some(position);
    }
    let key_sql = key.to_string();
    projection
        .iter()
        .position(|item| match item {
            SelectItem::UnnamedExpr(expr) => expr.to_string() == key_sql,
            SelectItem::ExprWithAlias { expr, alias } => alias.value == key_sql || expr.to_string() == key_sql,
            _ => false,
        })
        .map(|index| index + 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqlToken {
    text: String,
    start: usize,
}

fn top_level_sql_tokens(sql: &str) -> Vec<SqlToken> {
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut depth = 0usize;

    while i < sql.len() {
        let ch = next_char(sql, i);
        let next = next_char_at(sql, i + ch.len_utf8());

        if ch == '-' && next == Some('-') {
            i += 2;
            while i < sql.len() && next_char(sql, i) != '\n' {
                i += next_char(sql, i).len_utf8();
            }
            continue;
        }

        if ch == '#' {
            i += 1;
            while i < sql.len() && next_char(sql, i) != '\n' {
                i += next_char(sql, i).len_utf8();
            }
            continue;
        }

        if ch == '/' && next == Some('*') {
            i += 2;
            while i < sql.len() {
                let current = next_char(sql, i);
                let following = next_char_at(sql, i + current.len_utf8());
                if current == '*' && following == Some('/') {
                    i += 2;
                    break;
                }
                i += current.len_utf8();
            }
            continue;
        }

        // PostgreSQL dollar-quoted bodies may contain arbitrary SQL keywords.
        // Skip them before scanning for top-level clauses such as FOR UPDATE.
        if ch == '$' {
            if let Some(end) = skip_sql_dollar_quoted(sql, i) {
                i = end;
                continue;
            }
        }

        if matches!(ch, '\'' | '"' | '`') {
            i = skip_sql_quoted(sql, i, ch);
            continue;
        }

        if ch == '[' {
            i = skip_sql_bracket_identifier(sql, i);
            continue;
        }

        if ch == '(' {
            depth += 1;
            i += ch.len_utf8();
            continue;
        }

        if ch == ')' {
            depth = depth.saturating_sub(1);
            i += ch.len_utf8();
            continue;
        }

        if depth == 0 && is_sql_token_start(ch) {
            let start = i;
            i += ch.len_utf8();
            while i < sql.len() && is_sql_token_part(next_char(sql, i)) {
                i += next_char(sql, i).len_utf8();
            }
            tokens.push(SqlToken { text: sql[start..i].to_ascii_uppercase(), start });
            continue;
        }

        i += ch.len_utf8();
    }

    tokens
}

fn skip_sql_dollar_quoted(sql: &str, pos: usize) -> Option<usize> {
    let tag_end_offset = sql.get(pos + 1..)?.find('$')?;
    let tag_end = pos + 1 + tag_end_offset;
    let tag = &sql[pos + 1..tag_end];
    let valid_tag = tag.is_empty()
        || (tag.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && tag.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
    if !valid_tag {
        return None;
    }

    let delimiter = &sql[pos..=tag_end];
    let content_start = tag_end + 1;
    sql.get(content_start..)?.find(delimiter).map(|closing_offset| content_start + closing_offset + delimiter.len())
}

fn skip_sql_quoted(sql: &str, pos: usize, quote: char) -> usize {
    let mut i = pos + quote.len_utf8();
    while i < sql.len() {
        let ch = next_char(sql, i);
        let next = next_char_at(sql, i + ch.len_utf8());
        if ch == quote {
            if next == Some(quote) {
                i += ch.len_utf8() + quote.len_utf8();
                continue;
            }
            return i + ch.len_utf8();
        }
        if quote == '\'' && ch == '\\' {
            i += ch.len_utf8();
            if i < sql.len() {
                i += next_char(sql, i).len_utf8();
            }
            continue;
        }
        i += ch.len_utf8();
    }
    sql.len()
}

fn skip_sql_bracket_identifier(sql: &str, pos: usize) -> usize {
    let mut i = pos + 1;
    while i < sql.len() {
        let ch = next_char(sql, i);
        let next = next_char_at(sql, i + ch.len_utf8());
        if ch == ']' {
            if next == Some(']') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += ch.len_utf8();
    }
    sql.len()
}

fn is_sql_token_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_sql_token_part(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '#')
}

fn next_char(sql: &str, index: usize) -> char {
    sql[index..].chars().next().unwrap_or('\0')
}

fn next_char_before(sql: &str, index: usize) -> char {
    sql[..index].chars().next_back().unwrap_or('\0')
}

fn next_char_at(sql: &str, index: usize) -> Option<char> {
    if index >= sql.len() {
        None
    } else {
        sql[index..].chars().next()
    }
}

fn build_derived_column_aliases(result_columns: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    result_columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let base = normalize_alias_base(column, index);
            let count = seen.entry(base.clone()).and_modify(|value| *value += 1).or_insert(1);
            if *count == 1 {
                base
            } else {
                format!("{base}_{count}")
            }
        })
        .collect()
}

fn normalize_alias_base(column: &str, index: usize) -> String {
    let compact = column.split_whitespace().collect::<Vec<_>>().join("_");
    let safe = compact
        .chars()
        .map(|ch| if ch.is_alphanumeric() || matches!(ch, '_' | '$') { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if safe.is_empty() {
        fallback_alias(index)
    } else {
        safe
    }
}

fn fallback_alias(index: usize) -> String {
    format!("column_{}", index + 1)
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn mysql_for_update_places_limit_before_locking_clause() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM `test`\nwhere id=1 for update".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 100,
            offset: 0,
        });

        assert!(result.ok);
        assert_eq!(result.sql.unwrap(), "SELECT * FROM `test`\nwhere id=1 LIMIT 100 for update;");
    }

    #[test]
    fn locking_query_plan_keeps_server_pagination_and_count() {
        let sql = "SELECT * FROM `test`\nwhere id=1 for update".to_string();
        let plan = build_query_pagination_execution_plan(QueryPaginationExecutionPlanOptions {
            sql: sql.clone(),
            query_base_sql: sql.clone(),
            database_type: Some(DatabaseType::Mysql),
            pagination: QueryPagination { limit: 100, offset: 0, session_id: None },
            use_agent_cursor: false,
            first_page_uses_actual_sql: false,
        });

        assert_eq!(plan.sql_to_execute, "SELECT * FROM `test`\nwhere id=1 LIMIT 100 for update;");
        assert_eq!(plan.page_sql, Some("SELECT * FROM `test`\nwhere id=1 LIMIT 100 for update;".to_string()));
        assert_eq!(plan.page_limit, Some(100));
        assert_eq!(plan.page_offset, Some(0));
        assert_eq!(
            plan.count_sql,
            Some("SELECT COUNT(*) AS dbx_total_rows FROM (SELECT * FROM `test`\nwhere id=1) `dbx_count`;".to_string())
        );
        assert!(!plan.use_agent_result_session);
    }

    #[test]
    fn locking_query_later_page_places_offset_before_locking_clause() {
        let sql = "SELECT * FROM t WHERE deleted = 0 FOR UPDATE SKIP LOCKED".to_string();
        let plan = build_query_pagination_execution_plan(QueryPaginationExecutionPlanOptions {
            sql: sql.clone(),
            query_base_sql: sql.clone(),
            database_type: Some(DatabaseType::Mysql),
            pagination: QueryPagination { limit: 100, offset: 100, session_id: None },
            use_agent_cursor: false,
            first_page_uses_actual_sql: false,
        });

        assert_eq!(
            plan.sql_to_execute,
            "SELECT * FROM t WHERE deleted = 0 LIMIT 100 OFFSET 100 FOR UPDATE SKIP LOCKED;"
        );
        assert_eq!(plan.page_limit, Some(100));
        assert_eq!(plan.page_offset, Some(100));
    }

    #[test]
    fn locking_query_count_removes_locking_clause() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "SELECT * FROM t WHERE deleted = 0 FOR UPDATE".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql,
            Some("SELECT COUNT(*) AS dbx_total_rows FROM (SELECT * FROM t WHERE deleted = 0) `dbx_count`;".to_string())
        );
    }

    #[test]
    fn nested_for_update_does_not_block_outer_limit_append() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM (SELECT id FROM t FOR UPDATE) locked".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 100,
            offset: 0,
        });

        assert!(result.ok);
        assert_eq!(result.sql.unwrap(), "SELECT * FROM (SELECT id FROM t FOR UPDATE) locked LIMIT 100;");
    }

    #[test]
    fn for_xml_is_not_treated_as_locking_clause() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT id, name FROM users FOR XML PATH('row')".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 100,
            offset: 0,
        });

        assert!(result.ok);
        assert_eq!(result.sql.unwrap(), "SELECT id, name FROM users FOR XML PATH('row') LIMIT 100;");
    }

    #[test]
    fn ordinary_select_still_appends_limit() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM t WHERE deleted = 0".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 100,
            offset: 0,
        });

        assert!(result.ok);
        assert_eq!(result.sql.unwrap(), "SELECT * FROM t WHERE deleted = 0 LIMIT 100;");
    }

    #[test]
    fn locking_keywords_inside_mysql_hash_comment_are_not_rewritten() {
        let sql = "SELECT * FROM t\n# FOR UPDATE LIMIT 1";
        let paginated = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: sql.to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 100,
            offset: 0,
        });
        let counted = build_count_query_sql(CountQuerySqlOptions {
            original_sql: sql.to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(paginated.sql.as_deref(), Some("SELECT * FROM t\n# FOR UPDATE LIMIT 1\nLIMIT 100;"));
        assert_eq!(
            counted.sql.as_deref(),
            Some("SELECT COUNT(*) AS dbx_total_rows FROM (SELECT * FROM t\n# FOR UPDATE LIMIT 1\n) `dbx_count`;")
        );
    }

    #[test]
    fn uses_mysql_style_alias_for_pagination() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT id FROM users WHERE active = 1".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "SELECT id FROM users WHERE active = 1 LIMIT 50;");
    }

    #[test]
    fn mysql_pagination_preserves_leading_ampersand_routing_hint() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "/*& tenant:'test' */\nSELECT id FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "/*& tenant:'test' */\nSELECT id FROM users LIMIT 50;");
    }

    #[test]
    fn mysql_pagination_preserves_supported_leading_execution_hints_only() {
        for hint in ["/*+ MAX_EXECUTION_TIME(1000) */", "/*@global:true*/", "/*& tenant:'test' */"] {
            let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
                original_sql: format!("{hint}\nSELECT id FROM users"),
                database_type: Some(DatabaseType::Mysql),
                limit: 50,
                offset: 0,
            });

            assert_eq!(result.sql.unwrap(), format!("{hint}\nSELECT id FROM users LIMIT 50;"));
        }

        let ordinary_comment = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "/* report query */\nSELECT id FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });
        assert_eq!(ordinary_comment.sql.unwrap(), "SELECT id FROM users LIMIT 50;");

        let ordinary_comment_before_hint = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "-- report query\n/*& tenant:'test' */\nSELECT id FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });
        assert_eq!(ordinary_comment_before_hint.sql.unwrap(), "/*& tenant:'test' */\nSELECT id FROM users LIMIT 50;");
    }

    #[test]
    fn mysql_pagination_preserves_exact_proxy_directive() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "/*proxy*/\nSELECT id FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "/*proxy*/\nSELECT id FROM users LIMIT 50;");
    }

    #[test]
    fn native_mysql_pagination_plan_preserves_exact_issue_directive() {
        let original_sql = "/*sets:allsets*/select @@server_id;";
        let plan = build_query_pagination_execution_plan(QueryPaginationExecutionPlanOptions {
            sql: original_sql.to_string(),
            query_base_sql: original_sql.to_string(),
            database_type: Some(DatabaseType::Mysql),
            pagination: QueryPagination { limit: 100, offset: 0, session_id: None },
            use_agent_cursor: false,
            first_page_uses_actual_sql: false,
        });

        assert_eq!(plan.sql_to_execute, "/*sets:allsets*/select @@server_id LIMIT 100;");
        assert_eq!(plan.page_sql.as_deref(), Some(plan.sql_to_execute.as_str()));
        assert_eq!(plan.page_limit, Some(100));
        assert_eq!(plan.page_offset, Some(0));
        assert_eq!(
            plan.count_sql.as_deref(),
            Some("/*sets:allsets*/SELECT COUNT(*) AS dbx_total_rows FROM (select @@server_id) `dbx_count`;")
        );
        assert!(!plan.use_agent_result_session);
    }

    #[test]
    fn mysql_count_preserves_leading_ampersand_routing_hint() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "/*& tenant:'test' */\nSELECT id FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.unwrap(),
            "/*& tenant:'test' */\nSELECT COUNT(*) AS dbx_total_rows FROM (SELECT id FROM users) `dbx_count`;"
        );
    }

    #[test]
    fn mysql_count_preserves_directives_at_outermost_start() {
        for prefix in [
            "/*sets:allsets*/",
            "/*master*/",
            "/*slave:set_1781591902_7*/",
            "/*future-route:anywhere*/",
            "/*proxy*/\n",
            "/*+ MAX_EXECUTION_TIME(1000) */\n",
            "/*@global:true*/\n",
            "/*& tenant:'test' */\n",
        ] {
            let result = build_count_query_sql(CountQuerySqlOptions {
                original_sql: format!("{prefix}select @@server_id;"),
                database_type: Some(DatabaseType::Mysql),
            });

            assert_eq!(
                result.sql.unwrap(),
                format!("{prefix}SELECT COUNT(*) AS dbx_total_rows FROM (select @@server_id) `dbx_count`;")
            );
        }
    }

    #[test]
    fn mysql_generated_queries_keep_standalone_comments_non_executable() {
        for original_sql in ["/* report query */\nSELECT id FROM users", "-- report query\nSELECT id FROM users"] {
            let page = build_paginated_query_sql(PaginatedQuerySqlOptions {
                original_sql: original_sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
                limit: 50,
                offset: 0,
            });
            let count = build_count_query_sql(CountQuerySqlOptions {
                original_sql: original_sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
            });
            let sort = build_sorted_query_sql(SortedQuerySqlOptions {
                original_sql: original_sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
                result_columns: vec!["id".to_string()],
                column_index: 0,
                column: "id".to_string(),
                direction: QuerySortDirection::Asc,
            });

            assert_eq!(page.sql.unwrap(), "SELECT id FROM users LIMIT 50;");
            assert_eq!(
                count.sql.unwrap(),
                "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT id FROM users) `dbx_count`;"
            );
            assert_eq!(sort, err("multi"));
        }
    }

    #[test]
    fn mysql_generated_queries_reject_non_executable_and_multi_statement_inputs() {
        for original_sql in ["/*sets:allsets*/", "/*sets:allsets SELECT id FROM users"] {
            let page = build_paginated_query_sql(PaginatedQuerySqlOptions {
                original_sql: original_sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
                limit: 50,
                offset: 0,
            });
            let count = build_count_query_sql(CountQuerySqlOptions {
                original_sql: original_sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
            });

            assert!(!page.ok, "{original_sql}");
            assert!(page.sql.is_none(), "{original_sql}");
            assert!(!count.ok, "{original_sql}");
            assert!(count.sql.is_none(), "{original_sql}");
        }

        let multi = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "/*sets:allsets*/SELECT 1; SELECT 2".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });
        assert_eq!(multi, err("multi"));
    }

    #[test]
    fn mysql_pagination_does_not_wrap_duplicate_result_columns() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT p.id, t.id FROM table1 p LEFT JOIN table2 t ON p.f = t.f".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 100,
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT p.id, t.id FROM table1 p LEFT JOIN table2 t ON p.f = t.f LIMIT 50 OFFSET 100;"
        );
    }

    #[test]
    fn mysql_pagination_keeps_limit_outside_trailing_line_comment() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT 1 AS id\n-- tail comment".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "SELECT 1 AS id\n-- tail comment\nLIMIT 50;");
    }

    #[test]
    fn mysql_pagination_keeps_limit_outside_trailing_hash_comment() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT 1 AS id\n# tail comment".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "SELECT 1 AS id\n# tail comment\nLIMIT 50;");
    }

    #[test]
    fn mysql_pagination_keeps_existing_top_level_limit() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT id FROM users LIMIT 20;".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 50,
            offset: 0,
        });

        assert_eq!(result.sql.unwrap(), "SELECT id FROM users LIMIT 20;");
    }

    #[test]
    fn mysql_pagination_wraps_wide_existing_limit_on_first_page() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000;".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 500,
            offset: 0,
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT * FROM (SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000) `dbx_page` LIMIT 500 OFFSET 0;"
        );
    }

    #[test]
    fn mysql_pagination_wraps_comma_limit_row_count_on_first_page() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM users LIMIT 20, 10000;".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 500,
            offset: 0,
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT * FROM (SELECT * FROM users LIMIT 20, 10000) `dbx_page` LIMIT 500 OFFSET 0;"
        );
    }

    #[test]
    fn mysql_pagination_wraps_existing_limit_for_later_pages() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000;".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 1000,
            offset: 1000,
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT * FROM (SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000) `dbx_page` LIMIT 1000 OFFSET 1000;"
        );
    }

    #[test]
    fn builds_count_query() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "WITH cte AS (SELECT 1 AS id) SELECT * FROM cte".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT COUNT(*) AS dbx_total_rows FROM (WITH cte AS (SELECT 1 AS id) SELECT * FROM cte) `dbx_count`;"
        );
    }

    #[test]
    fn mysql_count_rewrites_ambiguous_join_projection() {
        for sql in [
            "SELECT a.*, b.* FROM a JOIN b ON b.a_id = a.id ORDER BY b.id",
            "SELECT * FROM a JOIN b ON b.a_id = a.id ORDER BY b.id",
        ] {
            let result = build_count_query_sql(CountQuerySqlOptions {
                original_sql: sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
            });

            assert_eq!(
                result.sql.as_deref(),
                Some(
                    "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT 1 AS dbx_count_value FROM a JOIN b ON b.a_id = a.id) `dbx_count`;"
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn mysql_count_rewrites_duplicate_explicit_names() {
        for sql in [
            "SELECT a.id AS id, b.id AS id FROM a JOIN b ON b.a_id = a.id",
            "SELECT a.`1111`, b.`1111` FROM a JOIN b ON b.a_id = a.id",
        ] {
            let result = build_count_query_sql(CountQuerySqlOptions {
                original_sql: sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
            });

            assert_eq!(
                result.sql.as_deref(),
                Some(
                    "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT 1 AS dbx_count_value FROM a JOIN b ON b.a_id = a.id) `dbx_count`;"
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn mysql_count_keeps_unique_projection_wrapper() {
        let sql = "SELECT a.id AS a_id, b.id AS b_id FROM a JOIN b ON b.a_id = a.id ORDER BY b.id";
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: sql.to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.as_deref(),
            Some(
                "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT a.id AS a_id, b.id AS b_id FROM a JOIN b ON b.a_id = a.id ORDER BY b.id) `dbx_count`;"
            )
        );
    }

    #[test]
    fn mysql_count_rewrite_preserves_limit_and_offset() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql:
                "SELECT a.*, b.* FROM a JOIN b ON b.a_id = a.id ORDER BY FIELD(b.id, 11, 10) LIMIT 2 OFFSET 1"
                    .to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.as_deref(),
            Some(
                "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT 1 AS dbx_count_value FROM a JOIN b ON b.a_id = a.id LIMIT 2 OFFSET 1) `dbx_count`;"
            )
        );
    }

    #[test]
    fn mysql_count_rejects_ambiguous_cardinality_dependent_queries() {
        for sql in [
            "SELECT DISTINCT a.id AS id, b.id AS id FROM a JOIN b ON b.a_id = a.id",
            "SELECT a.id AS id, b.id AS id FROM a JOIN b ON b.a_id = a.id GROUP BY a.id, b.id",
            "SELECT a.id AS id, b.id AS id FROM a JOIN b ON b.a_id = a.id HAVING id > 0",
            "SELECT a.id AS id, b.id AS id FROM a JOIN b ON b.a_id = a.id UNION ALL SELECT c.id AS id, d.id AS id FROM c JOIN d ON d.c_id = c.id",
            "SELECT COUNT(*) AS id, SUM(a.id) AS id FROM a",
        ] {
            let result = build_count_query_sql(CountQuerySqlOptions {
                original_sql: sql.to_string(),
                database_type: Some(DatabaseType::Mysql),
            });

            assert_eq!(result, err("unsupported"), "{sql}");
        }
    }

    #[test]
    fn mysql_count_rejects_ambiguous_select_into() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "SELECT a.id AS id, b.id AS id INTO OUTFILE 'dump.tsv' FROM a JOIN b ON b.a_id = a.id"
                .to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert!(!result.ok);
        assert!(result.sql.is_none());
    }

    #[test]
    fn mysql_count_keeps_parse_failure_fallback() {
        let sql = "SELECT a.id AS id, FROM a";
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: sql.to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.as_deref(),
            Some("SELECT COUNT(*) AS dbx_total_rows FROM (SELECT a.id AS id, FROM a) `dbx_count`;")
        );
    }

    #[test]
    fn count_query_keeps_wrapper_outside_trailing_line_comment() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "SELECT 1 AS id\n-- tail comment".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT 1 AS id\n-- tail comment\n) `dbx_count`;"
        );
    }

    #[test]
    fn count_query_keeps_wrapper_outside_trailing_hash_comment() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "SELECT 1 AS id\n# tail comment".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT 1 AS id\n# tail comment\n) `dbx_count`;"
        );
    }

    #[test]
    fn count_query_preserves_user_limit() {
        let result = build_count_query_sql(CountQuerySqlOptions {
            original_sql: "SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000".to_string(),
            database_type: Some(DatabaseType::Mysql),
        });

        assert_eq!(
            result.sql.unwrap(),
            "SELECT COUNT(*) AS dbx_total_rows FROM (SELECT * FROM dy_promotion_item WHERE create_time < '2026-06-01' LIMIT 10000) `dbx_count`;"
        );
    }

    #[test]
    fn top_level_top_row_count_extracts_only_concrete_bounds() {
        assert_eq!(top_level_top_row_count("SELECT TOP 100 * FROM events"), Some(100));
        assert_eq!(top_level_top_row_count("SELECT TOP(100) * FROM events"), Some(100));
        assert_eq!(top_level_top_row_count("SELECT TOP (100) * FROM events"), Some(100));
        // Whitespace-only inside the parens is still a single literal bound.
        assert_eq!(top_level_top_row_count("SELECT TOP ( 100 ) * FROM events"), Some(100));
        assert_eq!(top_level_top_row_count("SELECT TOP 100 events.name FROM events ORDER BY events.name"), Some(100));
        assert_eq!(top_level_top_row_count("SELECT TOP 100 * FROM events LIMIT 5"), Some(100));

        // Parenthesized expressions have a real bound different from the leading
        // digits; refusing them (None) beats silently under-counting the export.
        assert_eq!(top_level_top_row_count("SELECT TOP (100 + 1) * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP (100 * 2) * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP (100 - 1) * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP (len(events.name)) * FROM events"), None);

        // Percentage TOP and WITH TIES are not concrete row-count bounds, with or
        // without parentheses.
        assert_eq!(top_level_top_row_count("SELECT TOP 10 PERCENT * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP (10) PERCENT * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP (2) WITH TIES * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP 2 WITH TIES * FROM events"), None);
        assert_eq!(top_level_top_row_count("SELECT TOP 2 * FROM events WITH (NOLOCK)"), Some(2));

        // No TOP clause at all.
        assert_eq!(top_level_top_row_count("SELECT * FROM events"), None);
        // TOP only inside a subquery is not a top-level clause.
        assert_eq!(top_level_top_row_count("SELECT * FROM (SELECT TOP 5 * FROM events) t"), None);
    }

    #[test]
    fn builds_mysql_sorted_query_without_alias_list() {
        let result = build_sorted_query_sql(SortedQuerySqlOptions {
            original_sql: "SELECT * FROM admin LIMIT 100;".to_string(),
            database_type: Some(DatabaseType::Mysql),
            result_columns: vec![
                "id".to_string(),
                "guid".to_string(),
                "role_guid".to_string(),
                "login_name".to_string(),
                "password".to_string(),
            ],
            column_index: 3,
            column: "login_name".to_string(),
            direction: QuerySortDirection::Asc,
        });

        assert_eq!(result.sql.unwrap(), "SELECT * FROM (SELECT * FROM admin LIMIT 100) t ORDER BY `login_name` ASC;");
    }

    #[test]
    fn mysql_sort_preserves_directives_at_outermost_start() {
        for prefix in [
            "/*sets:allsets*/",
            "/*master*/",
            "/*slave:set_1781591902_7*/",
            "/*future-route:anywhere*/",
            "/*proxy*/\n",
            "/*+ MAX_EXECUTION_TIME(1000) */\n",
            "/*@global:true*/\n",
            "/*& tenant:'test' */\n",
        ] {
            let result = build_sorted_query_sql(SortedQuerySqlOptions {
                original_sql: format!("{prefix}select @@server_id;"),
                database_type: Some(DatabaseType::Mysql),
                result_columns: vec!["@@server_id".to_string()],
                column_index: 0,
                column: "@@server_id".to_string(),
                direction: QuerySortDirection::Asc,
            });

            assert!(result.ok, "{prefix}: {result:?}");
            assert_eq!(
                result.sql.unwrap(),
                format!("{prefix}SELECT * FROM (select @@server_id) t ORDER BY `@@server_id` ASC;")
            );
        }
    }

    // -----------------------------------------------------------------------
    // Dedup query ORDER BY injection (DISTINCT / GROUP BY)
    // -----------------------------------------------------------------------

    #[test]
    fn dedup_count_detects_distinct_query_without_order_by() {
        assert_eq!(dedup_projection_count_without_order_by("SELECT DISTINCT a, b, c FROM t"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn dedup_count_detects_group_by_query() {
        assert_eq!(
            dedup_projection_count_without_order_by("SELECT city, COUNT(*) FROM users GROUP BY city"),
            Some(vec![1])
        );
    }

    #[test]
    fn dedup_count_returns_none_for_plain_select() {
        assert_eq!(dedup_projection_count_without_order_by("SELECT a, b FROM t"), None);
    }

    #[test]
    fn dedup_count_returns_none_when_order_by_exists() {
        assert_eq!(dedup_projection_count_without_order_by("SELECT DISTINCT a, b FROM t ORDER BY a"), None);
    }

    #[test]
    fn dedup_count_returns_none_for_wildcard() {
        assert_eq!(dedup_projection_count_without_order_by("SELECT DISTINCT * FROM t"), None);
    }

    #[test]
    fn mysql_distinct_query_first_page_gets_order_by() {
        let result = build_paginated_query_sql(PaginatedQuerySqlOptions {
            original_sql: "SELECT DISTINCT city FROM users".to_string(),
            database_type: Some(DatabaseType::Mysql),
            limit: 200,
            offset: 0,
        });

        assert!(result.ok);
        assert_eq!(result.sql.unwrap(), "SELECT DISTINCT city FROM users ORDER BY 1 LIMIT 200;");
    }

    // -----------------------------------------------------------------------
    // Complex queries with aliases, expressions, subqueries
    // -----------------------------------------------------------------------

    #[test]
    fn dedup_count_handles_aliases() {
        assert_eq!(
            dedup_projection_count_without_order_by("SELECT DISTINCT a AS x, b AS y, c AS z FROM t"),
            Some(vec![1, 2, 3])
        );
    }

    #[test]
    fn dedup_count_handles_expressions() {
        assert_eq!(
            dedup_projection_count_without_order_by(
                "SELECT DISTINCT a + b AS sum_col, CASE WHEN c > 0 THEN 'Y' ELSE 'N' END AS flag FROM t"
            ),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn dedup_count_handles_aggregate_with_alias() {
        assert_eq!(
            dedup_projection_count_without_order_by(
                "SELECT city, COUNT(*) AS cnt, AVG(salary) AS avg_sal FROM users GROUP BY city"
            ),
            Some(vec![1])
        );
    }

    #[test]
    fn group_by_order_by_matches_key_by_output_alias() {
        assert_eq!(
            dedup_projection_count_without_order_by(
                "SELECT SUM(salary) AS total, dept AS team FROM employees GROUP BY team"
            ),
            Some(vec![2])
        );
    }

    #[test]
    fn group_by_order_by_accepts_positional_keys() {
        assert_eq!(
            dedup_projection_count_without_order_by("SELECT city, dept, COUNT(*) FROM employees GROUP BY 1, 2"),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn group_by_order_by_falls_back_when_key_is_not_projected() {
        assert_eq!(
            dedup_projection_count_without_order_by(
                "SELECT COUNT(*) AS cnt, SUM(salary) AS total FROM employees GROUP BY city"
            ),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn group_by_order_by_falls_back_for_wildcard_projection() {
        assert_eq!(
            dedup_projection_count_without_order_by("SELECT e.*, COUNT(*) FROM employees e GROUP BY city"),
            Some(vec![1, 2])
        );
    }

    #[test]
    fn union_query_is_not_treated_as_dedup() {
        assert_eq!(dedup_projection_count_without_order_by("SELECT a FROM t1 UNION SELECT b FROM t2"), None);
    }
}
