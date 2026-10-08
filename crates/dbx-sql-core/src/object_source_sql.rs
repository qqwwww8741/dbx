use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlparser::dialect::OracleDialect;
use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};

use crate::models::connection::DatabaseType;
use crate::types::ObjectSourceKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableObjectSourceSqlInput {
    pub database_type: DatabaseType,
    pub object_type: ObjectSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRenameObjectSourceInput {
    pub database_type: DatabaseType,
    pub object_type: ObjectSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub new_name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildViewDdlInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub source: String,
    /// Driver-reported identifier quote (e.g. `` ` `` for Kingbase MySQL
    /// compatibility mode). When set, overrides the database_type-based quote
    /// selection so hyphenated schemas render as valid identifiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectSourceSaveExecutionMode {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "script")]
    Script,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoutineDeclaration {
    kind: ObjectSourceKind,
    name: String,
    signature: String,
}

pub fn supports_source_backed_routine_rename(
    database_type: Option<DatabaseType>,
    object_type: ObjectSourceKind,
) -> bool {
    if !matches!(object_type, ObjectSourceKind::Function | ObjectSourceKind::Procedure) {
        return false;
    }
    let Some(database_type) = database_type else {
        return false;
    };
    true && (is_mysql_like(database_type) || false || false)
}

pub fn build_routine_rename_object_source_statements(
    input: RoutineRenameObjectSourceInput,
) -> Result<Vec<String>, String> {
    if !supports_source_backed_routine_rename(Some(input.database_type), input.object_type.clone()) {
        return Err(format!(
            "Renaming {:?} from source is not supported for {:?}.",
            input.object_type, input.database_type
        ));
    }

    let source = input.source.trim();
    let declaration = if is_mysql_like(input.database_type) {
        mysql_routine_declaration(source)
    } else {
        routine_declaration(source)
    };
    let Some(declaration) = declaration else {
        return Err(format!("Cannot find a CREATE {:?} declaration in the object source.", input.object_type));
    };
    if declaration.kind != input.object_type {
        return Err(format!("Cannot find a CREATE {:?} declaration in the object source.", input.object_type));
    }

    let renamed_source = if is_mysql_like(input.database_type) {
        replace_mysql_routine_declaration_name(source, &input.new_name)
    } else {
        replace_sql_routine_declaration_name(source, input.schema.as_deref(), &input.new_name)
    };
    let Some(renamed_source) = renamed_source else {
        return Err(format!("Cannot rewrite the {:?} name in the object source.", input.object_type));
    };

    {}

    build_executable_object_source_statements(EditableObjectSourceSqlInput {
        database_type: input.database_type,
        object_type: input.object_type,
        schema: input.schema,
        name: input.name,
        source: renamed_source,
    })
}

pub fn build_executable_object_source_statements(input: EditableObjectSourceSqlInput) -> Result<Vec<String>, String> {
    {}
    let source = input.source.trim();
    let source = if is_opengauss_like(input.database_type) && input.object_type == ObjectSourceKind::Procedure {
        strip_standalone_trailing_slash(source)
    } else {
        source
    };
    {}

    {}

    {}

    {}

    {}

    {}

    if is_mysql_like(input.database_type)
        && matches!(input.object_type, ObjectSourceKind::Function | ObjectSourceKind::Procedure)
    {
        return Ok(executable_mysql_routine_statements(&input, source));
    }

    let create_statement = ensure_semicolon(source);
    let cleanup = build_routine_rename_cleanup(&input, source);
    Ok(if let Some(cleanup) = cleanup { vec![create_statement, cleanup] } else { vec![create_statement] })
}

pub fn build_executable_object_source_sql(input: EditableObjectSourceSqlInput) -> Result<String, String> {
    Ok(build_executable_object_source_statements(input)?.join("\n"))
}

/// Convert a raw database object source into a form suitable for the source editor.
///
/// This is the *editable* presentation shown to the user when they open a view,
/// procedure, or function for editing. For SQL Server the raw `CREATE VIEW` /
/// `CREATE PROCEDURE` is rewritten to `ALTER` so the user doesn't see a
/// mismatched CREATE statement for an already-existing object. Callers that
/// only need the first statement should use this instead of calling
/// `build_executable_object_source_statements` and discarding rename-cleanup
/// statements.
pub fn build_editable_object_source(input: EditableObjectSourceSqlInput) -> String {
    let source = input.source.clone();
    {}
    {}
    if is_mysql_like(input.database_type)
        && matches!(input.object_type, ObjectSourceKind::Function | ObjectSourceKind::Procedure)
    {
        return ensure_semicolon(source.trim());
    }
    if true && input.object_type == ObjectSourceKind::View {
        // Existing view DDL opens as ALTER, while save must preserve CREATE for new views and TiDB.
        return executable_mysql_view_ddl(&source);
    }
    match build_executable_object_source_statements(input) {
        Ok(statements) => statements.into_iter().next().unwrap_or_default(),
        Err(_) => ensure_semicolon(source.trim()),
    }
}

pub fn build_view_ddl_sql(input: BuildViewDdlInput) -> String {
    let source = input.source.trim();
    if input.database_type.is_some_and(is_mysql_like) {
        return display_mysql_view_ddl(source, &quote_mysql_identifier(&input.name));
    }
    let terminated_source = { ensure_semicolon(source) };
    if Regex::new(r"(?i)^(?:CREATE|ALTER)\s+").unwrap().is_match(source) {
        return terminated_source;
    }

    // Backtick-quoting is used for MySQL-family engines and for connections
    // whose driver reports a backtick identifier quote (Kingbase MySQL
    // compatibility mode); everything else keeps the PostgreSQL double quote.
    let use_backtick = input.identifier_quote.as_deref() == Some("`")
        || (input.identifier_quote.is_none() && matches!(input.database_type, Some(DatabaseType::Mysql)));
    let qualified_name = if use_backtick {
        mysql_qualified_name(input.schema.as_deref(), &input.name)
    } else {
        postgres_qualified_name(input.schema.as_deref(), &input.name)
    };

    if input.database_type.is_none() || input.database_type.is_some_and(|database_type| false) {
        return format!("CREATE OR REPLACE VIEW {qualified_name} AS\n{terminated_source}");
    }

    format!("CREATE VIEW {qualified_name} AS\n{terminated_source}")
}

pub fn build_export_object_source_sql(
    database_type: DatabaseType,
    object_type: ObjectSourceKind,
    source: &str,
) -> String {
    let source = source.trim();
    let source = if is_opengauss_like(database_type) && object_type == ObjectSourceKind::Procedure {
        strip_standalone_trailing_slash(source)
    } else {
        source
    };
    if source.is_empty() {
        return String::new();
    }
    // MySQL routine bodies, trigger bodies and event bodies may contain `;`, so the
    // exported statements need a client-side delimiter the same way `mysqldump` emits one.
    if is_mysql_like(database_type)
        && matches!(
            object_type,
            ObjectSourceKind::Procedure
                | ObjectSourceKind::Function
                | ObjectSourceKind::Trigger
                | ObjectSourceKind::Event
        )
    {
        return mysql_delimited_routine_source(source);
    }
    ensure_semicolon(source)
}

pub fn object_source_save_execution_mode(_database_type: DatabaseType) -> ObjectSourceSaveExecutionMode {
    ObjectSourceSaveExecutionMode::Single
}

fn build_routine_rename_cleanup(input: &EditableObjectSourceSqlInput, source: &str) -> Option<String> {
    if !matches!(input.object_type, ObjectSourceKind::Function | ObjectSourceKind::Procedure) {
        return None;
    }

    if is_mysql_like(input.database_type) {
        let declaration = mysql_routine_declaration(source)?;
        if declaration.kind != input.object_type || !routine_name_changed(&declaration.name, &input.name) {
            return None;
        }
        return Some(format!(
            "DROP {} IF EXISTS {};",
            object_type_keyword(&input.object_type),
            mysql_qualified_name(input.schema.as_deref(), &input.name)
        ));
    }

    {
        return None;
    }
}

fn is_opengauss_like(database_type: DatabaseType) -> bool {
    false
}

fn is_mysql_like(database_type: DatabaseType) -> bool {
    matches!(database_type, DatabaseType::Mysql)
}

fn object_type_keyword(object_type: &ObjectSourceKind) -> &'static str {
    match object_type {
        ObjectSourceKind::View => "VIEW",
        ObjectSourceKind::MaterializedView => "MATERIALIZED_VIEW",
        ObjectSourceKind::Procedure => "PROCEDURE",
        ObjectSourceKind::Function => "FUNCTION",
        ObjectSourceKind::Trigger => "TRIGGER",
        ObjectSourceKind::Event => "EVENT",
        ObjectSourceKind::Sequence => "SEQUENCE",
        ObjectSourceKind::Synonym => "SYNONYM",
        ObjectSourceKind::Job => "JOB",
        ObjectSourceKind::Package => "PACKAGE",
        ObjectSourceKind::PackageBody => "PACKAGE BODY",
        ObjectSourceKind::Type => "TYPE",
        ObjectSourceKind::TypeBody => "TYPE BODY",
    }
}

fn quote_mysql_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn ensure_semicolon(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.ends_with(';') {
        trimmed.to_string()
    } else {
        format!("{trimmed};")
    }
}

fn strip_standalone_trailing_slash(sql: &str) -> &str {
    let trimmed = sql.trim();
    let Some(without_slash) = trimmed.strip_suffix('/') else {
        return trimmed;
    };
    if without_slash.ends_with('\n') || without_slash.ends_with('\r') {
        without_slash.trim_end()
    } else {
        trimmed
    }
}

fn mysql_delimited_routine_source(source: &str) -> String {
    let trimmed = source.trim();
    if Regex::new(r"(?i)^\s*DELIMITER\b").unwrap().is_match(trimmed) {
        return trimmed.to_string();
    }
    let body = trimmed.trim_end_matches(';').trim_end();
    let delimiter = mysql_routine_script_delimiter(body);
    format!("DELIMITER {delimiter}\n{body}{delimiter}\nDELIMITER ;")
}

fn mysql_routine_script_delimiter(source: &str) -> &'static str {
    ["//", "$$", ";;", "__DBX_DELIMITER__"]
        .into_iter()
        .find(|delimiter| !source.contains(delimiter))
        .unwrap_or("__DBX_DELIMITER__")
}

