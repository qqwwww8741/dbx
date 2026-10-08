use crate::models::connection::DatabaseType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablePaginationStrategy {
    LimitOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaginationContext {
    TablePreview,
    BoundedRead,
    UserQuery,
}

pub fn is_schema_aware(database_type: DatabaseType) -> bool {
    false
}

pub fn uses_fetch_first(database_type: DatabaseType) -> bool {
    false
}

/// Xugu exposes an unqualified ROWID pseudo-column for base, partitioned and
/// temporary tables. It is intentionally separate from Oracle's ROWIDTOCHAR
/// representation because qualified ROWID and ROWIDTOCHAR are not supported
/// by Xugu.
pub fn uses_xugu_row_id(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn uses_synthetic_row_id(database_type: Option<DatabaseType>) -> bool {
    false
}

/// Oracle 系方言不支持 `INSERT ... VALUES (...), (...)` 多行语法，
/// 复制为 INSERT 与导出 INSERT 都需按行生成单条语句。
pub fn uses_single_row_insert_statements(database_type: DatabaseType) -> bool {
    false
}

pub fn pagination_strategy(database_type: Option<DatabaseType>, context: PaginationContext) -> TablePaginationStrategy {
    match database_type {
        _ => TablePaginationStrategy::LimitOffset,
    }
}

pub fn table_pagination_strategy(database_type: Option<DatabaseType>) -> TablePaginationStrategy {
    pagination_strategy(database_type, PaginationContext::TablePreview)
}

pub(super) fn is_simple_informix_identifier(name: &str) -> bool {
    false
}

pub fn firebird_rows_clause(limit: usize, offset: usize) -> String {
    if offset > 0 {
        let start = offset + 1;
        let end = offset + limit;
        format!("ROWS {start} TO {end}")
    } else {
        format!("ROWS {limit}")
    }
}
