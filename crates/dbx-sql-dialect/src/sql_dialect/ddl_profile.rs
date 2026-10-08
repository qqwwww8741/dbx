//! Target-database DDL profile: how to *write* SQL for a concrete [`DatabaseType`].
//!
//! Profiles are data + small enums. Call sites must not branch on individual databases;
//! they only consult profile fields (quote style, auto-increment form, type map, …).

use crate::models::connection::DatabaseType;

/// Identifier quoting style for generated DDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    /// MySQL-family: `name`
    Backtick,
    /// PostgreSQL / Access / most ANSI: "name"
    DoubleQuote,
    /// SQL Server: [name]
    Brackets,
    /// Oracle-style unquoted uppercase
    UnquotedUpper,
}

/// How auto-increment / identity is expressed on the target database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoIncSyntax {
    /// No special auto-increment DDL.
    None,
    /// Append a fixed suffix to the column definition (e.g. ` AUTO_INCREMENT`, ` IDENTITY(1,1)`).
    Suffix(&'static str),
    /// Replace the mapped type with this type name for auto PK columns (e.g. Access `COUNTER`).
    ReplaceTypeWith(&'static str),
    /// PostgreSQL-style sequence + DEFAULT nextval (handled by generator after CREATE).
    PostgresSequence,
}

/// Static type rewrite rule: source base type (uppercase, no params) → target template.
/// Target may use `{}` for a single length/precision placeholder (first param only).
#[derive(Debug, Clone, Copy)]
pub struct TypeMapEntry {
    pub source_base: &'static str,
    pub target_template: &'static str,
}

/// How CREATE INDEX places the index method / type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexTypePlacement {
    /// No index-type clause.
    None,
    /// PostgreSQL: `CREATE INDEX … ON t USING btree (…)`
    UsingSuffix,
    /// SQL Server: `CREATE CLUSTERED INDEX …`
    TypePrefix,
    /// MySQL: `CREATE INDEX … USING BTREE ON t (…)`
    UsingBeforeOn,
}

/// CREATE TRIGGER body shape (templates use `{name}` `{timing}` `{event}` `{table}` `{body}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerTemplate {
    /// `CREATE TRIGGER {name} {timing} {event} ON {table} FOR EACH ROW BEGIN {body} END;`
    MysqlStyle,
    /// `CREATE TRIGGER {name} {timing} {event} ON {table} FOR EACH ROW EXECUTE FUNCTION {body};`
    PostgresStyle,
    /// `CREATE TRIGGER {name} ON {table} {timing} {event} AS BEGIN {body} END;`
    SqlServerStyle,
    /// Conservative MySQL-like default for unknown engines.
    GenericRowBody,
}

/// RENAME COLUMN strategy inside ALTER TABLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameColumnSyntax {
    /// `CHANGE COLUMN old new_def` (MySQL)
    MysqlChangeColumn,
    /// `RENAME COLUMN old TO new`
    RenameColumn,
}

/// Column DDL that cannot use the generic `ADD COLUMN` / `ALTER COLUMN` forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnDdlSyntax {
    Generic,
}

/// Standalone table and column comment representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentDdlSyntax {
    Generic,
}