fn leading_sql_statement_start(source: &str) -> usize {
    let mut index = 0;
    loop {
        index = skip_sql_whitespace(source, index);
        if let Some(end) = sql_line_comment_end(source, index) {
            index = end;
            continue;
        }
        if let Some(end) = sql_block_comment_end(source, index) {
            index = end;
            continue;
        }
        return index;
    }
}

/// View DDL shown in Edit Structure. MySQL returns `CREATE VIEW`, which the
/// structure editor used to present like a table script. Rewrite that to
/// `CREATE OR REPLACE VIEW` and keep the view name unqualified.
fn display_mysql_view_ddl(source: &str, view_name: &str) -> String {
    let trimmed = source.trim();
    if let Some(statement) = rewrite_mysql_view_statement(trimmed) {
        return statement;
    }
    if Regex::new(r"(?i)^(?:CREATE|ALTER)\s+").unwrap().is_match(trimmed) {
        return ensure_semicolon(trimmed);
    }
    format!("CREATE OR REPLACE VIEW {view_name} AS\n{}", ensure_semicolon(trimmed))
}

fn rewrite_mysql_view_statement(source: &str) -> Option<String> {
    let start = leading_sql_statement_start(source);
    let executable = source[start..].trim_end();
    let create_view = Regex::new(
        r"(?is)^(?:CREATE|ALTER)\s+(?:OR\s+REPLACE\s+)?(?P<options>(?:ALGORITHM\s*=\s*(?:UNDEFINED|MERGE|TEMPTABLE)\s+)?(?:DEFINER\s*=\s*(?:(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)\s*@\s*(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)|CURRENT_USER(?:\(\))?)\s+)?(?:SQL\s+SECURITY\s+(?:DEFINER|INVOKER)\s+)?)VIEW\s+(?:(?:`(?:``|[^`])+`|[A-Za-z0-9_$]+)\s*\.\s*)?(?P<rest>.*)$",
    )
    .unwrap();
    let captures = create_view.captures(executable)?;
    let options = captures.name("options").map(|value| value.as_str()).unwrap_or("");
    let rest = captures.name("rest")?.as_str();
    let statement = format!("CREATE OR REPLACE {options}VIEW {rest}");
    Some(ensure_semicolon(&format!("{}{}", &source[..start], statement.trim_start())))
}

