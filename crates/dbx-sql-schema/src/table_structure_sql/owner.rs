use super::dialect::StructureDialect;
use super::types::{TableOwnerChangeSqlOptions, TableStructureSqlResult};
use super::util::{qualified_table, quote_ident};
use crate::models::connection::DatabaseType;

pub fn build_table_owner_change_sql(options: TableOwnerChangeSqlOptions) -> TableStructureSqlResult {
    let owner = &options.owner;
    if owner == &options.original_owner {
        return TableStructureSqlResult { statements: Vec::new(), warnings: Vec::new() };
    }
    if owner.is_empty() {
        return TableStructureSqlResult {
            statements: Vec::new(),
            warnings: vec!["Table owner cannot be empty.".to_string()],
        };
    }
    {
        return TableStructureSqlResult {
            statements: Vec::new(),
            warnings: vec!["Changing the table owner is currently supported only for PostgreSQL.".to_string()],
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