/// DDL generation profile for one [`DatabaseType`].
#[derive(Debug, Clone, Copy)]
pub struct DdlDialectProfile {
    pub database_type: DatabaseType,
    pub quote: QuoteStyle,
    pub auto_inc: AutoIncSyntax,
    /// When false, display widths like `INT(11)` are stripped if no type_map hit.
    pub supports_display_width: bool,
    /// Whether this target's VARCHAR/CHARACTER VARYING/NVARCHAR syntax rejects a
    /// missing length (e.g. `VARCHAR` with no length is a MySQL syntax error).
    /// Distinct from `supports_display_width`: that flag is about whether the
    /// target *prints* MySQL-style `INT(11)` widths, not whether VARCHAR needs one.
    pub requires_explicit_varchar_length: bool,
    /// Cap for length params when applying `{}` templates (e.g. Access TEXT max 255).
    pub max_varchar_len: Option<u32>,
    /// MySQL-style inline `COMMENT '...'` on columns.
    pub inline_column_comment: bool,
    /// SQLite-style: emit FOREIGN KEY clauses inside CREATE TABLE.
    pub foreign_keys_inline_in_create: bool,
    /// `DROP INDEX name ON table` (MySQL) vs `DROP INDEX IF EXISTS name`.
    pub drop_index_uses_on_table: bool,
    /// `DROP FOREIGN KEY` (MySQL) vs `DROP CONSTRAINT`.
    pub drop_fk_as_foreign_key: bool,
    /// `DROP TABLE t CASCADE` is accepted. Oracle's `CASCADE CONSTRAINTS` is a
    /// different clause and does not count.
    pub drop_table_supports_cascade: bool,
    /// `DROP TABLE IF EXISTS t` is valid grammar. False for Oracle (pre-23c),
    /// Access, Firebird, Db2, Informix, SAP HANA and Teradata.
    pub drop_table_supports_if_exists: bool,
    /// Index method placement.
    pub index_type_placement: IndexTypePlacement,
    /// `INCLUDE (cols)` on indexes.
    pub index_supports_include: bool,
    /// Partial index `WHERE …`.
    pub index_supports_filter: bool,
    /// Index `COMMENT '…'` (MySQL).
    pub index_supports_comment: bool,
    /// Table comment: `ALTER TABLE t COMMENT = '…'` vs `COMMENT ON TABLE`.
    pub table_comment_via_alter: bool,
    /// Standalone column comment SQL is unsupported (MySQL needs MODIFY COLUMN).
    pub column_comment_via_modify_only: bool,
    pub comment_ddl: CommentDdlSyntax,
    pub trigger_template: TriggerTemplate,
    pub rename_column: RenameColumnSyntax,
    pub column_ddl: ColumnDdlSyntax,
    /// MySQL `MODIFY COLUMN` vs ANSI `ALTER COLUMN … TYPE/SET`.
    pub alter_uses_modify_column: bool,
    /// Spell `ADD COLUMN` (MySQL/Postgres/...) instead of the Oracle-family bare `ADD (...)`.
    pub add_column_uses_column_keyword: bool,
    /// Spell `MODIFY COLUMN` (MySQL) instead of the bare `MODIFY (...)` Oracle-family and
    /// Dameng both use.
    pub modify_column_uses_column_keyword: bool,
    /// Wrap the definition in parentheses: Oracle's `ADD (...)` / `MODIFY (...)` clauses.
    pub parenthesized_alter_column_clause: bool,
    /// Write `DEFAULT` before `NOT NULL`. Oracle's grammar is
    /// `datatype [DEFAULT expr] [NOT NULL]` and it rejects the MySQL order outright
    /// (ORA-00907), so the tail of a column definition is dialect data, not cosmetics.
    pub column_default_precedes_not_null: bool,
    /// Batch multiple alter clauses in one `ALTER TABLE` statement.
    pub alter_batches_clauses: bool,
    /// Prefer emitting source SHOW CREATE / native DDL when dialects match (MySQL-family).
    pub prefers_native_source_ddl: bool,
    /// GRANT/REVOKE identifier style: backtick + user quotes vs ANSI double-quote.
    pub grant_uses_mysql_user_syntax: bool,
    /// Emit a comment that FK changes may need table rebuild (SQLite-family).
    pub warn_fk_needs_table_rebuild: bool,
    pub create_table_if_not_exists: bool,
    pub create_index_if_not_exists: bool,
    pub create_function_or_replace: bool,
    pub supports_function_ddl: bool,
    pub supports_sequence_ddl: bool,
    pub supports_rule_ddl: bool,
    pub supports_owner_ddl: bool,
    /// `{create_kw} {name} {definition};` — `create_kw` is CREATE [OR REPLACE] FUNCTION.
    pub function_create_template: Option<&'static str>,
    /// `DROP FUNCTION IF EXISTS {name}{cascade};`
    pub function_drop_template: Option<&'static str>,
    /// `CREATE SEQUENCE {name} AS {data_type} START WITH {start_value} … {cycle};`
    pub sequence_create_template: Option<&'static str>,
    /// `ALTER SEQUENCE {name} AS {data_type} START WITH {start_value} … {cycle};`
    pub sequence_alter_template: Option<&'static str>,
    /// `DROP SEQUENCE {name}{cascade};`
    pub sequence_drop_template: Option<&'static str>,
    /// `DROP RULE IF EXISTS {rule_name} ON {table_name};`
    pub rule_drop_template: Option<&'static str>,
    /// `ALTER {object_type} {name} OWNER TO {owner};`
    pub owner_alter_template: Option<&'static str>,
    /// `DROP TABLE {table}{cascade};` — the caller quotes `{table}` itself. Engines
    /// that drop a differently-named object override the whole shape.
    pub drop_table_template: &'static str,
    /// Optional session lock-timeout preamble for generated scripts.
    pub lock_timeout_sql: Option<&'static str>,
    /// Data-driven base-type rewrites for this target (empty → rely on matrix / normalize only).
    pub type_map: &'static [TypeMapEntry],
}