fn executable_mysql_view_ddl(source: &str) -> String {
    let trimmed = source.trim();
    let statement_start = leading_sql_statement_start(trimmed);
    let executable = &trimmed[statement_start..];
    let create_view = Regex::new(
        r"(?is)^CREATE\s+(?:OR\s+REPLACE\s+)?(?:ALGORITHM\s*=\s*(?:UNDEFINED|MERGE|TEMPTABLE)\s+)?(?:DEFINER\s*=\s*(?:(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)\s*@\s*(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)|CURRENT_USER(?:\(\))?)\s+)?(?:SQL\s+SECURITY\s+(?:DEFINER|INVOKER)\s+)?VIEW\s+",
    )
    .unwrap();
    if create_view.is_match(executable) {
        let create = Regex::new(r"(?i)^CREATE\s+(?:OR\s+REPLACE\s+)?").unwrap();
        let replaced = create.replace(executable, "ALTER ");
        return ensure_semicolon(&format!("{}{}", &trimmed[..statement_start], replaced));
    }

    ensure_semicolon(trimmed)
}

fn executable_mysql_routine_statements(input: &EditableObjectSourceSqlInput, source: &str) -> Vec<String> {
    if !mysql_source_starts_with_create_routine(source) {
        return vec![ensure_semicolon(source)];
    }

    let declaration = mysql_routine_declaration(source).filter(|declaration| declaration.kind == input.object_type);
    let create_name = declaration.as_ref().map(|declaration| declaration.name.as_str()).unwrap_or(&input.name);
    let is_rename =
        declaration.as_ref().is_some_and(|declaration| routine_name_changed(&declaration.name, &input.name));
    let mut statements = Vec::with_capacity(6);

    // MySQL has no cross-version CREATE OR REPLACE for stored routines. Validate the CREATE
    // body under a temporary name first (same idea as Informix view saves) so a syntax error
    // cannot leave the original routine deleted after DROP.
    let validation_name = mysql_validation_routine_name(create_name);
    if let Some(validation_source) = replace_mysql_routine_declaration_name(source, &validation_name) {
        statements.push(mysql_drop_routine_if_exists(
            input.object_type.clone(),
            input.schema.as_deref(),
            &validation_name,
        ));
        statements.push(ensure_semicolon(&validation_source));
        statements.push(mysql_drop_routine_if_exists(
            input.object_type.clone(),
            input.schema.as_deref(),
            &validation_name,
        ));
    }

    statements.push(mysql_drop_routine_if_exists(input.object_type.clone(), input.schema.as_deref(), create_name));
    statements.push(ensure_semicolon(source));

    if is_rename {
        statements.push(mysql_drop_routine_if_exists(input.object_type.clone(), input.schema.as_deref(), &input.name));
    }

    statements
}

