use crate::models::connection::DatabaseType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialectKind {
    Mysql,

    Unsupported,
}

impl DialectKind {
    pub fn from_database_type(db_type: DatabaseType) -> Self {
        crate::database_manifest::dialect_name(&db_type).and_then(Self::from_label).unwrap_or(DialectKind::Unsupported)
    }

    pub fn to_database_type(self) -> Option<DatabaseType> {
        match self {
            DialectKind::Mysql => Some(DatabaseType::Mysql),

            DialectKind::Unsupported => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DialectKind::Mysql => "mysql",

            DialectKind::Unsupported => "unsupported",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        if label.eq_ignore_ascii_case("mysql") {
            Some(Self::Mysql)
        } else {
            None
        }
    }
}

type DdlCapabilityFlags = u64;

pub const CAP_ADD_COLUMN: DdlCapabilityFlags = 1 << 0;
pub const CAP_DROP_COLUMN: DdlCapabilityFlags = 1 << 1;
pub const CAP_RENAME_COLUMN: DdlCapabilityFlags = 1 << 2;
pub const CAP_ALTER_EXISTING_COLUMN: DdlCapabilityFlags = 1 << 3;
pub const CAP_REORDER_COLUMN: DdlCapabilityFlags = 1 << 4;
pub const CAP_COMMENT: DdlCapabilityFlags = 1 << 5;
pub const CAP_CREATE_INDEX: DdlCapabilityFlags = 1 << 6;
pub const CAP_DROP_INDEX: DdlCapabilityFlags = 1 << 7;
pub const CAP_REBUILD_INDEX: DdlCapabilityFlags = 1 << 8;
pub const CAP_INDEX_TYPE: DdlCapabilityFlags = 1 << 9;
pub const CAP_INDEX_INCLUDE: DdlCapabilityFlags = 1 << 10;
pub const CAP_INDEX_FILTER: DdlCapabilityFlags = 1 << 11;
pub const CAP_INDEX_COMMENT: DdlCapabilityFlags = 1 << 12;
pub const CAP_ALTER_PRIMARY_KEY: DdlCapabilityFlags = 1 << 13;
pub const CAP_FOREIGN_KEY: DdlCapabilityFlags = 1 << 14;
pub const CAP_CREATE_TABLE: DdlCapabilityFlags = 1 << 15;
pub const CAP_DROP_TABLE: DdlCapabilityFlags = 1 << 16;
pub const CAP_TRUNCATE_TABLE: DdlCapabilityFlags = 1 << 17;
pub const CAP_CREATE_TRIGGER: DdlCapabilityFlags = 1 << 18;
pub const CAP_DROP_TRIGGER: DdlCapabilityFlags = 1 << 19;
pub const CAP_CREATE_FUNCTION: DdlCapabilityFlags = 1 << 20;
pub const CAP_DROP_FUNCTION: DdlCapabilityFlags = 1 << 21;
pub const CAP_CREATE_SEQUENCE: DdlCapabilityFlags = 1 << 22;
pub const CAP_DROP_SEQUENCE: DdlCapabilityFlags = 1 << 23;
pub const CAP_ALTER_OWNER: DdlCapabilityFlags = 1 << 24;
pub const CAP_GRANT_REVOKE: DdlCapabilityFlags = 1 << 25;
pub const CAP_IF_NOT_EXISTS: DdlCapabilityFlags = 1 << 26;
pub const CAP_CREATE_OR_REPLACE: DdlCapabilityFlags = 1 << 27;
pub const CAP_TEMPORARY_TABLE: DdlCapabilityFlags = 1 << 28;
pub const CAP_TRANSACTIONAL_DDL: DdlCapabilityFlags = 1 << 29;
pub const CAP_AUTO_INCREMENT: DdlCapabilityFlags = 1 << 30;
pub const CAP_IDENTITY_COLUMNS: DdlCapabilityFlags = 1 << 31;

const CAP_NAMES: &[(DdlCapabilityFlags, &str)] = &[
    (CAP_ADD_COLUMN, "add_column"),
    (CAP_DROP_COLUMN, "drop_column"),
    (CAP_RENAME_COLUMN, "rename_column"),
    (CAP_ALTER_EXISTING_COLUMN, "alter_existing_column"),
    (CAP_REORDER_COLUMN, "reorder_column"),
    (CAP_COMMENT, "comment"),
    (CAP_CREATE_INDEX, "create_index"),
    (CAP_DROP_INDEX, "drop_index"),
    (CAP_REBUILD_INDEX, "rebuild_index"),
    (CAP_INDEX_TYPE, "index_type"),
    (CAP_INDEX_INCLUDE, "index_include"),
    (CAP_INDEX_FILTER, "index_filter"),
    (CAP_INDEX_COMMENT, "index_comment"),
    (CAP_ALTER_PRIMARY_KEY, "alter_primary_key"),
    (CAP_FOREIGN_KEY, "foreign_key"),
    (CAP_CREATE_TABLE, "create_table"),
    (CAP_DROP_TABLE, "drop_table"),
    (CAP_TRUNCATE_TABLE, "truncate_table"),
    (CAP_CREATE_TRIGGER, "create_trigger"),
    (CAP_DROP_TRIGGER, "drop_trigger"),
    (CAP_CREATE_FUNCTION, "create_function"),
    (CAP_DROP_FUNCTION, "drop_function"),
    (CAP_CREATE_SEQUENCE, "create_sequence"),
    (CAP_DROP_SEQUENCE, "drop_sequence"),
    (CAP_ALTER_OWNER, "alter_owner"),
    (CAP_GRANT_REVOKE, "grant_revoke"),
    (CAP_IF_NOT_EXISTS, "if_not_exists"),
    (CAP_CREATE_OR_REPLACE, "create_or_replace"),
    (CAP_TEMPORARY_TABLE, "temporary_table"),
    (CAP_TRANSACTIONAL_DDL, "transactional_ddl"),
    (CAP_AUTO_INCREMENT, "auto_increment"),
    (CAP_IDENTITY_COLUMNS, "identity_columns"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DialectCapabilityDescriptor {
    pub dialect: DialectKind,
    pub flags: DdlCapabilityFlags,
    pub max_identifier_length: u32,
    pub supports_schemas: bool,
    pub supports_catalogs: bool,
    pub max_columns_per_table: u32,
    pub max_indexes_per_table: u32,
    pub max_query_size_bytes: u64,
    pub supports_full_text_index: bool,
    pub supports_spatial_index: bool,
    pub supports_partitioning: bool,
    pub supports_table_sampling: bool,
    pub max_foreign_key_name_length: u32,
    pub supports_on_update_cascade: bool,
    pub supports_on_delete_set_null: bool,
    pub supports_deferrable_constraints: bool,
    pub supports_array_type: bool,
    pub supports_json_type: bool,
    pub supports_enum_type: bool,
    pub supports_uuid_type: bool,
    pub supports_identity_columns: bool,
    pub supports_auto_increment: bool,
    pub supports_sequences: bool,
}

impl Default for DialectCapabilityDescriptor {
    fn default() -> Self {
        Self {
            dialect: DialectKind::Unsupported,
            flags: 0,
            max_identifier_length: 0,
            supports_schemas: false,
            supports_catalogs: false,
            max_columns_per_table: 0,
            max_indexes_per_table: 0,
            max_query_size_bytes: 0,
            supports_full_text_index: false,
            supports_spatial_index: false,
            supports_partitioning: false,
            supports_table_sampling: false,
            max_foreign_key_name_length: 0,
            supports_on_update_cascade: false,
            supports_on_delete_set_null: false,
            supports_deferrable_constraints: false,
            supports_array_type: false,
            supports_json_type: false,
            supports_enum_type: false,
            supports_uuid_type: false,
            supports_identity_columns: false,
            supports_auto_increment: false,
            supports_sequences: false,
        }
    }
}

impl DialectCapabilityDescriptor {
    pub fn has_capability(&self, flag: DdlCapabilityFlags) -> bool {
        self.flags & flag != 0
    }

    pub fn for_dialect(kind: DialectKind) -> Self {
        match kind {
            DialectKind::Mysql => Self {
                dialect: DialectKind::Mysql,
                flags: CAP_ADD_COLUMN
                    | CAP_DROP_COLUMN
                    | CAP_RENAME_COLUMN
                    | CAP_ALTER_EXISTING_COLUMN
                    | CAP_REORDER_COLUMN
                    | CAP_COMMENT
                    | CAP_CREATE_INDEX
                    | CAP_DROP_INDEX
                    | CAP_REBUILD_INDEX
                    | CAP_INDEX_TYPE
                    | CAP_INDEX_COMMENT
                    | CAP_ALTER_PRIMARY_KEY
                    | CAP_FOREIGN_KEY
                    | CAP_CREATE_TABLE
                    | CAP_DROP_TABLE
                    | CAP_TRUNCATE_TABLE
                    | CAP_CREATE_TRIGGER
                    | CAP_DROP_TRIGGER
                    | CAP_CREATE_FUNCTION
                    | CAP_DROP_FUNCTION
                    | CAP_CREATE_SEQUENCE
                    | CAP_DROP_SEQUENCE
                    | CAP_ALTER_OWNER
                    | CAP_GRANT_REVOKE
                    | CAP_IF_NOT_EXISTS
                    | CAP_TEMPORARY_TABLE
                    | CAP_AUTO_INCREMENT,
                max_identifier_length: 64,
                max_columns_per_table: 4096,
                max_indexes_per_table: 64,
                max_query_size_bytes: 16 * 1024 * 1024,
                max_foreign_key_name_length: 64,
                supports_full_text_index: true,
                supports_spatial_index: true,
                supports_partitioning: true,
                supports_on_update_cascade: true,
                supports_on_delete_set_null: true,
                supports_json_type: true,
                supports_enum_type: true,
                supports_auto_increment: true,
                ..Default::default()
            },

            DialectKind::Unsupported => Self::default(),
        }
    }

    pub fn capabilities_for_database_type(db_type: DatabaseType) -> Self {
        crate::sql_dialect::resolve_for_db(db_type)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TypeConversionRule {
    pub source_type: String,
    pub target_type: String,
    pub precision_loss: bool,
    pub requires_cast: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeMappingMatrix {
    pub from: DialectKind,
    pub to: DialectKind,
    pub rules: Vec<TypeConversionRule>,
}

impl TypeMappingMatrix {
    pub fn for_dialects(from: DialectKind, to: DialectKind) -> Self {
        let rules = Self::build_rules(from, to);
        Self { from, to, rules }
    }

    fn build_rules(from: DialectKind, to: DialectKind) -> Vec<TypeConversionRule> {
        let mut rules = Vec::new();
        match (from, to) {
            _ => {}
        }
        rules
    }

    pub fn convert_type(&self, source_type: &str) -> (String, bool) {
        let trimmed = source_type.trim().to_ascii_uppercase();

        for rule in &self.rules {
            if trimmed == rule.source_type {
                return (rule.target_type.clone(), rule.requires_cast);
            }
            if trimmed.starts_with(&rule.source_type) {
                let remaining = trimmed.trim_start_matches(&rule.source_type);
                if remaining.is_empty() || remaining.starts_with(' ') || remaining.starts_with('(') {
                    return (rule.target_type.clone(), rule.requires_cast);
                }
            }
        }

        (source_type.to_string(), true)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DialectInfo {
    pub kind: DialectKind,
    pub label: String,
    pub capabilities: Vec<String>,
    pub max_identifier_length: u32,
    pub supports_schemas: bool,
    pub supports_catalogs: bool,
    pub max_columns_per_table: u32,
    pub supports_json_type: bool,
    pub supports_array_type: bool,
    pub supports_enum_type: bool,
    pub supports_uuid_type: bool,
    pub supports_auto_increment: bool,
    pub supports_identity_columns: bool,
    pub supports_sequences: bool,
    pub supports_partitioning: bool,
    pub supports_full_text_index: bool,
    pub supports_spatial_index: bool,
    pub supports_transactional_ddl: bool,
}

impl From<DialectCapabilityDescriptor> for DialectInfo {
    fn from(caps: DialectCapabilityDescriptor) -> Self {
        Self {
            kind: caps.dialect,
            label: caps.dialect.label().to_string(),
            capabilities: CAP_NAMES
                .iter()
                .filter(|(flag, _)| caps.has_capability(*flag))
                .map(|(_, name)| name.to_string())
                .collect(),
            max_identifier_length: caps.max_identifier_length,
            supports_schemas: caps.supports_schemas,
            supports_catalogs: caps.supports_catalogs,
            max_columns_per_table: caps.max_columns_per_table,
            supports_json_type: caps.supports_json_type,
            supports_array_type: caps.supports_array_type,
            supports_enum_type: caps.supports_enum_type,
            supports_uuid_type: caps.supports_uuid_type,
            supports_auto_increment: caps.supports_auto_increment,
            supports_identity_columns: caps.supports_identity_columns,
            supports_sequences: caps.supports_sequences,
            supports_partitioning: caps.supports_partitioning,
            supports_full_text_index: caps.supports_full_text_index,
            supports_spatial_index: caps.supports_spatial_index,
            supports_transactional_ddl: caps.has_capability(CAP_TRANSACTIONAL_DDL),
        }
    }
}

impl DialectInfo {
    pub fn for_kind(kind: DialectKind) -> Self {
        let caps = crate::sql_dialect::resolve(kind);
        Self::from(caps)
    }

    pub fn all() -> Vec<Self> {
        vec![Self::for_kind(DialectKind::Mysql)]
    }
}

pub fn dialect_check(kind: DialectKind) -> DialectInfo {
    DialectInfo::for_kind(kind)
}

pub fn dialect_check_all() -> Vec<DialectInfo> {
    DialectInfo::all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_capabilities_match_capabilities_for() {
        let desc = DialectCapabilityDescriptor::for_dialect(DialectKind::Mysql);
        assert!(desc.has_capability(CAP_ADD_COLUMN));
        assert!(desc.has_capability(CAP_DROP_COLUMN));
        assert!(desc.has_capability(CAP_RENAME_COLUMN));
        assert!(desc.has_capability(CAP_FOREIGN_KEY));
        assert!(desc.supports_auto_increment);
        assert!(desc.supports_json_type);
        assert!(desc.supports_enum_type);
        assert!(!desc.supports_schemas);
        assert!(!desc.supports_identity_columns);
    }

    #[test]
    fn unsupported_capabilities_are_all_false() {
        let desc = DialectCapabilityDescriptor::for_dialect(DialectKind::Unsupported);
        assert_eq!(desc.flags, 0);
        assert!(!desc.supports_schemas);
        assert!(!desc.supports_partitioning);
    }
}
