use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, Ident, JoinConstraint, JoinOperator,
    ObjectName, ObjectNamePart, OrderByKind, Query, Select, SelectItem, SetExpr, Statement, TableFactor,
    TableWithJoins,
};
use sqlparser::dialect::{
    ClickHouseDialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
    SparkSqlDialect,
};
use sqlparser::keywords::Keyword;
use sqlparser::parser::{Parser, ParserError};
use sqlparser::tokenizer::{Span, Token, TokenWithSpan, Tokenizer};

use crate::models::connection::DatabaseType;
use crate::sql::{
    starts_with_duckdb_result_sql_keyword, starts_with_executable_sql_keyword, statement_ranges_for_database,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlReferenceAnalysis {
    pub tables: Vec<SqlTableReference>,
    pub columns: Vec<SqlColumnReference>,
    pub scopes: Vec<SqlReferenceScope>,
    /// SELECT projection columns that are neither aggregated nor listed in GROUP BY.
    #[serde(default)]
    pub group_by_violations: Vec<SqlGroupByViolation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlTableReference {
    pub name: String,
    pub schema: Option<String>,
    pub alias: Option<String>,
    pub span: SqlTextSpan,
    pub scope_id: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlColumnReference {
    pub name: String,
    pub qualifier: Option<String>,
    pub span: SqlTextSpan,
    pub scope_id: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlGroupByViolation {
    pub span: SqlTextSpan,
    pub column: String,
    pub qualifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlReferenceScope {
    pub id: usize,
    pub parent_id: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlTextSpan {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl From<Span> for SqlTextSpan {
    fn from(span: Span) -> Self {
        Self {
            start_line: span.start.line as usize,
            start_column: span.start.column as usize,
            end_line: span.end.line as usize,
            end_column: span.end.column as usize,
        }
    }
}

/// One SELECT projection item: the columns it references plus its output alias.
/// Columns are flagged `aggregated` when they sit inside an aggregate call or a
/// window function, which exempts them from the GROUP BY membership check.
#[derive(Default)]
struct ProjectionItemAcc {
    alias: Option<String>,
    columns: Vec<ProjectionColumnRef>,
}

#[derive(Clone)]
struct ProjectionColumnRef {
    name: String,
    qualifier: Option<String>,
    span: SqlTextSpan,
    aggregated: bool,
}

/// GROUP BY context of the SELECT currently being analyzed. Plain identifier
/// groups are recorded for membership matching; any other shape (ordinals,
/// computed expressions, `GROUP BY ALL`, rollups) marks the whole statement
/// unresolved so the conservative check skips it entirely.
#[derive(Default)]
struct GroupByContext {
    bare: HashSet<String>,
    qualified: HashSet<String>,
    unresolved: bool,
}

#[derive(Default)]
struct Analyzer {
    tables: Vec<SqlTableReference>,
    columns: Vec<SqlColumnReference>,
    scopes: Vec<SqlReferenceScope>,
    scope_stack: Vec<usize>,
    cte_scope_stack: Vec<HashSet<String>>,
    next_scope_id: usize,
    is_sqlserver: bool,
    is_spark: bool,
    group_by_violations: Vec<SqlGroupByViolation>,
    projection_items: Vec<ProjectionItemAcc>,
    current_projection_item: Option<ProjectionItemAcc>,
    aggregate_depth: usize,
    window_depth: usize,
    group_by: Option<GroupByContext>,
}

pub fn analyze_sql_references(sql: &str, dialect: Option<&str>) -> Result<SqlReferenceAnalysis, String> {
    let statements = Parser::parse_sql(&MySqlDialect {}, sql).map_err(|error| error.to_string())?;
    let mut analyzer = Analyzer::default();
    for statement in statements {
        analyzer.visit_statement(&statement);
    }

    let mut analysis = SqlReferenceAnalysis {
        tables: analyzer.tables,
        columns: analyzer.columns,
        scopes: analyzer.scopes,
        group_by_violations: analyzer.group_by_violations,
    };
    // Engines that do not enforce strict GROUP BY semantics (MySQL, SQLite,
    // DuckDB, ClickHouse, Spark) accept ungrouped projection columns — MySQL
    // enforces ONLY_FULL_GROUP_BY by default since 5.7 but the frontend has no
    // access to the connection's sql_mode, so skipping is the conservative
    // choice — flagging them there would be noise.
    {
        analysis.group_by_violations.clear();
    }
    Ok(analysis)
}

fn normalize_dialect(dialect: Option<&str>) -> String {
    match dialect.unwrap_or("generic").to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" | "redshift" | "opengauss" | "gaussdb" | "highgo" | "uxdb" | "questdb" => {
            "postgres".to_string()
        }
        "mysql" | "mariadb" | "doris" | "starrocks" | "manticoresearch" | "oceanbase" => "mysql".to_string(),
        "sqlite" => "sqlite".to_string(),
        "sqlserver" | "mssql" => "sqlserver".to_string(),
        "clickhouse" => "clickhouse".to_string(),
        "duckdb" => "duckdb".to_string(),
        "spark" | "sparksql" => "spark".to_string(),
        _ => "generic".to_string(),
    }
}

impl Analyzer {
    fn visit_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Query(query) => self.visit_query_in_new_scope(query, None),

            _ => {}
        }
    }

    fn visit_query_in_new_scope(&mut self, query: &Query, parent_id: Option<usize>) {
        let scope_id = self.next_scope_id;
        self.next_scope_id += 1;
        self.scopes.push(SqlReferenceScope { id: scope_id, parent_id });
        self.scope_stack.push(scope_id);
        self.cte_scope_stack.push(HashSet::new());
        self.visit_query(query);
        self.cte_scope_stack.pop();
        self.scope_stack.pop();
    }

    fn visit_child_query(&mut self, query: &Query) {
        self.visit_query_in_new_scope(query, self.current_scope_id());
    }

    fn current_scope_id(&self) -> Option<usize> {
        self.scope_stack.last().copied()
    }

    fn add_visible_cte(&mut self, ident: &Ident) {
        let key = self.cte_name_key(ident);
        if let Some(visible_ctes) = self.cte_scope_stack.last_mut() {
            visible_ctes.insert(key);
        }
    }

    fn is_visible_cte(&self, name: &ObjectName) -> bool {
        if name.0.len() != 1 {
            return false;
        }
        let Some(ident) = name.0.first().and_then(ObjectNamePart::as_ident) else {
            return false;
        };
        let key = self.cte_name_key(ident);
        self.cte_scope_stack.iter().rev().any(|visible_ctes| visible_ctes.contains(&key))
    }

    fn cte_name_key(&self, ident: &Ident) -> String {
        if self.is_sqlserver || ident.quote_style.is_none() {
            ident.value.to_ascii_lowercase()
        } else {
            ident.value.clone()
        }
    }

    fn visit_query(&mut self, query: &Query) {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                // Add each name before its body: recursive/self and earlier CTEs are visible, later CTEs are not.
                self.add_visible_cte(&cte.alias.name);
                self.visit_child_query(&cte.query);
            }
        }
        self.visit_set_expr(&query.body);
        if let Some(order_by) = &query.order_by {
            if let OrderByKind::Expressions(exprs) = &order_by.kind {
                for expr in exprs {
                    self.visit_expr(&expr.expr);
                }
            }
        }
    }

    fn visit_set_expr(&mut self, set_expr: &SetExpr) {
        match set_expr {
            SetExpr::Select(select) => self.visit_select(select),
            SetExpr::Query(query) => self.visit_child_query(query),
            SetExpr::SetOperation { left, right, .. } => {
                self.visit_set_expr_in_child_scope(left);
                self.visit_set_expr_in_child_scope(right);
            }
            _ => {}
        }
    }

    fn visit_set_expr_in_child_scope(&mut self, set_expr: &SetExpr) {
        let scope_id = self.next_scope_id;
        self.next_scope_id += 1;
        self.scopes.push(SqlReferenceScope { id: scope_id, parent_id: self.current_scope_id() });
        self.scope_stack.push(scope_id);
        self.visit_set_expr(set_expr);
        self.scope_stack.pop();
    }

    fn visit_select(&mut self, select: &Select) {
        // Nested selects (subqueries, derived tables, CTE bodies) run their own
        // projection/group-by collection; save the outer context so their columns
        // and violations never mix with this statement's.
        let outer_projection_items = std::mem::take(&mut self.projection_items);
        let outer_projection_item = self.current_projection_item.take();
        let outer_group_by = self.group_by.take();

        for table in &select.from {
            self.visit_table_with_joins(table);
        }

        for item in &select.projection {
            match item {
                SelectItem::UnnamedExpr(expr) => self.visit_projection_item(expr, None),
                SelectItem::ExprWithAlias { expr, alias } => {
                    self.visit_projection_item(expr, Some(alias.value.clone()))
                }
                SelectItem::ExprWithAliases { expr, .. } => self.visit_projection_item(expr, None),
                _ => {}
            }
        }

        if let Some(expr) = &select.prewhere {
            self.visit_expr(expr);
        }
        if let Some(expr) = &select.selection {
            self.visit_expr(expr);
        }
        let mut group_by_context = GroupByContext::default();
        if let GroupByExpr::Expressions(exprs, _) = &select.group_by {
            for expr in exprs {
                match plain_identifier_parts(expr) {
                    Some((qualifier, name)) => {
                        if let Some(qualifier) = qualifier {
                            group_by_context.qualified.insert(format!(
                                "{}.{}",
                                qualifier.to_ascii_lowercase(),
                                name.to_ascii_lowercase()
                            ));
                        } else {
                            group_by_context.bare.insert(name.to_ascii_lowercase());
                        }
                    }
                    None => group_by_context.unresolved = true,
                }
                self.visit_expr(expr);
            }
        } else if matches!(select.group_by, GroupByExpr::All(_)) {
            // GROUP BY ALL / rollup-style grouping has no explicit membership to match.
            group_by_context.unresolved = true;
        }
        self.group_by = Some(group_by_context);

        for expr in &select.cluster_by {
            self.visit_expr(expr);
        }
        for expr in &select.distribute_by {
            self.visit_expr(expr);
        }
        for expr in &select.sort_by {
            self.visit_expr(&expr.expr);
        }
        if let Some(expr) = &select.having {
            self.visit_expr(expr);
        }
        if let Some(expr) = &select.qualify {
            self.visit_expr(expr);
        }

        self.finish_group_by_check();
        self.projection_items = outer_projection_items;
        self.current_projection_item = outer_projection_item;
        self.group_by = outer_group_by;
    }

    fn visit_projection_item(&mut self, expr: &Expr, alias: Option<String>) {
        self.current_projection_item = Some(ProjectionItemAcc { alias, ..ProjectionItemAcc::default() });
        self.visit_expr(expr);
        if let Some(item) = self.current_projection_item.take() {
            self.projection_items.push(item);
        }
    }

    /// Flag SELECT projection columns that are neither aggregated nor part of the
    /// statement's GROUP BY. Conservative by design: unresolved GROUP BY shapes
    /// skip the check entirely, alias-based coverage covers `GROUP BY <alias>`.
    fn finish_group_by_check(&mut self) {
        let Some(context) = self.group_by.as_ref() else {
            return;
        };
        if context.unresolved || (context.bare.is_empty() && context.qualified.is_empty()) {
            return;
        }
        // Columns of projection items whose alias is what GROUP BY references
        // (`SELECT a AS x ... GROUP BY x`) count as grouped for identical column
        // names elsewhere in the projection.
        let mut alias_grouped_columns: HashSet<String> = HashSet::new();
        for item in &self.projection_items {
            let Some(alias) = item.alias.as_ref() else { continue };
            if !context.bare.contains(&alias.to_ascii_lowercase()) {
                continue;
            }
            for column in &item.columns {
                alias_grouped_columns.insert(column.name.to_ascii_lowercase());
            }
        }
        // Qualifier asymmetry: `u.name` in the projection and `users.name` in
        // GROUP BY refer to the same table when one side is the other's alias,
        // so normalize every qualifier to the underlying table name before
        // comparing.
        let qualifier_table_names: HashMap<String, String> = self
            .tables
            .iter()
            .filter_map(|table| {
                table.alias.as_ref().map(|alias| (alias.to_ascii_lowercase(), table.name.to_ascii_lowercase()))
            })
            .collect();
        let normalize_qualifier = |qualifier: &str| -> String {
            qualifier_table_names
                .get(&qualifier.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| qualifier.to_ascii_lowercase())
        };
        let grouped_qualified: HashSet<String> = context
            .qualified
            .iter()
            .map(|entry| match entry.split_once('.') {
                Some((qualifier, name)) => format!("{}.{}", normalize_qualifier(qualifier), name),
                None => entry.clone(),
            })
            .collect();
        let mut violations: Vec<SqlGroupByViolation> = Vec::new();
        for item in &self.projection_items {
            let item_alias_grouped =
                item.alias.as_ref().map(|alias| context.bare.contains(&alias.to_ascii_lowercase())).unwrap_or(false);
            for column in &item.columns {
                if column.aggregated || item_alias_grouped {
                    continue;
                }
                let name = column.name.to_ascii_lowercase();
                if context.bare.contains(&name) {
                    continue;
                }
                if let Some(qualifier) = &column.qualifier {
                    if grouped_qualified.contains(&format!("{}.{}", normalize_qualifier(qualifier), name)) {
                        continue;
                    }
                }
                if alias_grouped_columns.contains(&name) {
                    continue;
                }
                violations.push(SqlGroupByViolation {
                    span: column.span,
                    column: column.name.clone(),
                    qualifier: column.qualifier.clone(),
                });
            }
        }
        self.group_by_violations.extend(violations);
    }

    fn visit_table_with_joins(&mut self, table: &TableWithJoins) {
        self.visit_table_factor(&table.relation);
        for join in &table.joins {
            self.visit_table_factor(&join.relation);
            self.visit_join_operator(&join.join_operator);
        }
    }

    fn visit_join_operator(&mut self, operator: &JoinOperator) {
        match operator {
            JoinOperator::Join(constraint)
            | JoinOperator::Inner(constraint)
            | JoinOperator::Left(constraint)
            | JoinOperator::LeftOuter(constraint)
            | JoinOperator::Right(constraint)
            | JoinOperator::RightOuter(constraint)
            | JoinOperator::FullOuter(constraint)
            | JoinOperator::CrossJoin(constraint)
            | JoinOperator::Semi(constraint)
            | JoinOperator::LeftSemi(constraint)
            | JoinOperator::RightSemi(constraint)
            | JoinOperator::Anti(constraint)
            | JoinOperator::LeftAnti(constraint)
            | JoinOperator::RightAnti(constraint)
            | JoinOperator::StraightJoin(constraint) => self.visit_join_constraint(constraint),
            JoinOperator::AsOf { match_condition, constraint } => {
                self.visit_expr(match_condition);
                self.visit_join_constraint(constraint);
            }
            _ => {}
        }
    }

    fn visit_join_constraint(&mut self, constraint: &JoinConstraint) {
        match constraint {
            JoinConstraint::On(expr) => self.visit_expr(expr),
            JoinConstraint::Using(names) => {
                for name in names {
                    if let Some(ident) = object_name_last_ident(name) {
                        self.push_column(None, ident);
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_table_factor(&mut self, factor: &TableFactor) {
        match factor {
            TableFactor::Table { name, alias, args, .. } => {
                // Qualified names remain physical objects even when their final component matches a visible CTE.
                if args.is_none() && !self.is_visible_cte(name) {
                    if let Some(table) = table_reference_from_name(
                        name,
                        alias.as_ref().map(|a| a.name.value.clone()),
                        self.current_scope_id(),
                    ) {
                        self.tables.push(table);
                    }
                }
            }
            TableFactor::Derived { subquery, .. } => self.visit_child_query(subquery),
            TableFactor::NestedJoin { table_with_joins, .. } => self.visit_table_with_joins(table_with_joins),
            TableFactor::TableFunction { expr, .. } => self.visit_expr(expr),
            TableFactor::Function { args, .. } => {
                for arg in args {
                    self.visit_function_arg(arg);
                }
            }
            TableFactor::UNNEST { array_exprs, .. } => {
                for expr in array_exprs {
                    self.visit_expr(expr);
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Identifier(ident) => self.push_column(None, ident),
            Expr::CompoundIdentifier(idents) => {
                if idents.len() >= 2 {
                    let column = idents.last().expect("checked length");
                    let qualifier = idents.get(idents.len() - 2).map(|ident| ident.value.clone());
                    self.push_column(qualifier, column);
                }
            }
            Expr::CompoundFieldAccess { root, .. } | Expr::JsonAccess { value: root, .. } => self.visit_expr(root),
            Expr::IsFalse(expr)
            | Expr::IsNotFalse(expr)
            | Expr::IsTrue(expr)
            | Expr::IsNotTrue(expr)
            | Expr::IsNull(expr)
            | Expr::IsNotNull(expr)
            | Expr::IsUnknown(expr)
            | Expr::IsNotUnknown(expr)
            | Expr::UnaryOp { expr, .. }
            | Expr::Nested(expr) => self.visit_expr(expr),
            Expr::IsDistinctFrom(left, right)
            | Expr::IsNotDistinctFrom(left, right)
            | Expr::BinaryOp { left, right, .. }
            | Expr::AnyOp { left, right, .. }
            | Expr::AllOp { left, right, .. } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            Expr::InList { expr, list, .. } => {
                self.visit_expr(expr);
                for item in list {
                    self.visit_expr(item);
                }
            }
            Expr::InSubquery { expr, subquery, .. } => {
                self.visit_expr(expr);
                self.visit_child_query(subquery);
            }
            Expr::InUnnest { expr, array_expr, .. } => {
                self.visit_expr(expr);
                self.visit_expr(array_expr);
            }
            Expr::Between { expr, low, high, .. } => {
                self.visit_expr(expr);
                self.visit_expr(low);
                self.visit_expr(high);
            }
            Expr::Like { expr, pattern, .. }
            | Expr::ILike { expr, pattern, .. }
            | Expr::SimilarTo { expr, pattern, .. }
            | Expr::RLike { expr, pattern, .. } => {
                self.visit_expr(expr);
                self.visit_expr(pattern);
            }
            Expr::Cast { expr, .. }
            | Expr::Extract { expr, .. }
            | Expr::Ceil { expr, .. }
            | Expr::Floor { expr, .. } => self.visit_expr(expr),
            Expr::AtTimeZone { timestamp, time_zone } => {
                self.visit_expr(timestamp);
                self.visit_expr(time_zone);
            }
            Expr::Position { expr, r#in } => {
                self.visit_expr(expr);
                self.visit_expr(r#in);
            }
            Expr::Function(function) => {
                // Columns wrapped in an aggregate call or a window function are
                // exempt from the GROUP BY membership check.
                let function_name =
                    object_name_last_ident(&function.name).map(|ident| ident.value.to_ascii_lowercase());
                let is_aggregate = function_name.as_deref().map(is_aggregate_function_name).unwrap_or(false);
                let has_window = function.over.is_some();
                if is_aggregate {
                    self.aggregate_depth += 1;
                }
                if has_window {
                    self.window_depth += 1;
                }
                self.visit_function_args(&function.parameters);
                self.visit_function_call_args(&function.name, &function.args);
                if let Some(filter) = &function.filter {
                    self.visit_expr(filter);
                }
                for order in &function.within_group {
                    self.visit_expr(&order.expr);
                }
                if is_aggregate {
                    self.aggregate_depth -= 1;
                }
                if has_window {
                    self.window_depth -= 1;
                }
            }
            Expr::Case { operand, conditions, else_result, .. } => {
                if let Some(operand) = operand {
                    self.visit_expr(operand);
                }
                for condition in conditions {
                    self.visit_expr(&condition.condition);
                    self.visit_expr(&condition.result);
                }
                if let Some(else_result) = else_result {
                    self.visit_expr(else_result);
                }
            }
            Expr::Subquery(query) | Expr::Exists { subquery: query, .. } => self.visit_child_query(query),
            _ => {}
        }
    }

    fn visit_function_args(&mut self, args: &FunctionArguments) {
        self.visit_function_args_skipping(args, |_, _| false);
    }

    fn visit_function_call_args(&mut self, name: &ObjectName, args: &FunctionArguments) {
        let sqlserver_datepart_function = self.is_sqlserver.then(|| sqlserver_datepart_function_name(name)).flatten();
        self.visit_function_args_skipping(args, |index, arg| false);
    }

    fn visit_function_args_skipping(
        &mut self,
        args: &FunctionArguments,
        mut should_skip: impl FnMut(usize, &FunctionArg) -> bool,
    ) {
        match args {
            FunctionArguments::Subquery(query) => self.visit_child_query(query),
            FunctionArguments::List(list) => {
                for (index, arg) in list.args.iter().enumerate() {
                    if !should_skip(index, arg) {
                        self.visit_function_arg(arg);
                    }
                }
                for clause in &list.clauses {
                    if let sqlparser::ast::FunctionArgumentClause::OrderBy(items) = clause {
                        for item in items {
                            self.visit_expr(&item.expr);
                        }
                    }
                }
            }
            FunctionArguments::None => {}
        }
    }

    fn visit_function_arg(&mut self, arg: &FunctionArg) {
        match arg {
            FunctionArg::Named { arg, .. } | FunctionArg::ExprNamed { arg, .. } | FunctionArg::Unnamed(arg) => {
                if let FunctionArgExpr::Expr(expr) = arg {
                    self.visit_expr(expr);
                }
            }
        }
    }

    fn push_column(&mut self, qualifier: Option<String>, ident: &Ident) {
        let span: SqlTextSpan = ident.span.into();
        if let Some(scope_id) = self.current_scope_id() {
            self.columns.push(SqlColumnReference {
                name: ident.value.clone(),
                qualifier: qualifier.clone(),
                span,
                scope_id,
            });
        }
        // Projection collection: the accumulator is Some exclusively while a
        // SELECT projection expression is being visited (nested selects swap it
        // in visit_select), so subquery columns never leak into outer items.
        if let Some(item) = self.current_projection_item.as_mut() {
            item.columns.push(ProjectionColumnRef {
                name: ident.value.clone(),
                qualifier,
                span,
                aggregated: self.aggregate_depth > 0 || self.window_depth > 0,
            });
        }
    }
}

/// Split a GROUP BY expression into its `(qualifier, name)` parts when it is a
/// plain (possibly qualified) identifier — the only shape the conservative
/// membership check can match against.
fn plain_identifier_parts(expr: &Expr) -> Option<(Option<String>, String)> {
    match expr {
        Expr::Identifier(ident) => Some((None, ident.value.clone())),
        Expr::CompoundIdentifier(idents) => {
            let column = idents.last()?;
            let qualifier =
                if idents.len() >= 2 { idents.get(idents.len() - 2).map(|ident| ident.value.clone()) } else { None };
            Some((qualifier, column.value.clone()))
        }
        _ => None,
    }
}

fn is_aggregate_function_name(name: &str) -> bool {
    const AGGREGATE_FUNCTION_NAMES: &[&str] = &[
        "sum",
        "count",
        "count_big",
        "avg",
        "min",
        "max",
        "total",
        "group_concat",
        "string_agg",
        "array_agg",
        "listagg",
        "bit_and",
        "bit_or",
        "bit_xor",
        "bool_and",
        "bool_or",
        "every",
        "median",
        "stddev",
        "stddev_pop",
        "stddev_samp",
        "stdev",
        "stdevp",
        "variance",
        "var_pop",
        "var_samp",
        "varp",
        "json_agg",
        "jsonb_agg",
        "json_group_array",
        "xmlagg",
        "wm_concat",
        "approx_count_distinct",
        "checksum_agg",
    ];
    AGGREGATE_FUNCTION_NAMES.contains(&name)
}

fn table_reference_from_name(
    name: &ObjectName,
    alias: Option<String>,
    scope_id: Option<usize>,
) -> Option<SqlTableReference> {
    let parts: Vec<&Ident> = name.0.iter().filter_map(ObjectNamePart::as_ident).collect();
    let table = parts.last()?;
    let schema = if parts.len() >= 2 { parts.get(parts.len() - 2).map(|ident| ident.value.clone()) } else { None };

    Some(SqlTableReference { name: table.value.clone(), schema, alias, span: table.span.into(), scope_id: scope_id? })
}

fn object_name_last_ident(name: &ObjectName) -> Option<&Ident> {
    name.0.iter().rev().find_map(ObjectNamePart::as_ident)
}

fn sqlserver_datepart_function_name(name: &ObjectName) -> Option<&str> {
    if name.0.len() != 1 {
        return None;
    }

    let ident = name.0.first()?.as_ident()?;
    ["DATEADD", "DATEDIFF", "DATEDIFF_BIG", "DATEPART", "DATENAME"]
        .iter()
        .copied()
        .find(|function_name| ident.value.eq_ignore_ascii_case(function_name))
}