impl DdlDialectProfile {
    pub fn lookup_type(&self, source_base_upper: &str) -> Option<&'static str> {
        self.type_map.iter().find(|e| e.source_base.eq_ignore_ascii_case(source_base_upper)).map(|e| e.target_template)
    }

    pub fn quote_ident(&self, name: &str) -> String {
        match self.quote {
            QuoteStyle::Backtick => format!("`{}`", name.replace('`', "``")),
            QuoteStyle::DoubleQuote => format!("\"{}\"", name.replace('"', "\"\"")),
            QuoteStyle::Brackets => format!("[{}]", name.replace(']', "]]")),
            // Oracle: unquoted identifiers fold to uppercase. Mixed-case names and
            // special characters must be double-quoted with original spelling preserved.
            QuoteStyle::UnquotedUpper => {
                let has_lower = name.chars().any(|c| c.is_ascii_lowercase());
                let has_upper = name.chars().any(|c| c.is_ascii_uppercase());
                let has_special = name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '$' && c != '#');
                let bad_start =
                    name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                // All-lower "emp" stays unquoted → EMP (Oracle fold). Mixed "empMixed" → "empMixed".
                let needs_quotes = (has_lower && has_upper) || has_special || bad_start;
                if needs_quotes {
                    format!("\"{}\"", name.replace('"', "\"\""))
                } else {
                    name.to_uppercase()
                }
            }
        }
    }

    pub fn alter_modify_keyword(&self) -> &'static str {
        if self.modify_column_uses_column_keyword {
            "MODIFY COLUMN"
        } else {
            "MODIFY"
        }
    }

    /// Clause that changes one column's type without restating the whole definition.
    ///
    /// `ALTER COLUMN x TYPE …` everywhere except the Oracle grammar, where `TYPE` does not
    /// exist and the change has to be spelled `MODIFY (x TYPE)`.
    pub fn alter_column_type_clause(&self, name: &str, data_type: &str) -> String {
        if self.parenthesized_alter_column_clause {
            format!("  {} ({name} {data_type})", self.alter_modify_keyword())
        } else {
            format!("  ALTER COLUMN {name} TYPE {data_type}")
        }
    }

    /// Clause that flips one column's nullability, in the dialect's spelling.
    pub fn alter_column_nullability_clause(&self, name: &str, nullable: bool) -> String {
        if self.parenthesized_alter_column_clause {
            format!("  {} ({name} {})", self.alter_modify_keyword(), if nullable { "NULL" } else { "NOT NULL" })
        } else if nullable {
            format!("  ALTER COLUMN {name} DROP NOT NULL")
        } else {
            format!("  ALTER COLUMN {name} SET NOT NULL")
        }
    }

    /// Replace `{key}` placeholders. Unknown keys are left unchanged.
    pub fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
        let mut out = template.to_string();
        for (key, value) in vars {
            out = out.replace(&format!("{{{key}}}"), value);
        }
        out
    }
}

const FN_CREATE: &str = "{create_kw} {name} {definition};";
const FN_DROP: &str = "DROP FUNCTION IF EXISTS {name}{cascade};";
const SEQ_CREATE: &str =
    "CREATE SEQUENCE {name} AS {data_type} START WITH {start_value} INCREMENT BY {increment} MINVALUE {min_value} MAXVALUE {max_value} {cycle};";
const SEQ_ALTER: &str =
    "ALTER SEQUENCE {name} AS {data_type} START WITH {start_value} INCREMENT BY {increment} MINVALUE {min_value} MAXVALUE {max_value} {cycle};";
const SEQ_DROP: &str = "DROP SEQUENCE {name}{cascade};";

const RULE_DROP: &str = "DROP RULE IF EXISTS {rule_name} ON {table_name}{cascade};";
const OWNER_ALTER: &str = "ALTER {object_type} {name} OWNER TO {owner};";
/// Shared ANSI `DROP TABLE` shape. `{if_exists}` is deliberately absent: whether a
/// dialect accepts `IF EXISTS` is a capability flag, not part of the rendered shape.
const DROP_TABLE: &str = "DROP TABLE {table}{cascade};";

