use crate::models::connection::DatabaseType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StructureDialect {
    Mysql,

    Unsupported,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TableStructureCapabilities {
    pub(super) dialect: StructureDialect,
    pub(super) add_column: bool,
    pub(super) drop_column: bool,
    pub(super) rename_column: bool,
    pub(super) alter_existing_column: bool,
    pub(super) reorder_column: bool,
    pub(super) comment: bool,
    pub(super) create_index: bool,
    pub(super) drop_index: bool,
    pub(super) rebuild_index: bool,
    pub(super) index_type: bool,
    pub(super) index_include: bool,
    pub(super) index_filter: bool,
    pub(super) index_comment: bool,
    pub(super) index_concurrent: bool,
    pub(super) add_primary_key: bool,
    pub(super) alter_primary_key: bool,
    pub(super) foreign_key: bool,
}

impl Default for TableStructureCapabilities {
    fn default() -> Self {
        Self {
            dialect: StructureDialect::Unsupported,
            add_column: false,
            drop_column: false,
            rename_column: false,
            alter_existing_column: false,
            reorder_column: false,
            comment: false,
            create_index: false,
            drop_index: false,
            rebuild_index: false,
            index_type: false,
            index_include: false,
            index_filter: false,
            index_comment: false,
            index_concurrent: false,
            add_primary_key: false,
            alter_primary_key: false,
            foreign_key: false,
        }
    }
}

pub(super) fn capabilities_for(
    database_type: Option<DatabaseType>,
    driver_profile: Option<&str>,
) -> TableStructureCapabilities {
    let base = TableStructureCapabilities::default();
    match database_type {
        Some(DatabaseType::Mysql) => TableStructureCapabilities {
            dialect: StructureDialect::Mysql,
            add_column: true,
            drop_column: true,
            rename_column: true,
            alter_existing_column: true,
            reorder_column: true,
            comment: true,
            create_index: true,
            drop_index: true,
            rebuild_index: true,
            index_type: true,
            index_comment: true,
            add_primary_key: true,
            alter_primary_key: true,
            foreign_key: true,
            ..base
        },

        _ => base,
    }
}

pub(super) fn database_label(database_type: Option<DatabaseType>) -> String {
    database_type
        .map(|database_type| {
            serde_json::to_value(database_type)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "this database".to_string())
        })
        .unwrap_or_else(|| "this database".to_string())
}

pub(super) fn dialect_label(dialect: StructureDialect) -> String {
    match dialect {
        StructureDialect::Mysql => "mysql",

        StructureDialect::Unsupported => "this database",
    }
    .to_string()
}

pub(super) fn database_type_for_dialect(dialect: StructureDialect) -> Option<DatabaseType> {
    match dialect {
        StructureDialect::Mysql => Some(DatabaseType::Mysql),

        StructureDialect::Unsupported => None,
    }
}