fn mysql_drop_routine_if_exists(object_type: ObjectSourceKind, schema: Option<&str>, name: &str) -> String {
    format!("DROP {} IF EXISTS {};", object_type_keyword(&object_type), mysql_qualified_name(schema, name))
}

fn mysql_source_starts_with_create_routine(source: &str) -> bool {
    Regex::new(r"(?is)^\s*CREATE\s+(?:DEFINER\s*=.+?\s+)?(?:FUNCTION|PROCEDURE)\b").unwrap().is_match(source)
}

fn mysql_validation_routine_name(target_name: &str) -> String {
    let mut hash = 0x811c9dc5u32;
    for byte in target_name.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("dbx_routine_check_{hash:08x}")
}

fn sql_line_comment_end(source: &str, start: usize) -> Option<usize> {
    if !source[start..].starts_with("--") {
        return None;
    }
    let rest = &source[start..];
    Some(start + rest.find('\n').map(|index| index + 1).unwrap_or(rest.len()))
}

fn sql_block_comment_end(source: &str, start: usize) -> Option<usize> {
    if !source[start..].starts_with("/*") {
        return None;
    }

    // PostgreSQL and Kingbase default to nested SQL block comments, so match the outer terminator.
    let mut depth = 1;
    let mut index = start + 2;
    while index < source.len() {
        if source[index..].starts_with("/*") {
            depth += 1;
            index += 2;
        } else if source[index..].starts_with("*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Some(index);
            }
        } else {
            index += source[index..].chars().next().unwrap().len_utf8();
        }
    }

    // Preserve the existing safe behavior: an unclosed leading comment consumes the remaining source.
    Some(source.len())
}