// ---------------------------------------------------------------------------
// Type maps (data only — no per-database logic at call sites)
// ---------------------------------------------------------------------------

const ACCESS_TYPE_MAP: &[TypeMapEntry] = &[
    TypeMapEntry { source_base: "BOOL", target_template: "YESNO" },
    TypeMapEntry { source_base: "BOOLEAN", target_template: "YESNO" },
    TypeMapEntry { source_base: "BIT", target_template: "YESNO" },
    TypeMapEntry { source_base: "YESNO", target_template: "YESNO" },
    TypeMapEntry { source_base: "TINYINT", target_template: "BYTE" },
    TypeMapEntry { source_base: "BYTE", target_template: "BYTE" },
    TypeMapEntry { source_base: "SMALLINT", target_template: "SMALLINT" },
    TypeMapEntry { source_base: "SHORT", target_template: "SMALLINT" },
    TypeMapEntry { source_base: "MEDIUMINT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "INT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "INTEGER", target_template: "INTEGER" },
    TypeMapEntry { source_base: "INT4", target_template: "INTEGER" },
    TypeMapEntry { source_base: "LONG", target_template: "INTEGER" },
    TypeMapEntry { source_base: "BIGINT", target_template: "DECIMAL(20,0)" },
    TypeMapEntry { source_base: "INT8", target_template: "DECIMAL(20,0)" },
    TypeMapEntry { source_base: "FLOAT", target_template: "SINGLE" },
    TypeMapEntry { source_base: "REAL", target_template: "SINGLE" },
    TypeMapEntry { source_base: "SINGLE", target_template: "SINGLE" },
    TypeMapEntry { source_base: "DOUBLE", target_template: "DOUBLE" },
    TypeMapEntry { source_base: "DOUBLE PRECISION", target_template: "DOUBLE" },
    TypeMapEntry { source_base: "DECIMAL", target_template: "DECIMAL({})" },
    TypeMapEntry { source_base: "NUMERIC", target_template: "DECIMAL({})" },
    TypeMapEntry { source_base: "NUMBER", target_template: "DECIMAL({})" },
    TypeMapEntry { source_base: "CURRENCY", target_template: "CURRENCY" },
    TypeMapEntry { source_base: "VARCHAR", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "CHARACTER VARYING", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "CHAR", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "CHARACTER", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "NVARCHAR", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "NVARCHAR2", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "VARCHAR2", target_template: "TEXT({})" },
    TypeMapEntry { source_base: "TEXT", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "TINYTEXT", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "MEDIUMTEXT", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "LONGTEXT", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "CLOB", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "MEMO", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "DATE", target_template: "DATETIME" },
    TypeMapEntry { source_base: "DATETIME", target_template: "DATETIME" },
    TypeMapEntry { source_base: "TIMESTAMP", target_template: "DATETIME" },
    TypeMapEntry { source_base: "TIME", target_template: "DATETIME" },
    TypeMapEntry { source_base: "YEAR", target_template: "SMALLINT" },
    TypeMapEntry { source_base: "BLOB", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "TINYBLOB", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "MEDIUMBLOB", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "LONGBLOB", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "BINARY", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "VARBINARY", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "IMAGE", target_template: "OLEOBJECT" },
    TypeMapEntry { source_base: "JSON", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "JSONB", target_template: "LONGTEXT" },
    TypeMapEntry { source_base: "UUID", target_template: "GUID" },
    TypeMapEntry { source_base: "UNIQUEIDENTIFIER", target_template: "GUID" },
    TypeMapEntry { source_base: "GUID", target_template: "GUID" },
];

