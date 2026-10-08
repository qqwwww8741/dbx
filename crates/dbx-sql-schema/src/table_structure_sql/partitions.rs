use super::create_table::build_create_table_sql_with_partition_clause;
use super::dialect::{capabilities_for, StructureDialect};
use super::types::{
    TablePartitionBoundDraft, TablePartitionDefinition, TablePartitionOperation, TablePartitionOperationKind,
    TablePartitionSqlOptions, TableStructureSqlOptions, TableStructureSqlResult,
};
use super::util::quote_ident;
use crate::models::connection::DatabaseType;
use crate::types::PgPartitionKind;
pub fn build_create_partitioned_table_sql(
    options: TableStructureSqlOptions,
    definition: TablePartitionDefinition,
) -> TableStructureSqlResult {
    let dialect = capabilities_for(options.database_type, options.driver_profile.as_deref()).dialect;
    {
        return TableStructureSqlResult {
            statements: Vec::new(),
            warnings: vec!["Partitioning is not supported for this database engine.".to_string()],
        };
    }
}
pub fn build_table_partition_operation_sql(options: TablePartitionSqlOptions) -> TableStructureSqlResult {
    if options.operations.is_empty() {
        return TableStructureSqlResult { statements: Vec::new(), warnings: Vec::new() };
    }
    let dialect = capabilities_for(options.database_type, options.driver_profile.as_deref()).dialect;
    {
        return TableStructureSqlResult {
            statements: Vec::new(),
            warnings: vec!["Partition operations are not supported for this database engine.".to_string()],
        };
    }
}