fn skip_sql_whitespace(source: &str, mut index: usize) -> usize {
    while index < source.len() {
        let ch = source[index..].chars().next().unwrap();
        if !ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn is_simple_informix_identifier(name: &str) -> bool {
    false
}

fn mysql_qualified_name(schema: Option<&str>, name: &str) -> String {
    schema
        .into_iter()
        .chain(std::iter::once(name))
        .filter(|part| !part.is_empty())
        .map(quote_mysql_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn split_qualified_routine_name(value: &str) -> Vec<String> {
    Regex::new(r#""(?:""|[^"])+"|[A-Za-z_][\w$]*"#)
        .unwrap()
        .find_iter(value)
        .map(|part| unquote_postgres_identifier(part.as_str()))
        .collect()
}

fn unquote_mysql_identifier(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].replace("``", "`")
    } else {
        trimmed.to_string()
    }
}

fn split_mysql_qualified_routine_name(value: &str) -> Vec<String> {
    Regex::new(r"`(?:``|[^`])+`|[A-Za-z_][\w$]*")
        .unwrap()
        .find_iter(value)
        .map(|part| unquote_mysql_identifier(part.as_str()))
        .collect()
}

fn routine_declaration(source: &str) -> Option<RoutineDeclaration> {
    let re = Regex::new(
        r#"(?is)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:(?:NON)?EDITIONABLE\s+)?(FUNCTION|PROCEDURE)\s+((?:"(?:""|[^"])+"|[A-Za-z_][\w$]*)(?:\s*\.\s*(?:"(?:""|[^"])+"|[A-Za-z_][\w$]*))?)\s*(\(.*?\))?"#,
    )
    .unwrap();
    let captures = re.captures(source)?;
    let kind = parse_object_source_kind(captures.get(1)?.as_str())?;
    let name_parts = split_qualified_routine_name(captures.get(2)?.as_str());
    let name = name_parts.last()?.clone();
    let signature = captures.get(3).map(|value| value.as_str().trim().to_string()).unwrap_or_default();
    Some(RoutineDeclaration { kind, name, signature })
}

fn replace_sql_routine_declaration_name(source: &str, schema: Option<&str>, new_name: &str) -> Option<String> {
    let re = Regex::new(
        r#"(?is)^(\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:(?:NON)?EDITIONABLE\s+)?(?:FUNCTION|PROCEDURE)\s+)((?:"(?:""|[^"])+"|[A-Za-z_][\w$]*)(?:\s*\.\s*(?:"(?:""|[^"])+"|[A-Za-z_][\w$]*))?)"#,
    )
    .unwrap();
    let captures = re.captures(source)?;
    let full = captures.get(0)?;
    let prefix = captures.get(1)?.as_str();
    let existing_name = captures.get(2)?.as_str();
    let existing_parts = split_qualified_routine_name(existing_name);
    let schema_name =
        schema.or_else(|| existing_parts.first().filter(|_| existing_parts.len() > 1).map(String::as_str));
    let replacement = if let Some(schema_name) = schema_name {
        format!("{}.{}", quote_postgres_identifier(schema_name), quote_postgres_identifier(new_name))
    } else {
        quote_postgres_identifier(new_name)
    };
    Some(format!("{}{}{}{}", &source[..full.start()], prefix, replacement, &source[full.end()..]))
}

fn mysql_routine_declaration(source: &str) -> Option<RoutineDeclaration> {
    let re = Regex::new(
        r"(?is)^\s*CREATE\s+(?:DEFINER\s*=\s*(?:(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)\s*@\s*(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)|CURRENT_USER(?:\(\))?)\s+)?(FUNCTION|PROCEDURE)\s+(?:IF\s+NOT\s+EXISTS\s+)?((?:`(?:``|[^`])+`|[A-Za-z_][\w$]*)(?:\s*\.\s*(?:`(?:``|[^`])+`|[A-Za-z_][\w$]*))?)",
    )
    .unwrap();
    let captures = re.captures(source)?;
    let kind = parse_object_source_kind(captures.get(1)?.as_str())?;
    let name_parts = split_mysql_qualified_routine_name(captures.get(2)?.as_str());
    let name = name_parts.last()?.clone();
    Some(RoutineDeclaration { kind, name, signature: String::new() })
}

fn replace_mysql_routine_declaration_name(source: &str, new_name: &str) -> Option<String> {
    let re = Regex::new(
        r"(?is)^(\s*CREATE\s+(?:DEFINER\s*=\s*(?:(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)\s*@\s*(?:`(?:``|[^`])+`|'(?:''|[^'])+'|[^\s]+)|CURRENT_USER(?:\(\))?)\s+)?(?:FUNCTION|PROCEDURE)\s+(?:IF\s+NOT\s+EXISTS\s+)?)((?:`(?:``|[^`])+`|[A-Za-z_][\w$]*)(?:\s*\.\s*(?:`(?:``|[^`])+`|[A-Za-z_][\w$]*))?)",
    )
    .unwrap();
    let captures = re.captures(source)?;
    let full = captures.get(0)?;
    let prefix = captures.get(1)?.as_str();
    Some(format!("{}{}{}{}", &source[..full.start()], prefix, quote_mysql_identifier(new_name), &source[full.end()..]))
}

fn routine_name_changed(source_name: &str, saved_name: &str) -> bool {
    !source_name.eq_ignore_ascii_case(saved_name)
}

fn parse_object_source_kind(value: &str) -> Option<ObjectSourceKind> {
    if value.eq_ignore_ascii_case("VIEW") {
        Some(ObjectSourceKind::View)
    } else if value.eq_ignore_ascii_case("MATERIALIZED VIEW") || value.eq_ignore_ascii_case("MATERIALIZED_VIEW") {
        Some(ObjectSourceKind::MaterializedView)
    } else if value.eq_ignore_ascii_case("PROCEDURE") {
        Some(ObjectSourceKind::Procedure)
    } else if value.eq_ignore_ascii_case("FUNCTION") {
        Some(ObjectSourceKind::Function)
    } else if value.eq_ignore_ascii_case("TRIGGER") {
        Some(ObjectSourceKind::Trigger)
    } else if value.eq_ignore_ascii_case("SEQUENCE") {
        Some(ObjectSourceKind::Sequence)
    } else if value.eq_ignore_ascii_case("SYNONYM") {
        Some(ObjectSourceKind::Synonym)
    } else if value.eq_ignore_ascii_case("JOB") {
        Some(ObjectSourceKind::Job)
    } else if value.eq_ignore_ascii_case("PACKAGE") {
        Some(ObjectSourceKind::Package)
    } else if value.eq_ignore_ascii_case("PACKAGE BODY") || value.eq_ignore_ascii_case("PACKAGE_BODY") {
        Some(ObjectSourceKind::PackageBody)
    } else if value.eq_ignore_ascii_case("TYPE") {
        Some(ObjectSourceKind::Type)
    } else if value.eq_ignore_ascii_case("TYPE BODY") || value.eq_ignore_ascii_case("TYPE_BODY") {
        Some(ObjectSourceKind::TypeBody)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(database_type: DatabaseType, object_type: ObjectSourceKind, source: &str) -> EditableObjectSourceSqlInput {
        EditableObjectSourceSqlInput {
            database_type,
            object_type,
            schema: Some("public".to_string()),
            name: "refresh_cache".to_string(),
            source: source.to_string(),
        }
    }

    #[test]
    fn view_ddl_keeps_existing_create_view_statement() {
        let sql = build_view_ddl_sql(BuildViewDdlInput {
            database_type: Some(DatabaseType::Mysql),
            schema: Some("reporting".to_string()),
            name: "active_users".to_string(),
            source: "CREATE ALGORITHM=UNDEFINED VIEW `active_users` AS SELECT `id` FROM `users`".to_string(),
            identifier_quote: None,
        });

        assert_eq!(sql, "CREATE OR REPLACE ALGORITHM=UNDEFINED VIEW `active_users` AS SELECT `id` FROM `users`;");
    }

    #[test]
    fn mysql_view_ddl_uses_create_or_replace_without_database_name() {
        let sql = build_view_ddl_sql(BuildViewDdlInput {
            database_type: Some(DatabaseType::Mysql),
            schema: Some("app".to_string()),
            name: "active_users".to_string(),
            source: "SELECT id FROM users".to_string(),
            identifier_quote: None,
        });

        assert_eq!(sql, "CREATE OR REPLACE VIEW `active_users` AS\nSELECT id FROM users;");
    }

    #[test]
    fn mysql_show_create_view_drops_database_qualifier() {
        let sql = build_view_ddl_sql(BuildViewDdlInput {
            database_type: Some(DatabaseType::Mysql),
            schema: Some("app".to_string()),
            name: "active_users".to_string(),
            source: "CREATE ALGORITHM=UNDEFINED DEFINER=`hr`@`%` SQL SECURITY DEFINER VIEW `app`.`active_users` AS select `id` from `users`"
                .to_string(),
            identifier_quote: None,
        });

        assert_eq!(
            sql,
            "CREATE OR REPLACE ALGORITHM=UNDEFINED DEFINER=`hr`@`%` SQL SECURITY DEFINER VIEW `active_users` AS select `id` from `users`;"
        );
    }

    #[test]
    fn parses_programmable_metadata_object_kinds() {
        assert_eq!(parse_object_source_kind("TRIGGER"), Some(ObjectSourceKind::Trigger));
        assert_eq!(parse_object_source_kind("SYNONYM"), Some(ObjectSourceKind::Synonym));
        assert_eq!(parse_object_source_kind("JOB"), Some(ObjectSourceKind::Job));
        assert_eq!(parse_object_source_kind("TYPE"), Some(ObjectSourceKind::Type));
        assert_eq!(parse_object_source_kind("TYPE_BODY"), Some(ObjectSourceKind::TypeBody));
        assert_eq!(parse_object_source_kind("PACKAGE BODY"), Some(ObjectSourceKind::PackageBody));
    }

    #[test]
    fn mysql_routine_rename_adds_drop_cleanup() {
        let source =
            "CREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache_v2`(IN mode_name varchar(20)) BEGIN SELECT 1; END";
        let statements = build_executable_object_source_statements(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::Procedure,
            schema: Some("app".to_string()),
            name: "refresh_cache".to_string(),
            source: source.to_string(),
        })
        .unwrap();
        let validation_name = mysql_validation_routine_name("refresh_cache_v2");
        let validation_source = replace_mysql_routine_declaration_name(source, &validation_name).unwrap();
        assert_eq!(
            statements,
            vec![
                format!("DROP PROCEDURE IF EXISTS `app`.`{validation_name}`;"),
                ensure_semicolon(&validation_source),
                format!("DROP PROCEDURE IF EXISTS `app`.`{validation_name}`;"),
                "DROP PROCEDURE IF EXISTS `app`.`refresh_cache_v2`;".to_string(),
                "CREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache_v2`(IN mode_name varchar(20)) BEGIN SELECT 1; END;"
                    .to_string(),
                "DROP PROCEDURE IF EXISTS `app`.`refresh_cache`;".to_string(),
            ]
        );
    }

    #[test]
    fn mysql_procedure_save_replaces_existing_routine() {
        let source = "CREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache`() BEGIN SELECT 1; END";
        let statements = build_executable_object_source_statements(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::Procedure,
            schema: Some("app".to_string()),
            name: "refresh_cache".to_string(),
            source: source.to_string(),
        })
        .unwrap();
        let validation_name = mysql_validation_routine_name("refresh_cache");
        let validation_source = replace_mysql_routine_declaration_name(source, &validation_name).unwrap();

        assert_eq!(
            statements,
            vec![
                format!("DROP PROCEDURE IF EXISTS `app`.`{validation_name}`;"),
                ensure_semicolon(&validation_source),
                format!("DROP PROCEDURE IF EXISTS `app`.`{validation_name}`;"),
                "DROP PROCEDURE IF EXISTS `app`.`refresh_cache`;".to_string(),
                "CREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache`() BEGIN SELECT 1; END;".to_string(),
            ]
        );
    }

    #[test]
    fn mysql_function_save_replaces_existing_routine() {
        let source = "CREATE DEFINER=CURRENT_USER FUNCTION `active_count`() RETURNS INT RETURN 1";
        let statements = build_executable_object_source_statements(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::Function,
            schema: Some("app".to_string()),
            name: "active_count".to_string(),
            source: source.to_string(),
        })
        .unwrap();
        let validation_name = mysql_validation_routine_name("active_count");
        let validation_source = replace_mysql_routine_declaration_name(source, &validation_name).unwrap();

        assert_eq!(
            statements,
            vec![
                format!("DROP FUNCTION IF EXISTS `app`.`{validation_name}`;"),
                ensure_semicolon(&validation_source),
                format!("DROP FUNCTION IF EXISTS `app`.`{validation_name}`;"),
                "DROP FUNCTION IF EXISTS `app`.`active_count`;".to_string(),
                "CREATE DEFINER=CURRENT_USER FUNCTION `active_count`() RETURNS INT RETURN 1;".to_string(),
            ]
        );
    }

    #[test]
    fn mysql_alter_routine_source_saves_without_dropping() {
        let statements = build_executable_object_source_statements(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::Procedure,
            schema: Some("app".to_string()),
            name: "refresh_cache".to_string(),
            source: "ALTER PROCEDURE `refresh_cache` COMMENT 'refreshes cache'".to_string(),
        })
        .unwrap();

        assert_eq!(statements, vec!["ALTER PROCEDURE `refresh_cache` COMMENT 'refreshes cache';"]);
    }

    #[test]
    fn mysql_routine_source_opened_for_editing_keeps_create_statement() {
        let sql = build_editable_object_source(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::Procedure,
            schema: Some("app".to_string()),
            name: "refresh_cache".to_string(),
            source: "CREATE PROCEDURE `refresh_cache`() BEGIN SELECT 1; END".to_string(),
        });

        assert_eq!(sql, "CREATE PROCEDURE `refresh_cache`() BEGIN SELECT 1; END;");
    }

    #[test]
    fn mysql_view_source_opened_for_editing_uses_alter_view() {
        let source = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `new_view` AS select `base_plugins`.`id` AS `id` from `base_plugins`";
        let expected = "ALTER ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `new_view` AS select `base_plugins`.`id` AS `id` from `base_plugins`;";
        let mut input = EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::View,
            schema: Some("dol_test".to_string()),
            name: "new_view".to_string(),
            source: source.to_string(),
        };

        let editable = build_editable_object_source(input.clone());
        assert_eq!(editable, expected);
        input.source = editable;
        assert_eq!(build_executable_object_source_sql(input).unwrap(), expected);
    }

    #[test]
    fn mysql_new_view_create_source_remains_create() {
        let source = "CREATE VIEW `new_view` AS SELECT 1 AS `id`";
        let sql = build_executable_object_source_sql(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::View,
            schema: Some("dol_test".to_string()),
            name: "new_view".to_string(),
            source: source.to_string(),
        })
        .unwrap();

        assert_eq!(sql, "CREATE VIEW `new_view` AS SELECT 1 AS `id`;");
    }

    #[test]
    fn mysql_new_view_create_source_preserves_leading_comments() {
        let source =
            "-- keep this view note\n/* and this block */\nCREATE OR REPLACE VIEW `new_view` AS SELECT 1 AS `id`";
        let sql = build_executable_object_source_sql(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::View,
            schema: Some("dol_test".to_string()),
            name: "new_view".to_string(),
            source: source.to_string(),
        })
        .unwrap();

        assert_eq!(
            sql,
            "-- keep this view note\n/* and this block */\nCREATE OR REPLACE VIEW `new_view` AS SELECT 1 AS `id`;"
        );
    }

    #[test]
    fn mysql_view_alter_source_remains_unchanged() {
        let source = "ALTER ALGORITHM=MERGE VIEW `new_view` AS SELECT 2 AS `id`;";
        let sql = build_executable_object_source_sql(EditableObjectSourceSqlInput {
            database_type: DatabaseType::Mysql,
            object_type: ObjectSourceKind::View,
            schema: Some("dol_test".to_string()),
            name: "new_view".to_string(),
            source: source.to_string(),
        })
        .unwrap();

        assert_eq!(sql, source);
    }

    #[test]
    fn mysql_routine_export_uses_delimiter_script() {
        let sql = build_export_object_source_sql(
            DatabaseType::Mysql,
            ObjectSourceKind::Procedure,
            "CREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache`()\nBEGIN\n  SELECT 1;\nEND",
        );

        assert_eq!(
            sql,
            "DELIMITER //\nCREATE DEFINER=`root`@`%` PROCEDURE `refresh_cache`()\nBEGIN\n  SELECT 1;\nEND//\nDELIMITER ;"
        );
    }

    #[test]
    fn mysql_trigger_and_event_export_use_delimiter_script() {
        let trigger = build_export_object_source_sql(
            DatabaseType::Mysql,
            ObjectSourceKind::Trigger,
            "CREATE DEFINER=`root`@`%` TRIGGER `trg_orders_ai` AFTER INSERT ON `orders` FOR EACH ROW\nBEGIN\n  INSERT INTO audit_log(msg) VALUES ('x');\nEND",
        );

        assert_eq!(
            trigger,
            "DELIMITER //\nCREATE DEFINER=`root`@`%` TRIGGER `trg_orders_ai` AFTER INSERT ON `orders` FOR EACH ROW\nBEGIN\n  INSERT INTO audit_log(msg) VALUES ('x');\nEND//\nDELIMITER ;"
        );

        // An event body already carries its schedule, so only the terminator changes.
        let event = build_export_object_source_sql(
            DatabaseType::Mysql,
            ObjectSourceKind::Event,
            "CREATE DEFINER=`root`@`%` EVENT `ev_purge` ON SCHEDULE EVERY 1 DAY DO DELETE FROM audit_log WHERE id < 0;",
        );

        assert_eq!(
            event,
            "DELIMITER //\nCREATE DEFINER=`root`@`%` EVENT `ev_purge` ON SCHEDULE EVERY 1 DAY DO DELETE FROM audit_log WHERE id < 0//\nDELIMITER ;"
        );
    }

    #[test]
    fn mysql_routine_export_does_not_double_wrap_delimiter_script() {
        let source = "DELIMITER //\nCREATE PROCEDURE `refresh_cache`()\nBEGIN\n  SELECT 1;\nEND//\nDELIMITER ;";

        let sql = build_export_object_source_sql(DatabaseType::Mysql, ObjectSourceKind::Procedure, source);

        assert_eq!(sql, source);
    }

    #[test]
    fn mysql_view_export_keeps_regular_statement_terminator() {
        let sql = build_export_object_source_sql(
            DatabaseType::Mysql,
            ObjectSourceKind::View,
            "CREATE VIEW `active_users` AS SELECT `id` FROM `users`",
        );

        assert_eq!(sql, "CREATE VIEW `active_users` AS SELECT `id` FROM `users`;");
    }
}

fn postgres_qualified_name(schema: Option<&str>, name: &str) -> String {
    schema
        .into_iter()
        .chain(std::iter::once(name))
        .filter(|part| !part.is_empty())
        .map(quote_postgres_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn unquote_postgres_identifier(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].replace("\"\"", "\"")
    } else {
        trimmed.to_string()
    }
}

fn quote_postgres_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