const SQLITE_TYPE_MAP: &[TypeMapEntry] = &[
    TypeMapEntry { source_base: "TINYINT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "SMALLINT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "MEDIUMINT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "INT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "INTEGER", target_template: "INTEGER" },
    TypeMapEntry { source_base: "BIGINT", target_template: "INTEGER" },
    TypeMapEntry { source_base: "FLOAT", target_template: "REAL" },
    TypeMapEntry { source_base: "DOUBLE", target_template: "REAL" },
    TypeMapEntry { source_base: "DOUBLE PRECISION", target_template: "REAL" },
    TypeMapEntry { source_base: "DECIMAL", target_template: "NUMERIC" },
    TypeMapEntry { source_base: "NUMERIC", target_template: "NUMERIC" },
    TypeMapEntry { source_base: "VARCHAR", target_template: "TEXT" },
    TypeMapEntry { source_base: "CHAR", target_template: "TEXT" },
    TypeMapEntry { source_base: "TEXT", target_template: "TEXT" },
    TypeMapEntry { source_base: "TINYTEXT", target_template: "TEXT" },
    TypeMapEntry { source_base: "MEDIUMTEXT", target_template: "TEXT" },
    TypeMapEntry { source_base: "LONGTEXT", target_template: "TEXT" },
    TypeMapEntry { source_base: "DATETIME", target_template: "TEXT" },
    TypeMapEntry { source_base: "TIMESTAMP", target_template: "TEXT" },
    TypeMapEntry { source_base: "DATE", target_template: "TEXT" },
    TypeMapEntry { source_base: "BLOB", target_template: "BLOB" },
    TypeMapEntry { source_base: "JSON", target_template: "TEXT" },
];

// ---------------------------------------------------------------------------
// Profile families (shared shapes; registration is the only DatabaseType match)
// ---------------------------------------------------------------------------

fn mysql_family(db: DatabaseType) -> DdlDialectProfile {
    DdlDialectProfile {
        database_type: db,
        quote: QuoteStyle::Backtick,
        auto_inc: AutoIncSyntax::Suffix(" AUTO_INCREMENT"),
        supports_display_width: true,
        requires_explicit_varchar_length: true,
        max_varchar_len: Some(65_535),
        inline_column_comment: true,
        foreign_keys_inline_in_create: false,
        drop_index_uses_on_table: true,
        drop_fk_as_foreign_key: true,
        drop_table_supports_cascade: false,
        drop_table_supports_if_exists: true,
        index_type_placement: IndexTypePlacement::UsingBeforeOn,
        index_supports_include: false,
        index_supports_filter: false,
        index_supports_comment: true,
        table_comment_via_alter: true,
        column_comment_via_modify_only: true,
        comment_ddl: CommentDdlSyntax::Generic,
        trigger_template: TriggerTemplate::MysqlStyle,
        rename_column: RenameColumnSyntax::MysqlChangeColumn,
        column_ddl: ColumnDdlSyntax::Generic,
        prefers_native_source_ddl: true,
        grant_uses_mysql_user_syntax: true,
        warn_fk_needs_table_rebuild: false,
        alter_uses_modify_column: true,
        add_column_uses_column_keyword: true,
        modify_column_uses_column_keyword: true,
        parenthesized_alter_column_clause: false,
        column_default_precedes_not_null: false,
        alter_batches_clauses: true,
        create_table_if_not_exists: true,
        create_index_if_not_exists: false,
        create_function_or_replace: false,
        supports_function_ddl: false,
        supports_sequence_ddl: false,
        supports_rule_ddl: false,
        supports_owner_ddl: false,
        function_create_template: None,
        function_drop_template: None,
        sequence_create_template: None,
        sequence_alter_template: None,
        sequence_drop_template: None,
        rule_drop_template: None,
        owner_alter_template: None,
        drop_table_template: DROP_TABLE,
        lock_timeout_sql: Some("SET SESSION lock_wait_timeout = 3;"),
        type_map: &[],
    }
}

/// Resolve DDL profile for a concrete target database type.
///
/// This is the **only** place that maps [`DatabaseType`] → profile data.
/// Generators must not re-match on individual databases afterward.
pub fn profile_for(db_type: DatabaseType) -> DdlDialectProfile {
    mysql_family(db_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_object_templates_are_absent() {
        let p = profile_for(DatabaseType::Mysql);
        assert!(p.function_create_template.is_none());
        assert!(p.sequence_create_template.is_none());
        assert!(p.rule_drop_template.is_none());
        assert!(p.owner_alter_template.is_none());
    }

    #[test]
    fn render_template_replaces_placeholders() {
        let rendered = DdlDialectProfile::render_template(
            "DROP FUNCTION IF EXISTS {name}{cascade};",
            &[("name", "\"public\".\"f\""), ("cascade", " CASCADE")],
        );
        assert_eq!(rendered, "DROP FUNCTION IF EXISTS \"public\".\"f\" CASCADE;");
    }
}
