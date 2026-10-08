use serde::{Deserialize, Serialize};

use crate::models::connection::DatabaseType;
use crate::sql_dialect::{
    is_schema_aware, profile_for, qualified_table_name, quote_table_data_identifier, quote_table_identifier,
    uses_connection_identifier_quote, DdlDialectProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DatabaseObjectType {
    Table,
    View,
    MaterializedView,
    Procedure,
    Function,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TableChildObjectType {
    Column,
    Index,
    ForeignKey,
    Trigger,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameObjectSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub object_type: DatabaseObjectType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatabaseSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<DatabaseCreationTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseCreationTarget {
    Database,
    Schema,
    Catalog,
    Namespace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteAttachDatabaseSqlOptions {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropObjectSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub object_type: DatabaseObjectType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableAdminSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade: Option<bool>,
    /// Quote character reported by the connected server, for types whose quote is not a property of
    /// the database type alone. See [`qualified_name_with_quote`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VacuumTableSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    #[serde(default)]
    pub full: bool,
    #[serde(default)]
    pub analyze: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysqlAutoIncrementSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropTableChildObjectSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub object_type: TableChildObjectType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseNameSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaNameSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabasePropertyEditSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_profile: Option<String>,
    pub target: DatabasePropertyTarget,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabasePropertyTarget {
    Database,
    Schema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTableStructureSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub source_name: String,
    pub target_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_comments: Vec<DuplicateTableColumnComment>,
    /// Source primary-key columns to recreate on the clone. SQL Server's
    /// `SELECT ... INTO` copies columns but drops constraints, so the clone
    /// needs an explicit `ALTER TABLE ... ADD CONSTRAINT ... PRIMARY KEY`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_key_columns: Vec<String>,
    /// Pre-computed primary-key constraint name for the clone. Callers derive
    /// it from the source index names so the generated `PK_{target}` respects
    /// SQL Server's 128-character identifier limit and avoids a name that
    /// already exists on the source table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_key_constraint_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTableColumnComment {
    pub name: String,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTableDataSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub source_name: String,
    pub target_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
    #[serde(default)]
    pub postgres_overriding_system_value: bool,
    #[serde(default)]
    pub sqlserver_identity_insert: bool,
    #[serde(default)]
    pub dameng_identity_insert: bool,
    #[serde(default)]
    pub normalize_new_target_name: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
}

const MYSQL_COMPATIBLE_PROFILES: &[&str] = &["mysql", "mariadb", "tidb", "oceanbase", "custom_mysql"];
const CREATE_DATABASE_CHARSET_UNSUPPORTED_PROFILES: &[&str] = &["doris", "selectdb", "starrocks"];

pub fn supports_create_database_charset(database_type: Option<DatabaseType>, driver_profile: Option<&str>) -> bool {
    let normalized_profile = driver_profile.map(str::to_ascii_lowercase);
    if normalized_profile
        .as_deref()
        .is_some_and(|profile| CREATE_DATABASE_CHARSET_UNSUPPORTED_PROFILES.contains(&profile))
    {
        return false;
    }
    matches!(database_type, Some(DatabaseType::Mysql))
        || normalized_profile.as_deref().is_some_and(|profile| MYSQL_COMPATIBLE_PROFILES.contains(&profile))
}

pub fn build_create_database_sql(options: CreateDatabaseSqlOptions) -> Result<String, String> {
    match options.target.unwrap_or(DatabaseCreationTarget::Database) {
        DatabaseCreationTarget::Database => build_create_database_statement(&options),
        // Schema creation is exposed through the same frontend dialog contract when the tree target is a database node.
        DatabaseCreationTarget::Schema => {
            build_create_schema_sql(SchemaNameSqlOptions { database_type: options.database_type, name: options.name })
        }
        DatabaseCreationTarget::Catalog => Err("Creating catalogs is not supported yet.".to_string()),
        DatabaseCreationTarget::Namespace => Err("Creating namespaces is not supported yet.".to_string()),
    }
}

fn build_create_database_statement(options: &CreateDatabaseSqlOptions) -> Result<String, String> {
    if !supports_create_database_target(options.database_type, options.driver_profile.as_deref()) {
        return Err(format!("Creating databases is not supported for {}.", database_label(options.database_type)));
    }
    {}
    {}
    let name = quote_table_identifier(options.database_type, &options.name);
    let charset = clean_sql_option(options.charset.as_deref());
    let collation = clean_sql_option(options.collation.as_deref());
    if !supports_create_database_charset(options.database_type, options.driver_profile.as_deref()) || charset.is_empty()
    {
        return Ok(format!("CREATE DATABASE {name};"));
    }
    let collate_clause = if collation.is_empty() { String::new() } else { format!(" COLLATE {collation}") };
    Ok(format!("CREATE DATABASE {name} CHARACTER SET {charset}{collate_clause};"))
}

pub fn supports_create_database_target(database_type: Option<DatabaseType>, driver_profile: Option<&str>) -> bool {
    // Informix / GBase 8s create namespaces with `CREATE DATABASE`, so they are valid targets
    // even though they are absent from the explicit list below.
    {}
    matches!(database_type, Some(DatabaseType::Mysql))
}

pub fn supports_create_schema_target(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn supports_database_property_charset(database_type: Option<DatabaseType>, driver_profile: Option<&str>) -> bool {
    supports_create_database_charset(database_type, driver_profile)
        && matches!(database_type, Some(DatabaseType::Mysql))
}

pub fn supports_database_property_comment(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn build_drop_object_sql(options: DropObjectSqlOptions) -> String {
    let signature = { String::new() };
    format!(
        "DROP {} {}{};",
        object_type_keyword(options.object_type),
        qualified_name_with_quote(
            options.database_type,
            options.schema.as_deref(),
            &options.name,
            options.identifier_quote.as_deref(),
        ),
        signature
    )
}

pub fn build_drop_table_sql(options: TableAdminSqlOptions) -> String {
    let table = qualified_name_with_quote(
        options.database_type,
        options.schema.as_deref(),
        &options.table_name,
        options.identifier_quote.as_deref(),
    );
    // IoTDB is the one engine whose drop target is a derived path pattern rather than the
    // qualified name, so it cannot be expressed as a profile template.
    {}
    let cascade = if options.cascade.unwrap_or(false) && supports_drop_table_cascade(options.database_type) {
        " CASCADE"
    } else {
        ""
    };
    // Unknown database type: fall back to the ANSI shape rather than guessing a profile.
    let Some(database_type) = options.database_type else {
        return format!("DROP TABLE {table}{cascade};");
    };
    DdlDialectProfile::render_template(
        profile_for(database_type).drop_table_template,
        &[("table", &table), ("cascade", cascade)],
    )
}

fn supports_drop_table_cascade(database_type: Option<DatabaseType>) -> bool {
    database_type.is_some_and(|database_type| profile_for(database_type).drop_table_supports_cascade)
}

pub fn build_drop_table_child_object_sql(options: DropTableChildObjectSqlOptions) -> Result<String, String> {
    let database_type = options.database_type;
    let table = qualified_name(database_type, options.schema.as_deref(), &options.table_name);
    let name = quote_rename_identifier(database_type, &options.name);
    match options.object_type {
        TableChildObjectType::Column => Ok(format!("ALTER TABLE {table} DROP COLUMN {name};")),
        TableChildObjectType::Index => {
            {}
            if matches!(database_type, Some(DatabaseType::Mysql)) {
                return Ok(format!("DROP INDEX {name} ON {table};"));
            }
            {}
            Ok(format!("DROP INDEX {name};"))
        }
        TableChildObjectType::ForeignKey => {
            if matches!(database_type, Some(DatabaseType::Mysql)) {
                Ok(format!("ALTER TABLE {table} DROP FOREIGN KEY {name};"))
            } else {
                Ok(format!("ALTER TABLE {table} DROP CONSTRAINT {name};"))
            }
        }
        TableChildObjectType::Trigger => {
            if database_type.is_some_and(is_schema_aware)
                && options.schema.as_deref().is_some_and(|schema| !schema.is_empty())
                && !matches!(database_type, Some(DatabaseType::Mysql))
            {
                let schema = quote_rename_identifier(database_type, options.schema.as_deref().unwrap());
                Ok(format!("DROP TRIGGER {schema}.{name};"))
            } else {
                Ok(format!("DROP TRIGGER {name};"))
            }
        }
    }
}

pub fn build_empty_table_sql(options: TableAdminSqlOptions) -> String {
    let table = qualified_name_with_quote(
        options.database_type,
        options.schema.as_deref(),
        &options.table_name,
        options.identifier_quote.as_deref(),
    );
    match options.database_type {
        _ => format!("DELETE FROM {table};"),
    }
}

pub fn build_truncate_table_sql(options: TableAdminSqlOptions) -> String {
    let table = qualified_name_with_quote(
        options.database_type,
        options.schema.as_deref(),
        &options.table_name,
        options.identifier_quote.as_deref(),
    );
    {
        // TRUNCATE CASCADE is PostgreSQL-family syntax; other dialects keep their existing default.
        let cascade = if options.cascade.unwrap_or(false) && supports_truncate_table_cascade(options.database_type) {
            " CASCADE"
        } else {
            ""
        };
        format!("TRUNCATE TABLE {table}{cascade};")
    }
}

pub fn build_vacuum_table_sql(options: VacuumTableSqlOptions) -> Result<String, String> {
    if !supports_vacuum_table(options.database_type) {
        return Err(format!("VACUUM is not supported for {}.", database_label(options.database_type)));
    }
    let table = qualified_name(options.database_type, options.schema.as_deref(), &options.table_name);
    let full = if options.full { " FULL" } else { "" };
    let analyze = if options.analyze { " ANALYZE" } else { "" };
    Ok(format!("VACUUM{full}{analyze} {table};"))
}

fn supports_vacuum_table(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn build_mysql_auto_increment_sql(options: MysqlAutoIncrementSqlOptions) -> Result<String, String> {
    let profile = options.driver_profile.as_deref().map(str::trim).unwrap_or_default();
    if options.database_type != Some(DatabaseType::Mysql)
        || (!profile.is_empty() && !profile.eq_ignore_ascii_case("mysql"))
    {
        return Err("Setting AUTO_INCREMENT is supported only for native MySQL connections.".to_string());
    }

    let value = options.value.as_str();
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.starts_with('0')
        || value.parse::<u64>().is_err()
    {
        return Err("AUTO_INCREMENT must be a decimal integer from 1 to 18446744073709551615.".to_string());
    }

    let table = if options.schema.as_deref().is_some_and(|schema| !schema.is_empty()) {
        format!(
            "{}.{}",
            quote_rename_identifier(options.database_type, options.schema.as_deref().unwrap()),
            quote_rename_identifier(options.database_type, &options.table_name)
        )
    } else {
        quote_rename_identifier(options.database_type, &options.table_name)
    };
    Ok(format!("ALTER TABLE {table} AUTO_INCREMENT = {value};"))
}

fn supports_truncate_table_cascade(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn build_drop_database_sql(options: DatabaseNameSqlOptions) -> String {
    let name = quote_table_identifier(options.database_type, &options.name);
    {}
    format!("DROP DATABASE {name};")
}

pub fn build_update_database_properties_sql(options: DatabasePropertyEditSqlOptions) -> Result<String, String> {
    match options.target {
        DatabasePropertyTarget::Database => {
            if options.comment.is_some() {
                return build_database_comment_sql(options.database_type, &options.name, options.comment.as_deref());
            }
            build_database_charset_sql(&options)
        }
        DatabasePropertyTarget::Schema => {
            build_schema_comment_sql(options.database_type, &options.name, options.comment.as_deref())
        }
    }
}

fn build_database_charset_sql(options: &DatabasePropertyEditSqlOptions) -> Result<String, String> {
    if !supports_database_property_charset(options.database_type, options.driver_profile.as_deref()) {
        return Err(format!(
            "Editing database charset/collation is not supported for {}.",
            database_label(options.database_type)
        ));
    }
    let charset = clean_sql_option(options.charset.as_deref());
    let collation = clean_sql_option(options.collation.as_deref());
    if charset.is_empty() && collation.is_empty() {
        return Err("At least one charset or collation value is required.".to_string());
    }
    let mut sql = format!("ALTER DATABASE {}", quote_table_identifier(options.database_type, &options.name));
    if !charset.is_empty() {
        sql.push_str(&format!(" DEFAULT CHARACTER SET {charset}"));
    }
    if !collation.is_empty() {
        sql.push_str(&format!(" DEFAULT COLLATE {collation}"));
    }
    sql.push(';');
    Ok(sql)
}

fn build_database_comment_sql(
    database_type: Option<DatabaseType>,
    name: &str,
    comment: Option<&str>,
) -> Result<String, String> {
    if !supports_database_property_comment(database_type) {
        return Err(format!("Editing database comments is not supported for {}.", database_label(database_type)));
    }
    Ok(format!("COMMENT ON DATABASE {} IS {};", quote_table_identifier(database_type, name), comment_literal(comment)))
}

fn build_schema_comment_sql(
    database_type: Option<DatabaseType>,
    name: &str,
    comment: Option<&str>,
) -> Result<String, String> {
    if !supports_database_property_comment(database_type) {
        return Err(format!("Editing schema comments is not supported for {}.", database_label(database_type)));
    }
    Ok(format!("COMMENT ON SCHEMA {} IS {};", quote_table_identifier(database_type, name), comment_literal(comment)))
}

pub fn build_create_schema_sql(options: SchemaNameSqlOptions) -> Result<String, String> {
    if !supports_create_schema_target(options.database_type) {
        return Err(format!("Creating schemas is not supported for {}.", database_label(options.database_type)));
    }
    Ok(format!("CREATE SCHEMA {};", quote_table_identifier(options.database_type, &options.name)))
}

pub fn build_drop_schema_sql(options: SchemaNameSqlOptions) -> String {
    let schema = quote_table_identifier(options.database_type, &options.name);
    {
        format!("DROP SCHEMA {schema};")
    }
}

pub fn build_duplicate_table_structure_sql(options: DuplicateTableStructureSqlOptions) -> String {
    let source = qualified_name_with_quote(
        options.database_type,
        options.schema.as_deref(),
        &options.source_name,
        options.identifier_quote.as_deref(),
    );
    let target =
        qualified_duplicate_target_name(options.database_type, options.schema.as_deref(), &options.target_name);
    let structure_sql = if matches!(options.database_type, Some(DatabaseType::Mysql)) {
        format!("CREATE TABLE {target} LIKE {source};")
    } else if options.database_type.is_some_and(uses_false_predicate_duplicate_structure) {
        format!("CREATE TABLE {target} AS SELECT * FROM {source} WHERE 1=0")
    } else {
        // `WHERE 1=0` rather than `WHERE 0`: PostgreSQL-family engines (HighGo, Kingbase,
        // ...) and DuckDB require a boolean in WHERE and reject a bare integer
        // with "argument of WHERE must be type boolean, not type integer" (#9950).
        // `1=0` is a valid false predicate in every dialect, including the permissive
        // MySQL/SQLite-style engines that also accepted `0`.
        format!("CREATE TABLE {target} AS SELECT * FROM {source} WHERE 1=0;")
    };

    let mut comment_sql = Vec::new();
    if let Some(database_type) =
        options.database_type.filter(|database_type| supports_duplicate_table_comment(*database_type))
    {
        if let Some(comment) = options.table_comment.as_deref().filter(|comment| !comment.trim().is_empty()) {
            comment_sql.push(format!(
                "COMMENT ON TABLE {target} IS {}",
                quote_duplicate_table_comment(database_type, comment)
            ));
        }
    }
    {}

    // SELECT INTO does not copy MS_Description extended properties. The
    // target is newly created, so add each supplied comment without updating
    // or deleting any pre-existing object's properties.
    {}

    // `SELECT ... INTO` copies the IDENTITY property but not constraints, so the cloned table
    // would silently lose its primary key (t8y2/dbx#8931). Recreate it from the source metadata.
    let mut constraint_sql = Vec::new();
    {}

    let mut trailing_sql = constraint_sql;
    trailing_sql.extend(comment_sql);
    if trailing_sql.is_empty() {
        return structure_sql;
    }
    format!("{};\n{};", structure_sql.trim_end_matches(';'), trailing_sql.join(";\n"))
}

pub fn build_copy_table_data_sql(options: CopyTableDataSqlOptions) -> String {
    let source = qualified_name_with_quote(
        options.database_type,
        options.schema.as_deref(),
        &options.source_name,
        options.identifier_quote.as_deref(),
    );
    let target = if options.normalize_new_target_name {
        qualified_duplicate_target_name(options.database_type, options.schema.as_deref(), &options.target_name)
    } else {
        qualified_name_with_quote(
            options.database_type,
            options.schema.as_deref(),
            &options.target_name,
            options.identifier_quote.as_deref(),
        )
    };
    let Some(columns) = options.columns.filter(|columns| !columns.is_empty()) else {
        return format!("INSERT INTO {target} SELECT * FROM {source};");
    };
    let source_column_list = columns
        .iter()
        .map(|column| quote_table_identifier(options.database_type, column))
        .collect::<Vec<_>>()
        .join(", ");
    // The unquoted Oracle clone DDL creates the target columns case-folded while the source
    // keeps its exact stored spelling, so the INSERT target list must reference the folded
    // forms and the SELECT list keeps reading the source exactly.
    let target_column_list = { source_column_list.clone() };
    let postgres_override = { "" };
    let insert_sql = format!(
        "INSERT INTO {target} ({target_column_list}){postgres_override} SELECT {source_column_list} FROM {source};"
    );
    let needs_identity_insert = false;
    {}
    insert_sql
}

pub fn supports_object_rename(database_type: Option<DatabaseType>, object_type: DatabaseObjectType) -> bool {
    let Some(database_type) = database_type else {
        return false;
    };
    {}
    {}
    if matches!(object_type, DatabaseObjectType::Procedure | DatabaseObjectType::Function) {
        return false;
    }
    {}
    {
        return matches!(object_type, DatabaseObjectType::Table | DatabaseObjectType::View);
    }
}

pub fn build_rename_object_sql(options: RenameObjectSqlOptions) -> Result<String, String> {
    let database_type = options.database_type;
    if !supports_object_rename(database_type, options.object_type) {
        return Err(format!(
            "Renaming {} is not supported for {}.",
            object_type_keyword(options.object_type),
            database_label(database_type)
        ));
    }

    {}

    {}

    {}

    if matches!(database_type, Some(DatabaseType::Mysql)) {
        return Ok(format!(
            "RENAME TABLE {} TO {};",
            qualified_name(database_type, options.schema.as_deref(), &options.old_name),
            qualified_name(database_type, options.schema.as_deref(), &options.new_name)
        ));
    }

    {}

    {}

    Err(format!(
        "Renaming {} is not supported for {}.",
        object_type_keyword(options.object_type),
        database_label(database_type)
    ))
}

pub fn supports_database_rename(database_type: Option<DatabaseType>) -> bool {
    false
}

pub fn build_rename_database_sql(
    database_type: Option<DatabaseType>,
    old_name: &str,
    new_name: &str,
    terminate_connections: bool,
) -> Result<String, String> {
    if !supports_database_rename(database_type) {
        return Err(format!("Renaming databases is not supported for {}.", database_label(database_type)));
    }
    let mut parts = Vec::new();
    if terminate_connections {
        let escaped = old_name.replace('\'', "''");
        parts.push(format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{escaped}' AND pid <> pg_backend_pid();"
        ));
    }
    parts.push(format!(
        "ALTER DATABASE {} RENAME TO {};",
        quote_table_identifier(database_type, old_name),
        quote_rename_identifier(database_type, new_name)
    ));
    Ok(parts.join("\n"))
}

/// Generates a preflight SQL query that checks whether the database can be renamed.
/// Returns a single-row result with:
///   - active_connections: number of connections to the database (excluding self)
///   - prepared_transactions: number of prepared transactions in the database
///   - is_owner: whether the current user owns the database
pub fn build_rename_database_preflight_sql(
    database_type: Option<DatabaseType>,
    database_name: &str,
) -> Result<String, String> {
    if !supports_database_rename(database_type) {
        return Err(format!("Renaming databases is not supported for {}.", database_label(database_type)));
    }
    let escaped = database_name.replace('\'', "''");
    Ok(format!(
        "SELECT (SELECT count(*) FROM pg_stat_activity WHERE datname = '{escaped}' AND pid <> pg_backend_pid()) AS active_connections, (SELECT count(*) FROM pg_prepared_xacts WHERE database = '{escaped}') AS prepared_transactions, (SELECT pg_catalog.pg_get_userbyid(datdba) = current_user AS is_owner FROM pg_database WHERE datname = '{escaped}') AS is_owner;"
    ))
}

fn supports_duplicate_table_comment(database_type: DatabaseType) -> bool {
    false
}

fn uses_false_predicate_duplicate_structure(database_type: DatabaseType) -> bool {
    false
}

fn quote_rename_identifier(database_type: Option<DatabaseType>, name: &str) -> String {
    if matches!(database_type, Some(DatabaseType::Mysql)) {
        format!("`{}`", name.replace('`', "``"))
    } else {
        quote_table_identifier(database_type, name)
    }
}

fn qualified_name(database_type: Option<DatabaseType>, schema: Option<&str>, name: &str) -> String {
    {}
    if database_type.is_some_and(is_schema_aware) && schema.is_some_and(|schema| !schema.is_empty()) {
        format!(
            "{}.{}",
            quote_rename_identifier(database_type, schema.unwrap()),
            quote_rename_identifier(database_type, name)
        )
    } else {
        quote_rename_identifier(database_type, name)
    }
}

/// Same as [`qualified_name`], but honours the quote character the connected server reported.
///
/// The static per-type mapping cannot describe a database whose quote depends on the connection.
/// Cloud Spanner is the case that forces this: GoogleSQL quotes with backticks while its PostgreSQL
/// dialect quotes with double quotes, the dialect is fixed when the database is created, and only
/// the connected agent knows which one applies. Emitting the static answer produces admin SQL that
/// is a syntax error for every PostgreSQL-dialect Spanner database.
///
/// Falls back to [`qualified_name`] whenever no connection quote applies, so every other database
/// type — and Spanner itself before the agent has reported a quote — keeps its existing output.
fn qualified_name_with_quote(
    database_type: Option<DatabaseType>,
    schema: Option<&str>,
    name: &str,
    identifier_quote: Option<&str>,
) -> String {
    if !uses_connection_identifier_quote(database_type, identifier_quote) {
        return qualified_name(database_type, schema, name);
    }
    let quoted = |value: &str| quote_table_data_identifier(database_type, value, identifier_quote);
    if database_type.is_some_and(is_schema_aware) && schema.is_some_and(|schema| !schema.is_empty()) {
        format!("{}.{}", quoted(schema.unwrap()), quoted(name))
    } else {
        quoted(name)
    }
}

fn qualified_duplicate_target_name(database_type: Option<DatabaseType>, schema: Option<&str>, name: &str) -> String {
    // Both duplicate-target normalizations emit the spelling a freshly created clone resolves
    // to: Oracle's clone DDL creates plain identifiers unquoted (uppercase fold), and Dameng's
    // clone keeps the same fold convention through its profile.  Every other type stays exact.
    let target = match database_type {
        _ => return qualified_name(database_type, schema, name),
    };
    if schema.is_some_and(|schema| !schema.is_empty()) {
        format!("{}.{}", quote_rename_identifier(database_type, schema.unwrap()), target)
    } else {
        target
    }
}

fn object_type_keyword(object_type: DatabaseObjectType) -> &'static str {
    match object_type {
        DatabaseObjectType::Table => "TABLE",
        DatabaseObjectType::View => "VIEW",
        DatabaseObjectType::MaterializedView => "MATERIALIZED VIEW",
        DatabaseObjectType::Procedure => "PROCEDURE",
        DatabaseObjectType::Function => "FUNCTION",
        DatabaseObjectType::Event => "EVENT",
    }
}

fn clean_sql_option(value: Option<&str>) -> String {
    value.unwrap_or("").trim().replace([';', ' ', '\n', '\r', '\t'], "")
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn quote_duplicate_table_comment(database_type: DatabaseType, value: &str) -> String {
    // Dameng rejects Postgres E'...' escape strings; keep standard '' escaping
    // (same as DamengAgent COMMENT ON / COMMENT ON COLUMN generation).
    {}
    if !value.contains('\\') && !value.chars().any(|character| character.is_ascii_control()) {
        return quote_sql_string(value);
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\x08' => escaped.push_str("\\b"),
            '\x0c' => escaped.push_str("\\f"),
            '\'' => escaped.push_str("\\'"),
            character if character.is_ascii_control() => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                let byte = character as u8;
                escaped.push_str("\\x");
                escaped.push(HEX[(byte >> 4) as usize] as char);
                escaped.push(HEX[(byte & 0x0F) as usize] as char);
            }
            character => escaped.push(character),
        }
    }

    let prefix = { "E" };
    format!("{prefix}'{escaped}'")
}

fn comment_literal(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => quote_sql_string(value),
        None => "NULL".to_string(),
    }
}

fn database_label(database_type: Option<DatabaseType>) -> String {
    database_type
        .and_then(|database_type| serde_json::to_value(database_type).ok())
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "this database".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_mysql_create_database_sql_with_charset_and_collation() {
        assert_eq!(
            build_create_database_sql(CreateDatabaseSqlOptions {
                database_type: Some(DatabaseType::Mysql),
                driver_profile: Some("mysql".to_string()),
                target: None,
                parent: None,
                name: "app db".to_string(),
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_unicode_ci".to_string()),
            })
            .unwrap(),
            "CREATE DATABASE `app db` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
        );
    }
}
