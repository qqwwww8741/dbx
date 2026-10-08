use serde::{Deserialize, Serialize};
use sqlparser::dialect::OracleDialect;
use sqlparser::tokenizer::{Token, Tokenizer};

use crate::models::connection::DatabaseType;

pub const MIN_FUZZY_FILTER_CHARS: usize = 2;

pub fn fuzzy_filter_enabled(filter: &str) -> bool {
    filter.trim().chars().count() >= MIN_FUZZY_FILTER_CHARS
}

pub fn fuzzy_subsequence_match(text: &str, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return true;
    }

    let text = text.to_lowercase();
    let mut chars = text.chars();
    for needle in filter.chars() {
        if !chars.any(|candidate| candidate == needle) {
            return false;
        }
    }
    true
}

pub fn contains_or_fuzzy_match(text: &str, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return true;
    }

    let text = text.to_lowercase();
    text.contains(&filter) || (fuzzy_filter_enabled(&filter) && fuzzy_subsequence_match(&text, &filter))
}

pub fn fuzzy_like_pattern_with_escape(value: &str, mut escape: impl FnMut(&str) -> String) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "%%".to_string();
    }

    let mut pattern = String::with_capacity(value.len() * 2 + 2);
    pattern.push('%');
    for ch in value.chars() {
        pattern.push_str(&escape(&ch.to_string()));
        pattern.push('%');
    }
    pattern
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlFileRequest {
    pub execution_id: String,
    pub connection_id: String,
    pub database: String,
    /// Optional schema/session namespace for databases where it is distinct
    /// from the database used to establish the connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub file_path: String,
    pub continue_on_error: bool,
    /// Reuse a held manual transaction instead of committing through ordinary query execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn_session_id: Option<String>,
    #[serde(default)]
    pub selected_tables: Option<Vec<dbx_types::sql_file::SqlFileTable>>,
    #[serde(default)]
    pub part_cooldown_ms: u64,
    /// Temporarily disable relational constraint enforcement across the entire
    /// target database for this import, and restore it on completion, error,
    /// or cancellation. The mechanism is engine-specific: MySQL-compatible
    /// connections toggle session-scoped `FOREIGN_KEY_CHECKS`; PostgreSQL-family
    /// connections (Postgres, GaussDB, openGauss) run `ALTER TABLE ... DISABLE
    /// TRIGGER ALL` / `ENABLE TRIGGER ALL` for every table; SQL Server runs
    /// `ALTER TABLE ... NOCHECK CONSTRAINT ALL` / `WITH NOCHECK CHECK CONSTRAINT
    /// ALL` for every table. Has no effect on other connection types. This does
    /// not bypass unique/primary-key violations on any engine — only foreign-key
    /// and (on PostgreSQL) other trigger-enforced constraints.
    #[serde(default)]
    pub skip_relational_constraints: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlFilePreview {
    pub file_name: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub preview: String,
    pub can_execute_without_selected_database: bool,
    #[serde(default)]
    pub establishes_database_context: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_file_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_part_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SqlFileStatus {
    Started,
    Running,
    StatementDone,
    StatementFailed,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlFileStatementAction {
    Execute(String),
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlFileImportStatementKind {
    Execute,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlFileImportStatement {
    pub kind: SqlFileImportStatementKind,
    pub sql: String,
    pub source_sqls: Vec<String>,
    pub source_statement_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SqlDialectProfile {
    supports_hash_line_comments: bool,
    /// Whether `/* ... */` comments nest. PostgreSQL and SQL Server document
    /// nesting (`/* /* */ */` needs both closers, so commenting out a block that
    /// already contains a comment keeps the whole block commented); MySQL
    /// documents the opposite ("Nested comments are not supported, and are
    /// deprecated"). A non-nesting scan ends the comment at the inner `*/` and
    /// then hands the commented-out statements to the executor.
    supports_nested_block_comments: bool,
    /// Whether a backslash inside an ordinary `'...'` string escapes the next
    /// character, so `'it\'s'` stays one string.
    ///
    /// This defaults to the historical behaviour (escape) because several
    /// engines — MySQL and ClickHouse among them — do escape there, and the ones
    /// that do not are listed explicitly below. PostgreSQL and its forks are the
    /// engines this repository has evidence for: `standard_conforming_strings` is
    /// on by default, which is also why the generated PostgreSQL SQL writes
    /// newlines as `E'\n'` instead of `'\n'`.
    supports_backslash_escaped_quotes: bool,
    /// PostgreSQL escape string literals (`E'...'`), which keep backslash escapes
    /// even where ordinary `'...'` literals do not.
    supports_postgres_escape_strings: bool,
    /// Oracle alternative quoting: `q'[text]'`, `q'{text}'`, `q'(text)'`,
    /// `q'<text>'` or `q'XtextX'`. The literal exists so that an apostrophe does
    /// not end the string, so reading it as an ordinary `'...'` string ends the
    /// text early and splits the statement at a semicolon inside it.
    supports_oracle_q_quotes: bool,
    supports_oracle_plsql_blocks: bool,
    supports_oracle_style_routine_bodies: bool,
    supports_slash_line_block_delimiter: bool,
    supports_custom_delimiter_commands: bool,
    supports_mysql_routine_blocks: bool,
    supports_dollar_quoted_strings: bool,
    supports_postgres_dollar_quoted_routines: bool,
    supports_hana_do_blocks: bool,
    supports_go_batch_separator: bool,
    keeps_sqlserver_module_batch_at_cursor: bool,
    preserves_tdsql_leading_directives: bool,
    requires_whitespace_after_line_comment_dashes: bool,
    supports_psql_control_commands: bool,
}

impl Default for SqlDialectProfile {
    fn default() -> Self {
        Self {
            supports_hash_line_comments: false,
            supports_nested_block_comments: false,
            supports_oracle_q_quotes: false,
            supports_backslash_escaped_quotes: true,
            supports_postgres_escape_strings: false,
            supports_oracle_plsql_blocks: false,
            supports_oracle_style_routine_bodies: false,
            supports_slash_line_block_delimiter: false,
            supports_custom_delimiter_commands: true,
            supports_mysql_routine_blocks: false,
            supports_dollar_quoted_strings: true,
            supports_postgres_dollar_quoted_routines: false,
            supports_hana_do_blocks: false,
            supports_go_batch_separator: false,
            keeps_sqlserver_module_batch_at_cursor: false,
            preserves_tdsql_leading_directives: false,
            requires_whitespace_after_line_comment_dashes: false,
            supports_psql_control_commands: false,
        }
    }
}

impl SqlDialectProfile {
    fn for_database_type(db_type: DatabaseType) -> Self {
        {
            return Self {
                preserves_tdsql_leading_directives: crate::tdsql_mysql::preserves_leading_directives_for_database_type(
                    db_type,
                ),
                ..Self::mysql_compatible()
            };
        }
    }

    fn mysql_compatible() -> Self {
        Self {
            supports_hash_line_comments: true,
            supports_backslash_escaped_quotes: true,
            supports_mysql_routine_blocks: true,
            requires_whitespace_after_line_comment_dashes: true,
            ..Self::default()
        }
    }

    fn is_mysql_compatible_database(db_type: DatabaseType) -> bool {
        matches!(db_type, DatabaseType::Mysql)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlParsingOptions {
    profile: SqlDialectProfile,
}

impl SqlParsingOptions {
    pub fn for_database_type(db_type: DatabaseType) -> Self {
        Self::from_profile(SqlDialectProfile::for_database_type(db_type))
    }

    pub fn for_database_type_and_compatibility(db_type: DatabaseType, compatibility_mode: Option<&str>) -> Self {
        {}
        Self::for_database_type(db_type)
    }

    pub fn mysql_compatible() -> Self {
        Self::from_profile(SqlDialectProfile::mysql_compatible())
    }

    fn from_profile(profile: SqlDialectProfile) -> Self {
        Self { profile }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlFileProgress {
    pub execution_id: String,
    pub status: SqlFileStatus,
    pub statement_index: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub affected_rows: u64,
    pub elapsed_ms: u128,
    pub statement_summary: String,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SqlFilePhase>,
    /// When processing multiple files, the 0-based index of the current file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_index: Option<usize>,
    /// When processing multiple files, the name of the current file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SqlFilePhase {
    Preparing,
    Reading,
    Executing,
}

pub fn decode_sql_file_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(&bytes[3..]).map(|text| text.to_string()).map_err(|_| sql_file_encoding_error());
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_sql_file_with_encoding(&bytes[2..], encoding_rs::UTF_16LE);
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_sql_file_with_encoding(&bytes[2..], encoding_rs::UTF_16BE);
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.strip_prefix('\u{feff}').unwrap_or(text).to_string());
    }

    decode_sql_file_with_encoding(bytes, encoding_rs::GBK)
}

/// Decode an SQL/text file using an explicit user-selected encoding.
pub fn decode_sql_file_bytes_with_encoding(bytes: &[u8], encoding: &str) -> Result<String, String> {
    match encoding {
        "utf8" => {
            decode_sql_file_with_encoding(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes), encoding_rs::UTF_8)
        }
        "utf8Bom" => {
            let payload = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
            decode_sql_file_with_encoding(payload, encoding_rs::UTF_8)
        }
        "utf16le" => {
            decode_sql_file_with_encoding(bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes), encoding_rs::UTF_16LE)
        }
        "utf16be" => {
            decode_sql_file_with_encoding(bytes.strip_prefix(&[0xFE, 0xFF]).unwrap_or(bytes), encoding_rs::UTF_16BE)
        }
        "gbk" => decode_sql_file_with_encoding(bytes, encoding_rs::GBK),
        "auto" | "" => decode_sql_file_bytes(bytes),
        _ => Err(format!("Unsupported SQL file encoding: {encoding}")),
    }
}

/// Encode editor text for an explicit file encoding. The returned bytes include
/// a BOM for UTF-8 BOM and UTF-16 variants.
pub fn encode_sql_file_text(content: &str, encoding: &str) -> Result<Vec<u8>, String> {
    if encoding == "utf16le" || encoding == "utf16be" {
        let mut bytes = if encoding == "utf16le" { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&if encoding == "utf16le" { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        return Ok(bytes);
    }
    let (encoded, _, had_errors) = match encoding {
        "utf8" | "auto" | "" => encoding_rs::UTF_8.encode(content),
        "utf8Bom" => encoding_rs::UTF_8.encode(content),
        "gbk" => encoding_rs::GBK.encode(content),
        _ => return Err(format!("Unsupported SQL file encoding: {encoding}")),
    };
    if had_errors {
        return Err(sql_file_encoding_error());
    }
    let mut bytes = encoded.into_owned();
    if encoding == "utf8Bom" {
        bytes.splice(0..0, [0xEF, 0xBB, 0xBF]);
    }
    Ok(bytes)
}

fn decode_sql_file_with_encoding(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Result<String, String> {
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    if had_errors {
        return Err(sql_file_encoding_error());
    }
    Ok(text.into_owned())
}

fn sql_file_encoding_error() -> String {
    "Unsupported SQL file encoding. Save the file as UTF-8, UTF-8 with BOM, UTF-16 with BOM, or GBK, then try again."
        .to_string()
}

/// Whether a `--` sequence starts a line comment. MySQL (and MySQL-wire-compatible
/// dialects) requires the second dash to be followed by whitespace, a control character,
/// or end-of-input —
/// `5--1` is subtraction (`5 - -1`), not a comment. Other dialects (Postgres, Oracle/DM,
/// SQL Server, ...) treat `--` as a comment opener unconditionally.
fn dash_dash_starts_line_comment(profile: SqlDialectProfile, char_after_dashes: Option<char>) -> bool {
    if !profile.requires_whitespace_after_line_comment_dashes {
        return true;
    }
    char_after_dashes.is_none_or(|ch| ch.is_whitespace() || ch.is_control())
}

/// Tracks an Oracle `q'...'` literal across characters, so a semicolon inside its
/// text never splits the statement.
#[derive(Default, Clone, Copy)]
struct OracleQQuoteState {
    /// 1 while the introducer's quote is expected, 2 while its delimiter is.
    awaiting: u8,
    closer: Option<char>,
}

impl OracleQQuoteState {
    fn is_open(&self) -> bool {
        self.awaiting > 0 || self.closer.is_some()
    }

    /// Consumes one character of an open literal; `previous` is the character
    /// before it, as tracked by the caller.
    fn consume(&mut self, ch: char, previous: Option<char>) {
        if self.awaiting == 1 {
            self.awaiting = if ch == '\'' { 2 } else { 0 };
        } else if self.awaiting == 2 {
            self.closer = oracle_q_quote_closer(ch);
            self.awaiting = 0;
        } else if let Some(closer) = self.closer {
            if previous == Some(closer) && ch == '\'' {
                self.closer = None;
            }
        }
    }
}

#[derive(Default)]
pub struct SqlStatementSplitter {
    buffer: String,
    in_single_quote: bool,
    /// Whether the open `'...'` literal reads a backslash as escaping the next
    /// character: MySQL strings always do, PostgreSQL only inside `E'...'`.
    single_quote_escape_string: bool,
    in_double_quote: bool,
    in_backtick: bool,
    in_line_comment: bool,
    /// Depth of the open `/* ... */` comment. Engines that nest block comments
    /// increment it for every inner `/*`, so the inner `*/` does not end the
    /// comment and hand commented-out statements to the executor.
    block_comment_depth: usize,
    dollar_quote_tag: Option<String>,
    postgres_dollar_quoted_routine: bool,
    previous: Option<char>,
    pending_mysql_line_comment_dashes: bool,
    /// Open Oracle `q'...'` literal, if any.
    oracle_q_quote: OracleQQuoteState,
    custom_delimiter: Option<String>,
    stop_on_error: bool,
    pending_psql_command: Option<String>,
    options: SqlParsingOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlStatementWithControl {
    pub sql: String,
    pub stop_on_error: bool,
}

impl SqlStatementSplitter {
    pub fn with_options(options: SqlParsingOptions) -> Self {
        Self { options, ..Self::default() }
    }

    pub fn push_chunk(&mut self, chunk: &str) -> Vec<String> {
        self.push_chunk_with_control(chunk).into_iter().map(|statement| statement.sql).collect()
    }

    pub fn push_chunk_with_control(&mut self, chunk: &str) -> Vec<SqlStatementWithControl> {
        let mut statements = Vec::new();
        let chars = chunk.chars().collect::<Vec<_>>();
        let mut i = 0;

        if self.pending_mysql_line_comment_dashes {
            if let Some(first) = chars.first().copied() {
                self.in_line_comment = dash_dash_starts_line_comment(self.options.profile, Some(first));
                self.pending_mysql_line_comment_dashes = false;
            }
        }

        while i < chars.len() {
            if let Some(command) = &mut self.pending_psql_command {
                let ch = chars[i];
                command.push(ch);
                if ch == '\n' {
                    self.finish_psql_command(&mut statements);
                }
                i += 1;
                continue;
            }
            // A psql command consumes the whole line, including quotes, comment
            // markers and semicolons in display text. Buffer it across chunks
            // before letting the SQL scanner interpret those characters.
            if self.options.profile.supports_psql_control_commands
                && chars[i] == '\\'
                && !self.in_single_quote
                && !self.in_double_quote
                && !self.in_backtick
                && !self.in_line_comment
                && self.block_comment_depth == 0
                && self.dollar_quote_tag.is_none()
                && self.buffer.rsplit('\n').next().unwrap_or("").trim().is_empty()
            {
                self.pending_psql_command = Some(String::from("\\"));
                i += 1;
                continue;
            }
            if let Some(tag) = &self.dollar_quote_tag {
                let tag_chars = tag.chars().collect::<Vec<_>>();
                if starts_with_chars(&chars, i, &tag_chars) {
                    for tag_ch in &tag_chars {
                        self.buffer.push(*tag_ch);
                        self.previous = Some(*tag_ch);
                    }
                    i += tag_chars.len();
                    self.dollar_quote_tag = None;
                    continue;
                }

                let ch = chars[i];
                self.buffer.push(ch);
                self.previous = Some(ch);
                i += 1;
                continue;
            }

            let ch = chars[i];
            let next = chars.get(i + 1).copied();

            if self.in_line_comment {
                self.buffer.push(ch);
                if ch == '\n' {
                    self.in_line_comment = false;
                }
                self.previous = Some(ch);
                i += 1;
                continue;
            }

            if self.block_comment_depth > 0 {
                self.buffer.push(ch);
                if self.options.profile.supports_nested_block_comments && self.previous == Some('/') && ch == '*' {
                    self.block_comment_depth += 1;
                    // A nested opener's asterisk must not pair with a following slash as a close.
                    self.previous = None;
                    i += 1;
                    continue;
                } else if self.previous == Some('*') && ch == '/' {
                    self.block_comment_depth -= 1;
                }
                self.previous = Some(ch);
                i += 1;
                continue;
            }

            if self.oracle_q_quote.is_open() {
                self.oracle_q_quote.consume(ch, self.previous);
                self.buffer.push(ch);
                self.previous = Some(ch);
                i += 1;
                continue;
            }

            if !self.in_single_quote && !self.in_double_quote && !self.in_backtick {
                {}
                if self.previous == Some('-') && ch == '-' {
                    if self.options.profile.requires_whitespace_after_line_comment_dashes && next.is_none() {
                        self.pending_mysql_line_comment_dashes = true;
                        self.buffer.push(ch);
                        self.previous = Some(ch);
                        i += 1;
                        continue;
                    }
                    if dash_dash_starts_line_comment(self.options.profile, next) {
                        self.in_line_comment = true;
                        self.buffer.push(ch);
                        self.previous = Some(ch);
                        i += 1;
                        continue;
                    }
                }
                if self.previous == Some('/') && ch == '*' {
                    self.block_comment_depth = 1;
                    self.buffer.push(ch);
                    // The opener's own asterisk must not read as a nested opener.
                    self.previous = None;
                    i += 1;
                    continue;
                }
                if self.options.profile.supports_hash_line_comments && ch == '#' {
                    self.in_line_comment = true;
                    self.buffer.push(ch);
                    self.previous = Some(ch);
                    i += 1;
                    continue;
                }
                if ch == '/' && next == Some('*') {
                    self.block_comment_depth = 1;
                    self.buffer.push(ch);
                    // The opener's own asterisk must not read as a nested opener.
                    self.previous = None;
                    i += 1;
                    continue;
                }
                if let Some(tag) = self
                    .options
                    .profile
                    .supports_dollar_quoted_strings
                    .then(|| dollar_quote_tag_at(&chars, i))
                    .flatten()
                {
                    if self.custom_delimiter.is_none() && !self.on_delimiter_line() {
                        {}
                        for tag_ch in tag.chars() {
                            self.buffer.push(tag_ch);
                            self.previous = Some(tag_ch);
                        }
                        i += tag.chars().count();
                        self.dollar_quote_tag = Some(tag);
                        continue;
                    }
                }
            }

            match ch {
                '\'' if !self.in_double_quote
                    && !self.in_backtick
                    && !(self.single_quote_backslash_escapes() && has_odd_trailing_backslashes(&self.buffer)) =>
                {
                    self.in_single_quote = !self.in_single_quote;
                    self.single_quote_escape_string =
                        self.in_single_quote && (self.options.profile.supports_backslash_escaped_quotes || (false));
                    self.buffer.push(ch);
                }
                '"' if !self.in_single_quote
                    && !self.in_backtick
                    && !(self.options.profile.supports_backslash_escaped_quotes
                        && has_odd_trailing_backslashes(&self.buffer)) =>
                {
                    self.in_double_quote = !self.in_double_quote;
                    self.buffer.push(ch);
                }
                '`' if !self.in_single_quote && !self.in_double_quote => {
                    self.in_backtick = !self.in_backtick;
                    self.buffer.push(ch);
                }
                ';' if !self.in_single_quote && !self.in_double_quote && !self.in_backtick => {
                    if (self.options.profile.supports_custom_delimiter_commands && self.on_delimiter_line())
                        || self.custom_delimiter.is_some()
                    {
                        self.buffer.push(ch);
                    } else if self.options.profile.supports_mysql_routine_blocks
                        && starts_with_mysql_routine_block(&self.buffer)
                    {
                        let mut candidate = self.buffer.clone();
                        candidate.push(ch);
                        if mysql_routine_block_is_complete(&candidate) {
                            // The final semicolon is the client-side statement delimiter.
                            // Keep semicolons inside BEGIN...END, but do not send the
                            // delimiter after END to the MySQL server.
                            self.push_current_statement(&mut statements);
                        } else {
                            self.buffer.push(ch);
                        }
                    } else {
                        self.push_current_statement(&mut statements);
                    }
                }
                _ => self.buffer.push(ch),
            }

            if !self.in_single_quote && !self.in_double_quote && !self.in_backtick && self.dollar_quote_tag.is_none() {
                if ch == '\n' {
                    let buf_end = self.buffer.len() - 1;
                    let last_line_start = self.buffer[..buf_end].rfind('\n').map_or(0, |p| p + 1);
                    let last_line = self.buffer[last_line_start..buf_end].trim();
                    if self.options.profile.supports_slash_line_block_delimiter && last_line == "/" {
                        let before = self.buffer[..last_line_start].trim();
                        if has_executable_sql_with_options(before, self.options) {
                            statements.push(SqlStatementWithControl {
                                sql: before.to_string(),
                                stop_on_error: self.stop_on_error,
                            });
                        }
                        self.buffer.clear();
                        self.postgres_dollar_quoted_routine = false;
                        self.previous = None;
                        i += 1;
                        continue;
                    }
                    if let Some(new_delim) = self
                        .options
                        .profile
                        .supports_custom_delimiter_commands
                        .then(|| {
                            parse_delimiter_command(last_line).filter(|_| {
                                !has_executable_sql_with_options(&self.buffer[..last_line_start], self.options)
                            })
                        })
                        .flatten()
                    {
                        self.custom_delimiter = if new_delim == ";" { None } else { Some(new_delim.to_string()) };
                        if last_line_start > 0 {
                            let before = self.buffer[..last_line_start].trim();
                            if has_executable_sql_with_options(before, self.options) {
                                statements.push(SqlStatementWithControl {
                                    sql: before.to_string(),
                                    stop_on_error: self.stop_on_error,
                                });
                            }
                        }
                        self.buffer.clear();
                        self.postgres_dollar_quoted_routine = false;
                        self.previous = None;
                        i += 1;
                        continue;
                    }
                }
                if let Some(delim) = self.custom_delimiter.clone() {
                    if self.buffer.ends_with(delim.as_str()) {
                        self.buffer.truncate(self.buffer.len() - delim.len());
                        self.push_current_statement(&mut statements);
                    }
                }
            }

            self.previous = Some(ch);
            i += 1;
        }

        statements
    }

    fn finish_psql_command(&mut self, statements: &mut Vec<SqlStatementWithControl>) {
        let command = self.pending_psql_command.take().expect("pending psql command");
        if let Some(control) = parse_ignorable_psql_control_command(command.trim()) {
            self.stop_on_error |= control == PsqlControlCommand::OnErrorStop;
        } else {
            // Unsupported commands must reach the SQL executor and fail visibly;
            // do not silently discard variable assignments or execution commands.
            self.options.profile.supports_psql_control_commands = false;
            statements.extend(self.push_chunk_with_control(&command));
            self.options.profile.supports_psql_control_commands = true;
        }
    }

    pub fn finish(self) -> Vec<String> {
        self.finish_with_control().into_iter().map(|statement| statement.sql).collect()
    }

    pub fn finish_with_control(mut self) -> Vec<SqlStatementWithControl> {
        let mut statements = Vec::new();
        if self.pending_psql_command.is_some() {
            self.finish_psql_command(&mut statements);
        }
        if self.pending_mysql_line_comment_dashes {
            self.in_line_comment = true;
            self.pending_mysql_line_comment_dashes = false;
        }
        let trimmed = self.buffer.trim();
        let last_line = trimmed.rsplit('\n').next().unwrap_or(trimmed).trim();
        let before_last_line = trimmed.rsplit_once('\n').map(|x| x.0).unwrap_or("").trim();
        if self.options.profile.supports_custom_delimiter_commands
            && parse_delimiter_command(last_line)
                .is_some_and(|_| !has_executable_sql_with_options(before_last_line, self.options))
        {
            let before = trimmed.rsplit_once('\n').map(|x| x.0).unwrap_or("").trim();
            if has_executable_sql_with_options(before, self.options) {
                statements.push(SqlStatementWithControl { sql: before.to_string(), stop_on_error: self.stop_on_error });
            }
            self.buffer.clear();
        } else if self.options.profile.supports_slash_line_block_delimiter && last_line == "/" {
            let before = trimmed.rsplit_once('\n').map(|x| x.0).unwrap_or("").trim();
            if has_executable_sql_with_options(before, self.options) {
                statements.push(SqlStatementWithControl { sql: before.to_string(), stop_on_error: self.stop_on_error });
            }
            self.buffer.clear();
        } else if let Some(ref delim) = self.custom_delimiter {
            if self.buffer.ends_with(delim.as_str()) {
                self.buffer.truncate(self.buffer.len() - delim.len());
            }
        }
        self.push_current_statement(&mut statements);
        statements
    }

    pub fn stop_on_error(&self) -> bool {
        self.stop_on_error
    }

    fn push_current_statement(&mut self, statements: &mut Vec<SqlStatementWithControl>) {
        let statement = self.buffer.trim();
        if has_executable_sql_with_options(statement, self.options) {
            statements.push(SqlStatementWithControl { sql: statement.to_string(), stop_on_error: self.stop_on_error });
        }
        self.buffer.clear();
        self.postgres_dollar_quoted_routine = false;
        self.previous = None;
        self.pending_mysql_line_comment_dashes = false;
    }

    /// A backslash escapes a quote only where the dialect says so: MySQL for
    /// ordinary strings, PostgreSQL for `E'...'` literals. Outside a string the
    /// profile decides, which keeps the historical MySQL guard that a quote
    /// preceded by an odd number of backslashes does not open a literal.
    fn single_quote_backslash_escapes(&self) -> bool {
        if self.in_single_quote {
            self.single_quote_escape_string
        } else {
            self.options.profile.supports_backslash_escaped_quotes
        }
    }

    fn on_delimiter_line(&self) -> bool {
        let start = self.buffer.rfind('\n').map_or(0, |p| p + 1);
        let line = self.buffer[start..].trim_start().as_bytes();
        line.len() >= 9
            && line[..9].eq_ignore_ascii_case(b"delimiter")
            && !has_executable_sql_with_options(self.buffer[..start].trim(), self.options)
    }
}

fn has_odd_trailing_backslashes(sql: &str) -> bool {
    sql.as_bytes().iter().rev().take_while(|byte| **byte == b'\\').count() % 2 == 1
}

/// True when `text` ends with a PostgreSQL `E'`/`e'` escape-string introducer.
/// The `E` must start its own token: a type name that merely ends in `e`
/// (`DATE'2020-01-01'`) or an identifier such as `x$e` is not an escape string.
fn ends_with_escape_string_prefix(text: &str) -> bool {
    let mut chars = text.chars().rev();
    if !matches!(chars.next(), Some('E' | 'e')) {
        return false;
    }
    !chars.next().is_some_and(is_identifier_continue_char)
}

/// Whether `ch` can continue an unquoted identifier, which decides whether a
/// one-letter introducer (`E'...'`, `q'...'`) starts its own token.
fn is_identifier_continue_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

pub fn split_sql_statements(sql: &str) -> Vec<String> {
    split_sql_statements_with_options(sql, SqlParsingOptions::default())
}

pub fn split_sql_statements_for_database(sql: &str, db_type: DatabaseType) -> Vec<String> {
    sql_execution_plan_for_database(sql, db_type).statements
}

pub fn split_sql_statements_for_database_with_compatibility(
    sql: &str,
    db_type: DatabaseType,
    compatibility_mode: Option<&str>,
) -> Vec<String> {
    sql_execution_plan_for_database_with_compatibility(sql, db_type, compatibility_mode).statements
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlExecutionPlan {
    pub statements: Vec<String>,
    pub stop_on_error: bool,
}

pub fn sql_execution_plan_for_database(sql: &str, db_type: DatabaseType) -> SqlExecutionPlan {
    sql_execution_plan_with_options(sql, SqlParsingOptions::for_database_type(db_type))
}

pub fn sql_execution_plan_for_database_with_compatibility(
    sql: &str,
    db_type: DatabaseType,
    compatibility_mode: Option<&str>,
) -> SqlExecutionPlan {
    sql_execution_plan_with_options(
        sql,
        SqlParsingOptions::for_database_type_and_compatibility(db_type, compatibility_mode),
    )
}

fn sql_execution_plan_with_options(sql: &str, options: SqlParsingOptions) -> SqlExecutionPlan {
    if !options.profile.supports_psql_control_commands {
        return SqlExecutionPlan { statements: split_sql_statements_with_options(sql, options), stop_on_error: false };
    }

    let (sql, stop_on_error) = preprocess_psql_control_commands(sql, options.profile);
    let mut parsing_options = options;
    parsing_options.profile.supports_psql_control_commands = false;
    SqlExecutionPlan { statements: split_sql_statements_with_options(&sql, parsing_options), stop_on_error }
}

pub fn split_sql_statements_with_options(sql: &str, options: SqlParsingOptions) -> Vec<String> {
    let mut splitter = SqlStatementSplitter::with_options(options);
    let mut statements = splitter.push_chunk(sql);
    statements.extend(splitter.finish());
    statements
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlStatementRange {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

pub fn find_statement_at_cursor(sql: &str, cursor_pos: usize) -> String {
    find_statement_at_cursor_with_options(sql, cursor_pos, SqlParsingOptions::default())
}

pub fn find_statement_at_cursor_for_database(sql: &str, cursor_pos: usize, db_type: DatabaseType) -> String {
    {}
    let mut options = SqlParsingOptions::for_database_type(db_type);
    if options.profile.supports_psql_control_commands {
        let (sql, _) = preprocess_psql_control_commands(sql, options.profile);
        options.profile.supports_psql_control_commands = false;
        return find_statement_at_cursor_with_options(&sql, cursor_pos, options);
    }
    find_statement_at_cursor_with_options(sql, cursor_pos, options)
}

pub fn find_statement_at_cursor_with_options(sql: &str, cursor_pos: usize, options: SqlParsingOptions) -> String {
    let statements = split_sql_statement_ranges_with_options(sql, options);
    let cursor = utf16_offset_to_byte_index(sql, cursor_pos);

    for (idx, statement) in statements.iter().enumerate() {
        if cursor > statement.start && cursor < statement.end {
            return statement_text_at_cursor(sql, statement, cursor, options);
        }

        if cursor == statement.start {
            if cursor_has_sql_after_cursor_on_line(sql, cursor) {
                return statement_text_at_cursor(sql, statement, cursor, options);
            }
            if let Some(prev) = idx.checked_sub(1).and_then(|prev_idx| statements.get(prev_idx)) {
                return statement_text_at_cursor(sql, prev, cursor, options);
            }
            return statement_text_at_cursor(sql, statement, cursor, options);
        }

        if cursor < statement.start {
            if let Some(prev) = idx.checked_sub(1).and_then(|prev_idx| statements.get(prev_idx)) {
                return statement_text_at_cursor(sql, prev, cursor, options);
            }
            return statement_text_at_cursor(sql, statement, cursor, options);
        }
    }

    statements
        .last()
        .map(|statement| statement_text_at_cursor(sql, statement, cursor, options))
        .unwrap_or_else(|| sql.trim().to_string())
}

fn cursor_has_sql_after_cursor_on_line(sql: &str, cursor: usize) -> bool {
    let line_end = sql[cursor..].find('\n').map_or(sql.len(), |offset| cursor + offset);
    sql[cursor..line_end].chars().any(|ch| !ch.is_whitespace())
}

fn statement_text_at_cursor(
    sql: &str,
    statement: &SqlStatementRange,
    cursor: usize,
    options: SqlParsingOptions,
) -> String {
    let soft_ranges = split_statement_range_at_blank_lines(sql, statement, options);
    find_statement_text_in_ranges(sql, &soft_ranges, cursor).unwrap_or_else(|| statement.text.clone())
}

fn find_statement_text_in_ranges(sql: &str, ranges: &[SqlStatementRange], cursor: usize) -> Option<String> {
    for (idx, range) in ranges.iter().enumerate() {
        if cursor > range.start && cursor < range.end {
            return Some(range.text.clone());
        }

        if cursor == range.start {
            if cursor_has_sql_after_cursor_on_line(sql, cursor) {
                return Some(range.text.clone());
            }
            if let Some(prev) = idx.checked_sub(1).and_then(|prev_idx| ranges.get(prev_idx)) {
                return Some(prev.text.clone());
            }
            return Some(range.text.clone());
        }

        if cursor < range.start {
            if let Some(prev) = idx.checked_sub(1).and_then(|prev_idx| ranges.get(prev_idx)) {
                return Some(prev.text.clone());
            }
            return Some(range.text.clone());
        }
    }

    ranges.last().map(|range| range.text.clone())
}

fn split_statement_range_at_blank_lines(
    sql: &str,
    statement: &SqlStatementRange,
    options: SqlParsingOptions,
) -> Vec<SqlStatementRange> {
    {}
    {}

    let mut ranges = Vec::new();
    let mut scanner = SqlScanner::with_profile(options.profile);
    let mut current_start = statement.start;
    let mut line_start = statement.start;
    let mut line_has_non_whitespace = false;
    let mut blank_line_run = 0usize;

    for (relative_idx, ch) in sql[statement.start..statement.end].char_indices() {
        let idx = statement.start + relative_idx;
        if ch == '\n' {
            if !line_has_non_whitespace && !scanner.is_masked() {
                blank_line_run += 1;
            } else {
                blank_line_run = 0;
            }
            scanner.step(sql, idx, ch);
            line_start = idx + ch.len_utf8();
            line_has_non_whitespace = false;
            continue;
        }

        if !line_has_non_whitespace && !ch.is_whitespace() {
            if blank_line_run >= 2
                && !scanner.is_masked()
                && has_executable_sql_with_options(&sql[current_start..line_start], options)
                && starts_with_soft_statement_keyword(&sql[line_start..statement.end], options)
            {
                push_statement_range(&mut ranges, sql, current_start, line_start, options);
                current_start = line_start;
            }
            blank_line_run = 0;
            line_has_non_whitespace = true;
        }

        scanner.step(sql, idx, ch);
    }

    push_statement_range(&mut ranges, sql, current_start, statement.end, options);
    if ranges.is_empty() {
        vec![statement.clone()]
    } else {
        ranges
    }
}

fn starts_with_soft_statement_keyword(sql: &str, options: SqlParsingOptions) -> bool {
    {}
    starts_with_executable_sql_keyword_with_options(
        sql,
        &[
            "CREATE", "ALTER", "DROP", "INSERT", "UPDATE", "DELETE", "MERGE", "REPLACE", "TRUNCATE", "GRANT", "REVOKE",
            "COMMENT", "EXPLAIN", "SHOW", "DESCRIBE", "USE", "SET", "CALL", "EXEC", "EXECUTE", "BEGIN", "COMMIT",
            "ROLLBACK", "DECLARE", "ANALYZE", "VACUUM", "PRAGMA", "REFRESH", "COPY",
        ],
        options,
    )
}

/// Statement ranges that keep their byte offsets into `sql`, so callers can rewrite
/// individual statements in place (see the Oracle administrative DDL tolerance in
/// `sql_analysis`).
pub(crate) fn statement_ranges_for_database(sql: &str, db_type: DatabaseType) -> Vec<SqlStatementRange> {
    split_sql_statement_ranges_with_options(sql, SqlParsingOptions::for_database_type(db_type))
}

fn split_sql_statement_ranges_with_options(sql: &str, options: SqlParsingOptions) -> Vec<SqlStatementRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;
    let mut dollar_quote_tag: Option<String> = None;
    let mut custom_delimiter: Option<String> = None;
    let mut single_quote_escape_string = false;
    let mut postgres_dollar_quoted_routine = false;

    while i < sql.len() {
        if let Some(tag) = &dollar_quote_tag {
            if sql[i..].starts_with(tag) {
                i += tag.len();
                dollar_quote_tag = None;
                continue;
            }
            i += next_char_len(sql, i);
            continue;
        }

        let ch = next_char(sql, i);
        let next = next_char_at(sql, i + ch.len_utf8());

        if in_line_comment {
            i += ch.len_utf8();
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }

        if block_comment_depth > 0 {
            if ch == '*' && next == Some('/') {
                i += 2;
                block_comment_depth -= 1;
            } else if options.profile.supports_nested_block_comments && ch == '/' && next == Some('*') {
                i += 2;
                block_comment_depth += 1;
            } else {
                i += ch.len_utf8();
            }
            continue;
        }

        if !in_single_quote && !in_double_quote && !in_backtick {
            {}
            if ch == '-'
                && next == Some('-')
                && dash_dash_starts_line_comment(options.profile, next_char_at(sql, i + 2))
            {
                in_line_comment = true;
                i += 2;
                continue;
            }
            if options.profile.supports_hash_line_comments && ch == '#' {
                in_line_comment = true;
                i += ch.len_utf8();
                continue;
            }
            if ch == '/' && next == Some('*') {
                block_comment_depth = 1;
                i += 2;
                continue;
            }
            if let Some(tag) =
                options.profile.supports_dollar_quoted_strings.then(|| dollar_quote_tag_at_str(sql, i)).flatten()
            {
                if custom_delimiter.is_none() && !is_on_delimiter_line(sql, start, i, options) {
                    {}
                    i += tag.len();
                    dollar_quote_tag = Some(tag);
                    continue;
                }
            }
            if ch == '\n' {
                let line_start = sql[..i].rfind('\n').map_or(0, |pos| pos + 1);
                let line = sql[line_start..i].trim();
                if options.profile.supports_slash_line_block_delimiter && line == "/" {
                    push_statement_range(&mut ranges, sql, start, line_start, options);
                    start = i + ch.len_utf8();
                    postgres_dollar_quoted_routine = false;
                    i = start;
                    continue;
                }
                if let Some(new_delimiter) = options
                    .profile
                    .supports_custom_delimiter_commands
                    .then(|| {
                        parse_delimiter_command(line)
                            .filter(|_| !has_executable_sql_with_options(&sql[start..line_start], options))
                    })
                    .flatten()
                {
                    let before = sql[start..line_start].trim();
                    if has_executable_sql_with_options(before, options) {
                        push_statement_range(&mut ranges, sql, start, line_start, options);
                    }
                    custom_delimiter = if new_delimiter == ";" { None } else { Some(new_delimiter.to_string()) };
                    start = i + ch.len_utf8();
                    postgres_dollar_quoted_routine = false;
                    i = start;
                    continue;
                }
            }
        }

        let single_quote_backslash_escapes = if in_single_quote {
            single_quote_escape_string
        } else {
            options.profile.supports_backslash_escaped_quotes
        };

        match ch {
            '\'' if !in_double_quote
                && !in_backtick
                && !(single_quote_backslash_escapes && has_odd_trailing_backslashes(&sql[start..i])) =>
            {
                in_single_quote = !in_single_quote;
                single_quote_escape_string =
                    in_single_quote && (options.profile.supports_backslash_escaped_quotes || (false));
                i += ch.len_utf8();
            }
            '"' if !in_single_quote
                && !in_backtick
                && !(options.profile.supports_backslash_escaped_quotes
                    && has_odd_trailing_backslashes(&sql[start..i])) =>
            {
                in_double_quote = !in_double_quote;
                i += ch.len_utf8();
            }
            '`' if !in_single_quote && !in_double_quote => {
                in_backtick = !in_backtick;
                i += ch.len_utf8();
            }
            ';' if !(in_single_quote
                || in_double_quote
                || in_backtick
                || custom_delimiter.is_some()
                || (options.profile.supports_custom_delimiter_commands
                    && is_on_delimiter_line(sql, start, i, options))) =>
            {
                let is_mysql_routine =
                    options.profile.supports_mysql_routine_blocks && starts_with_mysql_routine_block(&sql[start..i]);
                if is_mysql_routine {
                    if !mysql_routine_block_is_complete(&sql[start..i + ch.len_utf8()]) {
                        i += ch.len_utf8();
                        continue;
                    }
                    push_statement_range(&mut ranges, sql, start, i, options);
                } else {
                    let is_oracle_plsql = false;
                    {
                        push_statement_range(&mut ranges, sql, start, i, options);
                    }
                }
                i += ch.len_utf8();
                start = i;
                postgres_dollar_quoted_routine = false;
            }
            _ => {
                i += ch.len_utf8();
                if !in_single_quote && !in_double_quote && !in_backtick {
                    if let Some(delimiter) = &custom_delimiter {
                        if sql[start..i].ends_with(delimiter) {
                            let end = i - delimiter.len();
                            push_statement_range(&mut ranges, sql, start, end, options);
                            start = i;
                            postgres_dollar_quoted_routine = false;
                        }
                    }
                }
            }
        }
    }

    let trimmed = sql[start..].trim();
    let last_line = trimmed.rsplit('\n').next().unwrap_or(trimmed).trim();
    let before_last_line = trimmed.rsplit_once('\n').map(|x| x.0).unwrap_or("").trim();
    if options.profile.supports_custom_delimiter_commands
        && parse_delimiter_command(last_line)
            .is_some_and(|_| !has_executable_sql_with_options(before_last_line, options))
    {
        if let Some(line_start) = sql[start..].rfind('\n').map(|pos| start + pos + 1) {
            push_statement_range(&mut ranges, sql, start, line_start, options);
        }
    } else if options.profile.supports_slash_line_block_delimiter && last_line == "/" {
        if let Some(line_start) = sql[start..].rfind('\n').map(|pos| start + pos + 1) {
            push_statement_range(&mut ranges, sql, start, line_start, options);
        }
    } else {
        push_statement_range(&mut ranges, sql, start, sql.len(), options);
    }

    ranges
}

fn push_statement_range(
    ranges: &mut Vec<SqlStatementRange>,
    sql: &str,
    start: usize,
    end: usize,
    options: SqlParsingOptions,
) {
    let Some((relative_start, relative_end)) = executable_sql_bounds(&sql[start..end], options) else {
        return;
    };
    let statement_start = start + relative_start;
    let statement_end = start + relative_end;
    let text = sql[statement_start..statement_end].to_string();
    if !text.is_empty() {
        ranges.push(SqlStatementRange { text, start: statement_start, end: statement_end });
    }
}

fn utf16_offset_to_byte_index(sql: &str, offset: usize) -> usize {
    let mut utf16_seen = 0;
    for (byte_index, ch) in sql.char_indices() {
        if utf16_seen >= offset {
            return byte_index;
        }
        utf16_seen += ch.len_utf16();
        if utf16_seen > offset {
            return byte_index + ch.len_utf8();
        }
    }
    sql.len()
}

fn next_char(sql: &str, index: usize) -> char {
    sql[index..].chars().next().unwrap_or('\0')
}

fn next_char_at(sql: &str, index: usize) -> Option<char> {
    if index >= sql.len() {
        None
    } else {
        sql[index..].chars().next()
    }
}

fn next_char_len(sql: &str, index: usize) -> usize {
    next_char(sql, index).len_utf8()
}

fn is_on_delimiter_line(sql: &str, range_start: usize, index: usize, options: SqlParsingOptions) -> bool {
    let line_start = sql[range_start..index].rfind('\n').map_or(range_start, |pos| range_start + pos + 1);
    sql[line_start..index].trim_start().as_bytes().get(..9).is_some_and(|prefix| {
        prefix.eq_ignore_ascii_case(b"delimiter")
            && !has_executable_sql_with_options(sql[range_start..line_start].trim(), options)
    })
}

fn dollar_quote_tag_at_str(sql: &str, index: usize) -> Option<String> {
    let rest = &sql[index..];
    if !rest.starts_with('$') {
        return None;
    }
    let end = rest[1..].find('$')? + 1;
    let tag = &rest[..=end];
    if tag.len() == 2 {
        return Some(tag.to_string());
    }
    let name = &tag[1..tag.len() - 1];
    if !name.chars().all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(tag.to_string())
}

pub fn split_sql_batches(sql: &str) -> Vec<String> {
    let ranges = split_sql_batch_ranges(sql, SqlDialectProfile::mysql_compatible());
    if ranges.is_empty() {
        let trimmed = sql.trim();
        return if trimmed.is_empty() { Vec::new() } else { vec![trimmed.to_string()] };
    }
    ranges.into_iter().map(|range| range.text).collect()
}

fn split_sql_batch_ranges(sql: &str, profile: SqlDialectProfile) -> Vec<SqlStatementRange> {
    let mut batches = Vec::new();
    let mut current_start = 0;
    let lines: Vec<&str> = sql.split('\n').collect();
    let mut offset = 0;
    let mut scanner = SqlScanner::with_profile(profile);

    for line in &lines {
        let line_start = offset;
        let line_end = offset + line.len();
        offset = line_end + 1; // +1 for the '\n'

        let trimmed = line.trim();
        let is_batch_separator = profile.supports_go_batch_separator
            && !scanner.is_masked()
            && (trimmed.eq_ignore_ascii_case("go")
                || trimmed.to_ascii_lowercase().starts_with("go ") && trimmed[2..].trim().is_empty());
        if is_batch_separator {
            push_batch_range(&mut batches, sql, current_start, line_start);
            current_start = line_end.min(sql.len());
            if current_start < sql.len() && sql.as_bytes()[current_start] == b'\n' {
                current_start += 1;
            }
        } else {
            for (relative_idx, ch) in line.char_indices() {
                scanner.step(sql, line_start + relative_idx, ch);
            }
        }
        if line_end < sql.len() {
            scanner.step(sql, line_end, '\n');
        }
    }

    push_batch_range(&mut batches, sql, current_start, sql.len());
    batches
}

fn push_batch_range(ranges: &mut Vec<SqlStatementRange>, sql: &str, start: usize, end: usize) {
    let Some((relative_start, relative_end)) = executable_sql_bounds(&sql[start..end], SqlParsingOptions::default())
    else {
        return;
    };
    let statement_start = start + relative_start;
    let statement_end = start + relative_end;
    let text = sql[statement_start..statement_end].to_string();
    if !text.is_empty() {
        ranges.push(SqlStatementRange { text, start: statement_start, end: statement_end });
    }
}

pub fn starts_with_sqlserver_module_ddl(sql: &str) -> bool {
    false
}

fn starts_with_mysql_routine_block(sql: &str) -> bool {
    is_mysql_routine_ddl_start(sql) && mysql_routine_tokens(sql).iter().any(|token| token.eq_ignore_ascii_case("BEGIN"))
}

fn is_mysql_routine_ddl_start(sql: &str) -> bool {
    let executable = leading_executable_sql_with_options(sql, SqlParsingOptions::mysql_compatible());
    let tokens = first_sql_tokens(executable, 16);
    if tokens.first().is_none_or(|token| !token.eq_ignore_ascii_case("CREATE")) {
        return false;
    }

    for token in tokens.iter().skip(1) {
        if ["PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"].iter().any(|keyword| token.eq_ignore_ascii_case(keyword)) {
            return true;
        }
        if [
            "DATABASE",
            "INDEX",
            "LOGFILE",
            "ROLE",
            "SCHEMA",
            "SERVER",
            "SPATIAL",
            "TABLE",
            "TEMPORARY",
            "UNIQUE",
            "USER",
            "VIEW",
        ]
        .iter()
        .any(|keyword| token.eq_ignore_ascii_case(keyword))
        {
            return false;
        }
    }

    false
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MysqlRoutineBlockType {
    Begin,
    Case,
}

fn mysql_routine_block_is_complete(sql: &str) -> bool {
    if !starts_with_mysql_routine_block(sql) {
        return false;
    }

    let tokens = mysql_routine_tokens(sql);
    let mut block_stack = Vec::new();
    let mut saw_begin = false;

    for (index, token) in tokens.iter().enumerate() {
        if token == ";" {
            continue;
        }
        if token.eq_ignore_ascii_case("BEGIN") {
            if previous_mysql_routine_word(&tokens, index).is_some_and(|previous| previous.eq_ignore_ascii_case("END"))
            {
                continue;
            }
            saw_begin = true;
            block_stack.push(MysqlRoutineBlockType::Begin);
            continue;
        }
        if token.eq_ignore_ascii_case("CASE") {
            if previous_mysql_routine_word(&tokens, index).is_some_and(|previous| previous.eq_ignore_ascii_case("END"))
            {
                continue;
            }
            block_stack.push(MysqlRoutineBlockType::Case);
            continue;
        }
        if token.eq_ignore_ascii_case("END") && saw_begin {
            let suffix = next_mysql_routine_word(&tokens, index);
            if suffix.is_some_and(|next| next.eq_ignore_ascii_case("CASE")) {
                if block_stack.last() == Some(&MysqlRoutineBlockType::Case) {
                    block_stack.pop();
                }
                continue;
            }
            if suffix.is_some_and(is_mysql_control_block_suffix) {
                continue;
            }
            block_stack.pop();
        }
    }

    saw_begin && block_stack.is_empty() && tokens.last().is_some_and(|token| token == ";")
}

fn is_mysql_control_block_suffix(token: &str) -> bool {
    ["IF", "LOOP", "CASE", "REPEAT", "WHILE"].iter().any(|keyword| token.eq_ignore_ascii_case(keyword))
}

fn previous_mysql_routine_word(tokens: &[String], index: usize) -> Option<&str> {
    tokens[..index].iter().rev().find(|token| token.as_str() != ";").map(String::as_str)
}

fn next_mysql_routine_word(tokens: &[String], index: usize) -> Option<&str> {
    tokens.get(index + 1..)?.iter().find(|token| token.as_str() != ";").map(String::as_str)
}

fn mysql_routine_tokens(sql: &str) -> Vec<String> {
    let chars = sql.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            // MySQL block comments do not nest, so a single flag is enough here.
            if ch == '*' && next == Some('/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if in_single_quote {
            if ch == '\\' && next.is_some() {
                i += 2;
                continue;
            }
            if ch == '\'' {
                if next == Some('\'') {
                    i += 2;
                    continue;
                }
                in_single_quote = false;
            }
            i += 1;
            continue;
        }

        if in_double_quote {
            if ch == '\\' && next.is_some() {
                i += 2;
                continue;
            }
            if ch == '"' {
                if next == Some('"') {
                    i += 2;
                    continue;
                }
                in_double_quote = false;
            }
            i += 1;
            continue;
        }

        if in_backtick {
            if ch == '`' {
                if next == Some('`') {
                    i += 2;
                    continue;
                }
                in_backtick = false;
            }
            i += 1;
            continue;
        }

        if ch == '-' && next == Some('-') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if ch == '#' {
            in_line_comment = true;
            i += 1;
            continue;
        }
        if ch == '/' && next == Some('*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if ch == '\'' {
            in_single_quote = true;
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double_quote = true;
            i += 1;
            continue;
        }
        if ch == '`' {
            in_backtick = true;
            i += 1;
            continue;
        }
        if ch == ';' {
            tokens.push(";".to_string());
            i += 1;
            continue;
        }
        if ch == '_' || ch.is_ascii_alphabetic() {
            let start = i;
            i += 1;
            while i < chars.len() && is_sql_ident_char(chars[i]) {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect::<String>().to_ascii_uppercase());
            continue;
        }

        i += 1;
    }

    tokens
}

fn first_sql_tokens(sql: &str, limit: usize) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            for comment_char in chars.by_ref() {
                if comment_char == '\n' {
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            let mut previous = None;
            for comment_char in chars.by_ref() {
                if previous == Some('*') && comment_char == '/' {
                    break;
                }
                previous = Some(comment_char);
            }
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            token.push(ch);
        } else if !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
        }

        if tokens.len() >= limit {
            break;
        }
    }

    if tokens.len() < limit && !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn parse_delimiter_command(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.eq_ignore_ascii_case("delimiter;") {
        return Some(";");
    }
    let bytes = line.as_bytes();
    let rest = if bytes.len() > 10
        && (bytes[..10].eq_ignore_ascii_case(b"delimiter ") || bytes[..10].eq_ignore_ascii_case(b"delimiter\t"))
    {
        Some(&line[10..])
    } else {
        None
    };
    rest.map(|r| r.trim()).filter(|r| !r.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PsqlControlCommand {
    OnErrorStop,
    ClientDisplay,
}

fn parse_ignorable_psql_control_command(line: &str) -> Option<PsqlControlCommand> {
    let mut parts = line.split_whitespace();
    let command = parts.next()?;

    if command.eq_ignore_ascii_case("\\echo") {
        // Display-only output can be omitted, but another meta-command on the
        // same line or psql's backtick command substitution must not be ignored.
        let arguments = &line[line.find(command)? + command.len()..];
        return (!arguments.contains(['\\', '`'])).then_some(PsqlControlCommand::ClientDisplay);
    }

    if command.eq_ignore_ascii_case("\\timing") {
        let valid =
            parts.next().is_none_or(|value| value.eq_ignore_ascii_case("on") || value.eq_ignore_ascii_case("off"));
        return (valid && parts.next().is_none()).then_some(PsqlControlCommand::ClientDisplay);
    }

    if !command.eq_ignore_ascii_case("\\set") {
        return None;
    }
    let variable = parts.next()?;
    let value = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    match variable.to_ascii_uppercase().as_str() {
        "ON_ERROR_STOP" if matches!(value.to_ascii_lowercase().as_str(), "on" | "true" | "1") => {
            Some(PsqlControlCommand::OnErrorStop)
        }
        "VERBOSITY" if matches!(value.to_ascii_lowercase().as_str(), "default" | "verbose" | "terse" | "sqlstate") => {
            Some(PsqlControlCommand::ClientDisplay)
        }
        _ => None,
    }
}

fn preprocess_psql_control_commands(sql: &str, profile: SqlDialectProfile) -> (String, bool) {
    let mut output = String::with_capacity(sql.len());
    let mut scanner = SqlScanner::with_profile(profile);
    let mut offset = 0usize;
    let mut stop_on_error = false;

    for segment in sql.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let command = (!scanner.is_masked()).then(|| parse_ignorable_psql_control_command(line.trim())).flatten();
        if let Some(command) = command {
            stop_on_error |= command == PsqlControlCommand::OnErrorStop;
            // Cursor positions arrive as UTF-16 offsets from the editor. Keep
            // those offsets stable even when display text contains emoji.
            for ch in line.chars() {
                if ch.is_whitespace() {
                    output.push(ch);
                } else {
                    output.extend(std::iter::repeat_n(' ', ch.len_utf16()));
                }
            }
            if segment.ends_with('\n') {
                output.push('\n');
                scanner.step(sql, offset + line.len(), '\n');
            }
        } else {
            output.push_str(segment);
            for (relative_idx, ch) in segment.char_indices() {
                scanner.step(sql, offset + relative_idx, ch);
            }
        }
        offset += segment.len();
    }

    (output, stop_on_error)
}

pub fn statement_summary(statement: &str) -> String {
    const MAX_LEN: usize = 120;

    let collapsed = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_LEN {
        return collapsed;
    }

    collapsed.chars().take(MAX_LEN).collect()
}

pub fn prepare_sql_file_statement(
    statement: &str,
    db_type: &DatabaseType,
    driver_profile: Option<&str>,
) -> SqlFileStatementAction {
    let statement = statement.trim();
    if statement.is_empty() || (false) {
        return SqlFileStatementAction::Skip;
    }

    let is_mysql_compatible_target = is_mysql_compatible_import_target(db_type, driver_profile);
    if is_mysql_compatible_target && is_mysql_lock_table_statement(statement) {
        return SqlFileStatementAction::Skip;
    }

    let Some(body) = mysql_executable_comment_body(statement) else {
        if is_mysql_compatible_target && is_mysql_session_restore_statement(statement) {
            return SqlFileStatementAction::Skip;
        }
        return SqlFileStatementAction::Execute(statement.to_string());
    };

    if !is_mysql_compatible_target {
        return SqlFileStatementAction::Skip;
    }

    let body = body.trim();
    if body.is_empty() || is_mysql_key_toggle_statement(body) || is_mysql_session_restore_statement(body) {
        return SqlFileStatementAction::Skip;
    }

    SqlFileStatementAction::Execute(body.to_string())
}

pub fn optimize_sql_file_import_statements(
    statements: &[String],
    db_type: Option<DatabaseType>,
    driver_profile: Option<&str>,
) -> Vec<SqlFileImportStatement> {
    optimize_sql_file_import_statements_with_max_insert_batch_statements(
        statements,
        db_type,
        driver_profile,
        SQL_FILE_INSERT_BATCH_MAX_STATEMENTS,
    )
}

pub fn optimize_sql_file_import_statements_with_max_insert_batch_statements(
    statements: &[String],
    db_type: Option<DatabaseType>,
    driver_profile: Option<&str>,
    max_insert_batch_statements: usize,
) -> Vec<SqlFileImportStatement> {
    let mut optimized = Vec::new();
    let mut pending_insert: Option<PendingInsertBatch> = None;
    let merge_adjacent_inserts =
        db_type.as_ref().is_none_or(|db_type| !is_mysql_compatible_import_target(db_type, driver_profile));

    for statement in statements {
        let action = db_type
            .as_ref()
            .map(|db_type| prepare_sql_file_statement(statement, db_type, driver_profile))
            .unwrap_or_else(|| SqlFileStatementAction::Execute(statement.trim().to_string()));

        match action {
            SqlFileStatementAction::Skip => {
                flush_pending_insert(&mut optimized, &mut pending_insert);
                optimized.push(SqlFileImportStatement {
                    kind: SqlFileImportStatementKind::Skip,
                    sql: statement.trim().to_string(),
                    source_sqls: vec![statement.trim().to_string()],
                    source_statement_count: 1,
                });
            }
            SqlFileStatementAction::Execute(sql) => {
                // Combining separate MySQL INSERT statements changes session
                // semantics such as LAST_INSERT_ID(), so execute them with the
                // same statement boundaries as the source file.
                let mergeable_insert = merge_adjacent_inserts
                    .then(|| {
                        let options = db_type.map(SqlParsingOptions::for_database_type).unwrap_or_default();
                        parse_mergeable_insert(&sql, options)
                    })
                    .flatten();
                if let Some(insert) = mergeable_insert {
                    match pending_insert.as_mut() {
                        Some(batch) if batch.can_accept(&insert, max_insert_batch_statements) => batch.push(insert),
                        Some(_) => {
                            flush_pending_insert(&mut optimized, &mut pending_insert);
                            pending_insert = Some(PendingInsertBatch::new(insert));
                        }
                        None => {
                            pending_insert = Some(PendingInsertBatch::new(insert));
                        }
                    }
                } else {
                    flush_pending_insert(&mut optimized, &mut pending_insert);
                    optimized.push(SqlFileImportStatement {
                        kind: SqlFileImportStatementKind::Execute,
                        sql: sql.clone(),
                        source_sqls: vec![sql],
                        source_statement_count: 1,
                    });
                }
            }
        }
    }

    flush_pending_insert(&mut optimized, &mut pending_insert);
    optimized
}

fn flush_pending_insert(optimized: &mut Vec<SqlFileImportStatement>, pending_insert: &mut Option<PendingInsertBatch>) {
    if let Some(batch) = pending_insert.take() {
        optimized.push(SqlFileImportStatement {
            kind: SqlFileImportStatementKind::Execute,
            sql: batch.to_sql(),
            source_sqls: batch.source_sqls,
            source_statement_count: batch.source_statement_count,
        });
    }
}

const SQL_FILE_INSERT_BATCH_MAX_STATEMENTS: usize = 500;
const SQL_FILE_INSERT_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
struct MergeableInsert {
    prefix: String,
    prefix_key: String,
    values: String,
    sql: String,
}

#[derive(Debug, Clone)]
struct PendingInsertBatch {
    prefix: String,
    prefix_key: String,
    values: Vec<String>,
    source_sqls: Vec<String>,
    source_statement_count: usize,
    byte_len: usize,
}

impl PendingInsertBatch {
    fn new(insert: MergeableInsert) -> Self {
        let byte_len = insert.prefix.len() + insert.values.len() + 16;
        Self {
            prefix: insert.prefix,
            prefix_key: insert.prefix_key,
            values: vec![insert.values],
            source_sqls: vec![insert.sql],
            source_statement_count: 1,
            byte_len,
        }
    }

    fn can_accept(&self, insert: &MergeableInsert, max_insert_batch_statements: usize) -> bool {
        self.prefix_key == insert.prefix_key
            && self.source_statement_count < max_insert_batch_statements
            && self.byte_len + insert.values.len() + 3 <= SQL_FILE_INSERT_BATCH_MAX_BYTES
    }

    fn push(&mut self, insert: MergeableInsert) {
        self.byte_len += insert.values.len() + 3;
        self.values.push(insert.values);
        self.source_sqls.push(insert.sql);
        self.source_statement_count += 1;
    }

    fn to_sql(&self) -> String {
        if self.source_statement_count == 1 {
            return self.source_sqls.first().cloned().unwrap_or_default();
        }
        format!("{} VALUES\n{}", self.prefix, self.values.join(",\n"))
    }
}

fn parse_mergeable_insert(sql: &str, options: SqlParsingOptions) -> Option<MergeableInsert> {
    let executable = leading_executable_sql_with_options(sql, options).trim().trim_end_matches(';').trim();
    if !starts_with_keyword(executable, "insert") {
        return None;
    }

    let (values_start, values_end) = find_top_level_values_keyword(executable)?;
    let prefix_without_values = executable[..values_start].trim_end();
    let values = executable[values_end..].trim();
    let values = parse_insert_values_tail(values)?;

    let prefix = prefix_without_values.to_string();
    Some(MergeableInsert {
        prefix_key: normalize_insert_prefix_key(&prefix),
        prefix,
        values,
        sql: executable.to_string(),
    })
}

fn find_top_level_values_keyword(sql: &str) -> Option<(usize, usize)> {
    let mut scanner = SqlScanner::default();
    let mut depth = 0usize;

    for (idx, ch) in sql.char_indices() {
        scanner.step(sql, idx, ch);
        if scanner.is_masked() {
            continue;
        }

        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                for keyword in ["values", "value"] {
                    if keyword_at(sql, idx, keyword) {
                        return Some((idx, idx + keyword.len()));
                    }
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_insert_values_tail(tail: &str) -> Option<String> {
    let tail = tail.trim().trim_end_matches(';').trim();
    if tail.is_empty() {
        return None;
    }

    let mut scanner = SqlScanner::default();
    let mut depth = 0usize;
    let mut saw_tuple = false;
    let mut expecting_tuple = true;

    for (idx, ch) in tail.char_indices() {
        scanner.step(tail, idx, ch);
        if scanner.is_masked() {
            continue;
        }

        if expecting_tuple {
            if ch.is_whitespace() || (saw_tuple && ch == ',') {
                continue;
            }
            if ch != '(' {
                return None;
            }
            expecting_tuple = false;
            saw_tuple = true;
            depth = 1;
            continue;
        }

        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    expecting_tuple = true;
                }
            }
            ',' if depth == 0 => {}
            _ if depth == 0 && !ch.is_whitespace() => return None,
            _ => {}
        }
    }

    if saw_tuple && depth == 0 {
        Some(tail.to_string())
    } else {
        None
    }
}

#[derive(Default)]
struct SqlScanner {
    profile: SqlDialectProfile,
    in_single_quote: bool,
    in_double_quote: bool,
    in_backtick: bool,
    in_line_comment: bool,
    block_comment_depth: usize,
    oracle_q_quote: OracleQQuoteState,
    dollar_quote_tag: Option<String>,
    previous: Option<char>,
}

impl SqlScanner {
    fn with_profile(profile: SqlDialectProfile) -> Self {
        Self { profile, ..Self::default() }
    }

    fn step(&mut self, sql: &str, idx: usize, ch: char) {
        if let Some(tag) = self.dollar_quote_tag.clone() {
            if sql[idx..].starts_with(&tag) {
                self.dollar_quote_tag = None;
            }
            self.previous = Some(ch);
            return;
        }

        let next = next_char_at(sql, idx + ch.len_utf8());
        if self.in_line_comment {
            if ch == '\n' {
                self.in_line_comment = false;
            }
            self.previous = Some(ch);
            return;
        }
        if self.block_comment_depth > 0 {
            if self.profile.supports_nested_block_comments && self.previous == Some('/') && ch == '*' {
                self.block_comment_depth += 1;
                // A nested opener's asterisk must not pair with a following slash as a close.
                self.previous = None;
                return;
            } else if self.previous == Some('*') && ch == '/' {
                self.block_comment_depth -= 1;
            }
            self.previous = Some(ch);
            return;
        }

        if self.oracle_q_quote.is_open() {
            self.oracle_q_quote.consume(ch, self.previous);
            self.previous = Some(ch);
            return;
        }

        if !self.in_single_quote && !self.in_double_quote && !self.in_backtick {
            {}
            if (ch == '-' && next == Some('-')) || (self.profile.supports_hash_line_comments && ch == '#') {
                self.in_line_comment = true;
            } else if ch == '/' && next == Some('*') {
                self.block_comment_depth = 1;
                // The opener's own asterisk must not read as a nested opener.
                self.previous = None;
                return;
            } else if let Some(tag) =
                self.profile.supports_dollar_quoted_strings.then(|| dollar_quote_tag_at_str(sql, idx)).flatten()
            {
                self.dollar_quote_tag = Some(tag);
            }
        }

        match ch {
            '\'' if !self.in_double_quote && !self.in_backtick && self.previous != Some('\\') => {
                self.in_single_quote = !self.in_single_quote;
            }
            '"' if !self.in_single_quote && !self.in_backtick && self.previous != Some('\\') => {
                self.in_double_quote = !self.in_double_quote;
            }
            '`' if !self.in_single_quote && !self.in_double_quote => {
                self.in_backtick = !self.in_backtick;
            }
            _ => {}
        }
        self.previous = Some(ch);
    }

    fn is_masked(&self) -> bool {
        self.in_single_quote
            || self.in_double_quote
            || self.in_backtick
            || self.in_line_comment
            || self.block_comment_depth > 0
            || self.oracle_q_quote.is_open()
            || self.dollar_quote_tag.is_some()
    }
}

fn keyword_at(sql: &str, idx: usize, keyword: &str) -> bool {
    let end = idx + keyword.len();
    sql.get(idx..end).is_some_and(|candidate| candidate.eq_ignore_ascii_case(keyword))
        && sql[..idx].chars().next_back().is_none_or(|ch| !is_sql_ident_char(ch))
        && sql.get(end..).and_then(|tail| tail.chars().next()).is_none_or(|ch| !is_sql_ident_char(ch))
}

fn starts_with_keyword(sql: &str, keyword: &str) -> bool {
    sql.get(..keyword.len()).is_some_and(|candidate| candidate.eq_ignore_ascii_case(keyword))
        && sql.get(keyword.len()..).and_then(|tail| tail.chars().next()).is_none_or(|ch| !is_sql_ident_char(ch))
}

fn is_sql_ident_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()
}

fn normalize_insert_prefix_key(prefix: &str) -> String {
    let mut scanner = SqlScanner::default();
    let mut key = String::with_capacity(prefix.len());
    let mut previous_space = false;

    for (idx, ch) in prefix.char_indices() {
        scanner.step(prefix, idx, ch);
        if scanner.in_single_quote
            || scanner.in_double_quote
            || scanner.in_backtick
            || scanner.dollar_quote_tag.is_some()
        {
            key.push(ch);
            previous_space = false;
            continue;
        }

        if ch.is_whitespace() {
            if !previous_space {
                key.push(' ');
            }
            previous_space = true;
        } else {
            key.push(ch.to_ascii_lowercase());
            previous_space = false;
        }
    }

    key.trim().to_string()
}

pub fn starts_with_executable_sql_keyword(sql: &str, keywords: &[&str]) -> bool {
    starts_with_executable_sql_keyword_with_options(sql, keywords, SqlParsingOptions::default())
}

pub fn starts_with_executable_sql_keyword_for_database(sql: &str, keywords: &[&str], db_type: DatabaseType) -> bool {
    starts_with_executable_sql_keyword_with_options(sql, keywords, SqlParsingOptions::for_database_type(db_type))
}

pub fn starts_with_duckdb_result_sql_keyword(sql: &str) -> bool {
    false
}

pub fn starts_with_executable_sql_keyword_with_options(
    sql: &str,
    keywords: &[&str],
    options: SqlParsingOptions,
) -> bool {
    let Some(token) = first_executable_sql_token_with_options(sql, options) else {
        return false;
    };
    keywords.iter().any(|keyword| executable_sql_keyword_matches(token, keyword))
}

fn executable_sql_keyword_matches(token: &str, keyword: &str) -> bool {
    token.eq_ignore_ascii_case(keyword)
        || (keyword.eq_ignore_ascii_case("DESCRIBE") && token.eq_ignore_ascii_case("DESC"))
}

pub fn supports_connection_level_database_bootstrap_target(
    db_type: &DatabaseType,
    driver_profile: Option<&str>,
) -> bool {
    true
}

pub fn is_mysql_compatible_import_target(db_type: &DatabaseType, driver_profile: Option<&str>) -> bool {
    true
}

fn mysql_executable_comment_body(statement: &str) -> Option<&str> {
    let bytes = statement.as_bytes();
    let start = leading_mysql_executable_comment_start(statement)?;
    let body_start = if bytes.get(start + 2) == Some(&b'!') { start + 3 } else { start + 4 };
    let mut body_start = body_start;
    while body_start < bytes.len() && (bytes[body_start].is_ascii_digit() || bytes[body_start].is_ascii_whitespace()) {
        body_start += 1;
    }

    let close = find_block_comment_close(bytes, body_start, false)?;
    if has_executable_sql(&statement[close + 2..]) {
        return None;
    }

    Some(&statement[body_start..close])
}

fn leading_mysql_executable_comment_start(statement: &str) -> Option<usize> {
    let bytes = statement.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'#' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if i + 2 < bytes.len() && (bytes[i + 2] == b'!' || (i + 3 < bytes.len() && &bytes[i + 2..i + 4] == b"M!")) {
                return Some(i);
            }

            let close = find_block_comment_close(bytes, i + 2, false)?;
            i = close + 2;
            continue;
        }

        return None;
    }

    None
}

fn find_block_comment_close(bytes: &[u8], mut start: usize, nested: bool) -> Option<usize> {
    let mut depth = 1usize;
    while start + 1 < bytes.len() {
        if bytes[start] == b'*' && bytes[start + 1] == b'/' {
            depth -= 1;
            if depth == 0 {
                return Some(start);
            }
            start += 2;
            continue;
        }
        if nested && bytes[start] == b'/' && bytes[start + 1] == b'*' {
            depth += 1;
            start += 2;
            continue;
        }
        start += 1;
    }
    None
}

fn is_mysql_key_toggle_statement(statement: &str) -> bool {
    let upper = statement.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_uppercase();
    upper.starts_with("ALTER TABLE ") && (upper.ends_with(" ENABLE KEYS") || upper.ends_with(" DISABLE KEYS"))
}

fn is_mysql_lock_table_statement(statement: &str) -> bool {
    let executable = leading_executable_sql(statement);
    let upper = executable.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_uppercase();
    upper == "UNLOCK TABLES" || (upper.starts_with("LOCK TABLES ") && upper.ends_with(" WRITE"))
}

fn is_mysql_session_restore_statement(statement: &str) -> bool {
    let executable = leading_executable_sql_with_options(statement, SqlParsingOptions::mysql_compatible());
    let upper = executable.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_uppercase();
    if !upper.starts_with("SET ") {
        return false;
    }

    let assignment = upper.trim_start_matches("SET ").trim();
    if assignment.starts_with('@') {
        return false;
    }

    assignment.contains("= @OLD_")
        || assignment.contains("=@OLD_")
        || assignment.contains("= @SAVED_")
        || assignment.contains("=@SAVED_")
}

fn leading_executable_sql(sql: &str) -> &str {
    leading_executable_sql_with_options(sql, SqlParsingOptions::default())
}

fn leading_executable_sql_with_options(sql: &str, options: SqlParsingOptions) -> &str {
    let bytes = sql.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        if i >= bytes.len() {
            break;
        }

        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if options.profile.supports_hash_line_comments && bytes[i] == b'#' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if i + 2 < bytes.len() && (bytes[i + 2] == b'!' || (i + 3 < bytes.len() && &bytes[i + 2..i + 4] == b"M!")) {
                break;
            }

            let Some(close) = find_block_comment_close(bytes, i + 2, options.profile.supports_nested_block_comments)
            else {
                return &sql[sql.len()..];
            };
            i = close + 2;
            continue;
        }

        break;
    }

    &sql[i..]
}

fn first_executable_sql_token_with_options(sql: &str, options: SqlParsingOptions) -> Option<&str> {
    let bytes = sql.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'(') {
            i += 1;
        }

        if i >= bytes.len() {
            break;
        }

        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if options.profile.supports_hash_line_comments && bytes[i] == b'#' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if i + 2 < bytes.len() && (bytes[i + 2] == b'!' || (i + 3 < bytes.len() && &bytes[i + 2..i + 4] == b"M!")) {
                i += if bytes[i + 2] == b'!' { 3 } else { 4 };
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i].is_ascii_whitespace()) {
                    i += 1;
                }
                break;
            }

            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        break;
    }

    let start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        i += 1;
    }

    (i > start).then_some(&sql[start..i])
}

fn starts_with_oracle_style_routine_body(sql: &str) -> bool {
    false
}

struct HanaDoBlock {
    tokens: Vec<OraclePlSqlToken>,
}

impl HanaDoBlock {
    fn parse(sql: &str) -> Self {
        Self { tokens: oracle_plsql_tokens(sql) }
    }

    fn starts_block(&self) -> bool {
        self.tokens.first().is_some_and(|token| token.is_word("DO"))
    }

    fn is_complete(&self) -> bool {
        if !self.starts_block() {
            return false;
        }

        let mut stack: Vec<String> = Vec::new();
        let mut saw_begin = false;

        for (index, token) in self.tokens.iter().enumerate() {
            if token.is_word("BEGIN") {
                if previous_word_token(&self.tokens, index).is_some_and(|previous| previous == "END") {
                    continue;
                }
                stack.push("BLOCK".to_string());
                saw_begin = true;
                continue;
            }
            if token.is_any_word(&["IF", "FOR", "WHILE", "CASE"]) {
                if previous_word_token(&self.tokens, index).is_none_or(|previous| previous != "END") {
                    stack.push(token.as_word().unwrap_or("BLOCK").to_string());
                }
                continue;
            }
            if token.is_word("END") {
                let next = next_word_token(&self.tokens, index);
                let top = stack.last().map(|value| value.as_str());
                let target = match next {
                    Some(keyword @ ("IF" | "FOR" | "WHILE")) => keyword,
                    _ if top == Some("CASE") => "CASE",
                    _ => "BLOCK",
                };
                if top == Some(target) {
                    stack.pop();
                }
            }
        }

        saw_begin && stack.is_empty() && self.tokens.last().is_some_and(OraclePlSqlToken::is_semicolon)
    }
}

struct OraclePlSqlBlock {
    tokens: Vec<OraclePlSqlToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OraclePlSqlToken {
    Word(String),
    /// A double-quoted identifier (`"schema"."procedure"`). Kept as its own variant
    /// instead of a [`Self::Word`]: the block detectors compare keywords, and a quoted
    /// name must never satisfy `is_word` (a column called `"END"` is not the block end).
    /// Dropping it entirely made `BEGIN "S"."P"(); END;` tokenize exactly like a
    /// transaction `BEGIN;`, so the splitter cut the block at the inner semicolon and
    /// the server rejected the truncated statement (#10434).
    QuotedIdentifier,
    Semicolon,
}

impl OraclePlSqlBlock {
    fn parse(sql: &str) -> Self {
        Self { tokens: oracle_plsql_tokens(sql) }
    }

    fn starts_block(&self) -> bool {
        match self.tokens.as_slice() {
            [first, ..] if first.is_word("DECLARE") => true,
            [first, second, ..] if first.is_word("BEGIN") && !Self::is_transaction_begin_tail(second) => true,
            [first, rest @ ..] if first.is_word("CREATE") => Self::starts_create_plsql_object(rest),
            _ => false,
        }
    }

    fn is_complete(&self) -> bool {
        if !self.starts_block() {
            return false;
        }

        // Package/type specifications have declarations and an outer END with
        // no BEGIN. Bodies also own an outer END beyond nested routine END
        // pairs so an inner END cannot finish the object.
        let object_kind = self.create_object_kind();
        let mut scopes = match object_kind {
            Some(OraclePlSqlCreateObjectKind::Body | OraclePlSqlCreateObjectKind::Spec) => {
                vec![OraclePlSqlScope::Object]
            }
            None => Vec::new(),
        };
        let mut saw_begin = false;
        let mut complete = false;

        for (index, token) in self.tokens.iter().enumerate() {
            if token.is_semicolon() {
                if matches!(scopes.last(), Some(OraclePlSqlScope::RoutineHeader)) {
                    scopes.pop();
                }
                continue;
            }

            if token.is_word("DECLARE") {
                if !matches!(scopes.last(), Some(OraclePlSqlScope::Declaration)) {
                    scopes.push(OraclePlSqlScope::Declaration);
                }
            } else if token.is_any_word(&["PROCEDURE", "FUNCTION"])
                && matches!(scopes.last(), Some(OraclePlSqlScope::Declaration | OraclePlSqlScope::Routine))
            {
                scopes.push(OraclePlSqlScope::RoutineHeader);
            } else if token.is_any_word(&["IS", "AS"]) && matches!(scopes.last(), Some(OraclePlSqlScope::RoutineHeader))
            {
                *scopes.last_mut().unwrap() = OraclePlSqlScope::Routine;
            } else if token.is_word("BEGIN") {
                // A local routine's BEGIN belongs to that routine, not to the outer block.
                if matches!(scopes.last(), Some(OraclePlSqlScope::Declaration | OraclePlSqlScope::Routine)) {
                    *scopes.last_mut().unwrap() = OraclePlSqlScope::Block;
                } else {
                    scopes.push(OraclePlSqlScope::Block);
                }
                saw_begin = true;
                complete = false;
            } else if token.is_word("CASE") {
                // Both CASE expressions and CASE statements own an END.
                // The CASE token that follows END CASE is a suffix, not a scope start.
                if previous_word_token(&self.tokens, index) != Some("END") {
                    scopes.push(OraclePlSqlScope::Case);
                }
            } else if token.is_word("END") {
                let next = self.tokens.get(index + 1).and_then(OraclePlSqlToken::as_word);
                if matches!(next, Some("IF" | "LOOP")) {
                    continue;
                }
                if matches!(next, Some("CASE")) && !matches!(scopes.last(), Some(OraclePlSqlScope::Case)) {
                    continue;
                }
                if scopes.pop().is_some() {
                    complete = scopes.is_empty();
                }
            }
        }

        match object_kind {
            // Package/type objects complete on their outer END [name]; a package
            // body may omit an initialization BEGIN when it has no executable
            // initialization section.
            Some(OraclePlSqlCreateObjectKind::Body | OraclePlSqlCreateObjectKind::Spec) => complete,
            _ => saw_begin && complete,
        }
    }

    fn starts_create_plsql_object(tokens: &[OraclePlSqlToken]) -> bool {
        let tokens = Self::skip_create_modifiers(tokens);
        match tokens {
            // PACKAGE BODY / TYPE BODY are programmable blocks with an outer END.
            [object, body, ..] if object.is_any_word(&["PACKAGE", "TYPE"]) && body.is_word("BODY") => true,
            // Plain CREATE TYPE ... AS OBJECT (...); ends with ");" — not a PL/SQL block.
            [object, ..] if object.is_word("TYPE") => false,
            [object, ..] if object.is_any_word(&["FUNCTION", "PROCEDURE", "TRIGGER", "PACKAGE"]) => true,
            _ => false,
        }
    }

    fn create_object_kind(&self) -> Option<OraclePlSqlCreateObjectKind> {
        if self.tokens.first().is_none_or(|token| !token.is_word("CREATE")) {
            return None;
        }
        let tokens = Self::skip_create_modifiers(&self.tokens[1..]);
        match tokens {
            [object, body, ..] if object.is_any_word(&["PACKAGE", "TYPE"]) && body.is_word("BODY") => {
                Some(OraclePlSqlCreateObjectKind::Body)
            }
            // Only PACKAGE specs lack BEGIN; plain TYPE objects are ordinary SQL.
            [object, ..] if object.is_word("PACKAGE") => Some(OraclePlSqlCreateObjectKind::Spec),
            _ => None,
        }
    }

    /// Skip OR REPLACE / FORCE / NOFORCE / EDITIONABLE modifiers after CREATE.
    fn skip_create_modifiers(tokens: &[OraclePlSqlToken]) -> &[OraclePlSqlToken] {
        let mut rest = tokens;
        loop {
            match rest {
                [or, replace, tail @ ..] if or.is_word("OR") && replace.is_word("REPLACE") => {
                    rest = tail;
                }
                [modifier, tail @ ..]
                    if modifier.is_any_word(&["FORCE", "NOFORCE", "EDITIONABLE", "NONEDITIONABLE"]) =>
                {
                    rest = tail;
                }
                _ => break,
            }
        }
        rest
    }

    fn is_transaction_begin_tail(token: &OraclePlSqlToken) -> bool {
        token.is_semicolon() || token.is_any_word(&["TRANSACTION", "WORK"])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OraclePlSqlCreateObjectKind {
    Spec,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OraclePlSqlScope {
    Object,
    /// DECLARE section awaiting its own BEGIN.
    Declaration,
    /// Local PROCEDURE/FUNCTION header, before IS/AS (a `;` here is a forward declaration).
    RoutineHeader,
    /// Local PROCEDURE/FUNCTION after IS/AS, awaiting its own BEGIN.
    Routine,
    Block,
    Case,
}

impl OraclePlSqlToken {
    fn word(value: String) -> Self {
        Self::Word(value)
    }

    fn from_sqlparser_token(token: Token) -> Option<Self> {
        match token {
            Token::Word(word) if word.quote_style.is_none() => Some(Self::Word(word.value.to_ascii_uppercase())),
            Token::Word(_) => Some(Self::QuotedIdentifier),
            Token::SemiColon => Some(Self::Semicolon),
            _ => None,
        }
    }

    fn is_word(&self, expected: &str) -> bool {
        matches!(self, Self::Word(value) if value == expected)
    }

    fn as_word(&self) -> Option<&str> {
        match self {
            Self::Word(value) => Some(value),
            Self::QuotedIdentifier | Self::Semicolon => None,
        }
    }

    fn is_any_word(&self, expected: &[&str]) -> bool {
        expected.iter().any(|word| self.is_word(word))
    }

    fn is_semicolon(&self) -> bool {
        matches!(self, Self::Semicolon)
    }
}

fn previous_word_token(tokens: &[OraclePlSqlToken], index: usize) -> Option<&str> {
    tokens[..index].iter().rev().find_map(OraclePlSqlToken::as_word)
}

fn next_word_token(tokens: &[OraclePlSqlToken], index: usize) -> Option<&str> {
    tokens[index + 1..].iter().find_map(OraclePlSqlToken::as_word)
}

fn starts_with_chars(chars: &[char], start: usize, needle: &[char]) -> bool {
    start + needle.len() <= chars.len() && chars[start..start + needle.len()] == *needle
}

fn dollar_quote_tag_at(chars: &[char], start: usize) -> Option<String> {
    if chars.get(start) != Some(&'$') {
        return None;
    }

    match chars.get(start + 1) {
        Some('$') => return Some("$$".to_string()),
        Some(ch) if ch.is_ascii_alphabetic() || *ch == '_' => {}
        _ => return None,
    }

    let mut end = start + 2;
    while let Some(ch) = chars.get(end) {
        if *ch == '$' {
            return Some(chars[start..=end].iter().collect());
        }
        if !ch.is_ascii_alphanumeric() && *ch != '_' {
            return None;
        }
        end += 1;
    }

    None
}

pub fn has_executable_sql(statement: &str) -> bool {
    has_executable_sql_with_options(statement, SqlParsingOptions::default())
}

pub fn has_executable_sql_for_database(statement: &str, db_type: DatabaseType) -> bool {
    has_executable_sql_with_options(statement, SqlParsingOptions::for_database_type(db_type))
}

fn executable_sql_bounds(statement: &str, options: SqlParsingOptions) -> Option<(usize, usize)> {
    let trimmed_end = statement.trim_end().len();
    let trimmed = &statement[..trimmed_end];
    let executable = leading_executable_sql_with_options(trimmed, options);
    if executable.is_empty() {
        return None;
    }
    let executable_start = trimmed.len() - executable.len();
    let start = if options.profile.preserves_tdsql_leading_directives {
        crate::tdsql_mysql::leading_directive_start(trimmed, executable_start).unwrap_or(executable_start)
    } else {
        executable_start
    };
    Some((start, trimmed_end))
}

fn has_executable_sql_with_options(statement: &str, options: SqlParsingOptions) -> bool {
    let chars = statement.chars().collect::<Vec<_>>();
    let mut in_line_comment = false;
    let mut block_comment_depth = 0usize;
    let mut previous = None;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            previous = Some(ch);
            i += 1;
            continue;
        }

        if block_comment_depth > 0 {
            if options.profile.supports_nested_block_comments && previous == Some('/') && ch == '*' {
                block_comment_depth += 1;
                // A nested opener's asterisk must not pair with a following slash as a close.
                previous = None;
                i += 1;
                continue;
            } else if previous == Some('*') && ch == '/' {
                block_comment_depth -= 1;
            }
            previous = Some(ch);
            i += 1;
            continue;
        }

        if ch == '-' && next == Some('-') {
            in_line_comment = true;
            previous = Some(ch);
            i += 1;
            continue;
        }

        if options.profile.supports_hash_line_comments && ch == '#' {
            in_line_comment = true;
            previous = Some(ch);
            i += 1;
            continue;
        }

        if ch == '/' && next == Some('*') {
            if is_mysql_executable_comment_start(&chars, i) {
                return true;
            }
            block_comment_depth = 1;
            // The opener's own asterisk must not read as a nested opener.
            previous = None;
            i += 1;
            continue;
        }

        if !ch.is_whitespace() {
            return true;
        }

        previous = Some(ch);
        i += 1;
    }

    false
}

fn is_mysql_executable_comment_start(chars: &[char], start: usize) -> bool {
    chars.get(start) == Some(&'/')
        && chars.get(start + 1) == Some(&'*')
        && (chars.get(start + 2) == Some(&'!')
            || (chars.get(start + 2) == Some(&'M') && chars.get(start + 3) == Some(&'!')))
}

#[cfg(test)]
fn split_sql_script(sql: &str) -> Result<Vec<String>, String> {
    Ok(split_sql_statements(sql))
}

#[cfg(test)]
mod tests {
    use crate::models::connection::DatabaseType;

    use super::{
        contains_or_fuzzy_match, dash_dash_starts_line_comment, decode_sql_file_bytes,
        find_statement_at_cursor_for_database, fuzzy_filter_enabled, fuzzy_like_pattern_with_escape,
        fuzzy_subsequence_match, optimize_sql_file_import_statements, prepare_sql_file_statement, split_sql_script,
        split_sql_statement_ranges_with_options, split_sql_statements_for_database, starts_with_executable_sql_keyword,
        starts_with_executable_sql_keyword_for_database, starts_with_oracle_style_routine_body, SqlDialectProfile,
        SqlFileRequest, SqlFileStatementAction, SqlParsingOptions, SqlStatementSplitter,
    };

    #[test]
    fn sql_file_request_schema_is_optional_and_round_trips_when_present() {
        let legacy = serde_json::json!({
            "executionId": "legacy",
            "connectionId": "mysql-1",
            "database": "app",
            "filePath": "backup.sql",
            "continueOnError": false
        });
        let legacy_request: SqlFileRequest = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy_request.schema, None);
        assert!(serde_json::to_value(&legacy_request).unwrap().get("schema").is_none());

        let with_schema = serde_json::json!({
            "executionId": "oracle-schema",
            "connectionId": "oceanbase-oracle-1",
            "database": "tenant_service",
            "schema": "APP",
            "filePath": "restore.sql",
            "continueOnError": false
        });
        let request: SqlFileRequest = serde_json::from_value(with_schema).unwrap();
        assert_eq!(request.database, "tenant_service");
        assert_eq!(request.schema.as_deref(), Some("APP"));
        assert_eq!(serde_json::to_value(request).unwrap()["schema"], "APP");
    }

    #[test]
    fn fuzzy_subsequence_match_matches_ordered_characters() {
        assert!(fuzzy_subsequence_match("system_user", "sysu"));
        assert!(contains_or_fuzzy_match("user_order", "uo"));
        assert!(!contains_or_fuzzy_match("alpha", "uo"));
    }

    #[test]
    fn contains_or_fuzzy_match_skips_fuzzy_for_single_character_filters() {
        assert!(fuzzy_filter_enabled("uo"));
        assert!(!fuzzy_filter_enabled("u"));
        assert!(contains_or_fuzzy_match("user_order", "u"));
        assert!(!contains_or_fuzzy_match("orders", "u"));
    }

    #[test]
    fn fuzzy_like_pattern_with_escape_keeps_wildcards_literal() {
        let pattern = fuzzy_like_pattern_with_escape("user_%", |value| value.replace('%', "\\%").replace('_', "\\_"));

        assert_eq!(pattern, "%u%s%e%r%\\_%\\%%");
    }

    #[test]
    fn splits_semicolon_delimited_statements() {
        assert_eq!(
            split_sql_script("CREATE TABLE a(id int); INSERT INTO a VALUES (1);").unwrap(),
            vec!["CREATE TABLE a(id int)", "INSERT INTO a VALUES (1)"]
        );
    }

    #[test]
    fn mysql_split_skips_comment_only_statement() {
        assert!(split_sql_statements_for_database("-- DBX SQL preview crash reproducer\n\n;", DatabaseType::Mysql)
            .is_empty());
    }

    #[test]
    fn mysql_dash_dash_without_trailing_space_is_not_a_comment_per_issue_5382() {
        // MySQL requires `--` to be followed by whitespace to start a line comment;
        // `5--1` is subtraction (5 - -1), not a comment.
        assert_eq!(split_sql_statements_for_database("SELECT 5--1;", DatabaseType::Mysql), vec!["SELECT 5--1"]);
    }

    #[test]
    fn mysql_dash_dash_without_trailing_space_does_not_merge_following_statements_per_issue_5382() {
        let statements = split_sql_statements_for_database(
            "INSERT INTO t VALUES (1, 5--1);\nINSERT INTO t VALUES (2, 10);",
            DatabaseType::Mysql,
        );
        assert_eq!(statements, vec!["INSERT INTO t VALUES (1, 5--1)", "INSERT INTO t VALUES (2, 10)"]);
    }

    #[test]
    fn mysql_dash_dash_with_trailing_space_still_starts_a_line_comment() {
        // The comment text itself is preserved verbatim in the following statement's
        // buffer (comments are never stripped, just protected from delimiter parsing) —
        // this only asserts the embedded `;` inside the comment doesn't split early.
        let statements = split_sql_statements_for_database(
            "INSERT INTO t VALUES (1); -- trailing comment; with semicolon\nINSERT INTO t VALUES (2);",
            DatabaseType::Mysql,
        );
        assert_eq!(
            statements,
            vec!["INSERT INTO t VALUES (1)", "-- trailing comment; with semicolon\nINSERT INTO t VALUES (2)"]
        );
    }

    #[test]
    fn dash_dash_requires_space_only_for_mysql_compatible_profiles() {
        let mysql_profile = SqlDialectProfile::for_database_type(DatabaseType::Mysql);
        assert!(dash_dash_starts_line_comment(mysql_profile, Some(' ')));
        assert!(dash_dash_starts_line_comment(mysql_profile, Some('\n')));
        assert!(dash_dash_starts_line_comment(mysql_profile, Some('\u{1}')));
        assert!(dash_dash_starts_line_comment(mysql_profile, None));
        assert!(!dash_dash_starts_line_comment(mysql_profile, Some('1')));

        // Postgres, Oracle/DM, SQL Server, ... treat `--` as a comment opener unconditionally.
        let default_profile = SqlDialectProfile::default();
        assert!(dash_dash_starts_line_comment(default_profile, Some('1')));
        assert!(dash_dash_starts_line_comment(default_profile, None));
    }

    #[test]
    fn mysql_dash_dash_opener_without_space_can_span_chunks() {
        let mut splitter = SqlStatementSplitter::with_options(SqlParsingOptions::mysql_compatible());

        assert_eq!(splitter.push_chunk("SELECT 5-"), Vec::<String>::new());
        assert_eq!(splitter.push_chunk("-1;"), vec!["SELECT 5--1"]);
        assert_eq!(splitter.finish(), Vec::<String>::new());
    }

    #[test]
    fn mysql_complete_dash_dash_sequence_waits_for_the_next_chunk() {
        let mut splitter = SqlStatementSplitter::with_options(SqlParsingOptions::mysql_compatible());

        assert_eq!(splitter.push_chunk("SELECT 5--"), Vec::<String>::new());
        assert_eq!(splitter.push_chunk("1;\nSELECT 2;"), vec!["SELECT 5--1", "SELECT 2"]);
        assert_eq!(splitter.finish(), Vec::<String>::new());
    }

    #[test]
    fn mysql_control_character_after_dashes_starts_a_line_comment() {
        let statements = split_sql_statements_for_database("SELECT 1--\u{1}; hidden\nSELECT 2;", DatabaseType::Mysql);

        assert_eq!(statements, vec!["SELECT 1--\u{1}; hidden\nSELECT 2"]);
    }

    #[test]
    fn mysql_keyword_detection_skips_comment_only_input() {
        assert!(!starts_with_executable_sql_keyword_for_database(
            "-- DBX SQL preview crash reproducer\n\n",
            &["SELECT"],
            DatabaseType::Mysql,
        ));
    }

    #[test]
    fn decodes_utf8_bom_sql_file_bytes_without_bom_statement_prefix() {
        let sql = decode_sql_file_bytes(b"\xEF\xBB\xBFCREATE TABLE t(id int);").unwrap();

        assert_eq!(sql, "CREATE TABLE t(id int);");
    }

    #[test]
    fn decodes_gbk_sql_file_bytes_before_execution() {
        let bytes = b"INSERT INTO t VALUES ('\xD6\xD0\xCE\xC4');";
        let sql = decode_sql_file_bytes(bytes).unwrap();

        assert_eq!(sql, "INSERT INTO t VALUES ('中文');");
    }

    #[test]
    fn decodes_utf16le_bom_sql_file_bytes() {
        let bytes = [
            0xFF, 0xFE, b'S', 0x00, b'E', 0x00, b'L', 0x00, b'E', 0x00, b'C', 0x00, b'T', 0x00, b' ', 0x00, b'1', 0x00,
            b';', 0x00,
        ];
        let sql = decode_sql_file_bytes(&bytes).unwrap();

        assert_eq!(sql, "SELECT 1;");
    }

    #[test]
    fn ignores_semicolons_inside_quotes_and_comments() {
        let sql = "\
            INSERT INTO logs VALUES ('a;b', \"c;d\", `weird;name`);\n\
            -- comment ; ignored\n\
            /* block ; ignored */\n\
            SELECT 1;";
        assert_eq!(
            split_sql_script(sql).unwrap(),
            vec![
                "INSERT INTO logs VALUES ('a;b', \"c;d\", `weird;name`)",
                "-- comment ; ignored\n/* block ; ignored */\nSELECT 1",
            ]
        );
    }

    #[test]
    fn closes_mysql_string_after_even_trailing_backslashes() {
        let sql = r#"CREATE TABLE paths (value varchar(100) COMMENT 'Windows path\\'); DROP TABLE paths;"#;

        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec![r#"CREATE TABLE paths (value varchar(100) COMMENT 'Windows path\\')"#, "DROP TABLE paths"]
        );
    }

    #[test]
    fn cursor_statement_closes_mysql_string_after_escaped_backslash() {
        let sql = "SELECT 'a\\\\';\nSELECT 2;";
        let cursor = sql.find("SELECT 2").unwrap() + 3;
        assert_eq!(find_statement_at_cursor_for_database(sql, cursor, DatabaseType::Mysql), "SELECT 2");
        assert_eq!(find_statement_at_cursor_for_database(sql, 3, DatabaseType::Mysql), "SELECT 'a\\\\'");
    }

    #[test]
    fn cursor_statement_treats_mysql_dash_dash_without_space_as_minus() {
        let sql = "SELECT 5--1;\nSELECT 2;";
        let cursor = sql.find("SELECT 2").unwrap() + 3;
        assert_eq!(find_statement_at_cursor_for_database(sql, cursor, DatabaseType::Mysql), "SELECT 2");
        assert_eq!(find_statement_at_cursor_for_database(sql, 3, DatabaseType::Mysql), "SELECT 5--1");
    }

    #[test]
    fn keeps_mysql_string_open_after_odd_trailing_backslashes() {
        let sql = r#"INSERT INTO notes VALUES ('it\'s; still one value'); SELECT 1;"#;

        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec![r#"INSERT INTO notes VALUES ('it\'s; still one value')"#, "SELECT 1"]
        );
    }

    #[test]
    fn closes_mysql_string_after_even_trailing_backslashes_across_chunks() {
        let mut splitter =
            SqlStatementSplitter::with_options(SqlParsingOptions::for_database_type(DatabaseType::Mysql));

        assert!(splitter.push_chunk("CREATE TABLE paths (value varchar(100) COMMENT 'Windows path\\").is_empty());
        assert_eq!(
            splitter.push_chunk("\\'); DROP TABLE paths;"),
            vec![r#"CREATE TABLE paths (value varchar(100) COMMENT 'Windows path\\')"#, "DROP TABLE paths"]
        );
        assert!(splitter.finish().is_empty());
    }

    #[test]
    fn mysql_block_comments_do_not_nest() {
        let sql = "/* a /* b */ SELECT 1; SELECT 2;";

        // MySQL documents that block comments do not nest, so the inner `*/` ends
        // the comment and the statements behind it stay separate.
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["/* a /* b */ SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn mysql_treats_q_quotes_as_plain_strings() {
        let sql = "SELECT q'[a;b]' FROM t; SELECT 1;";

        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["SELECT q'[a;b]' FROM t", "SELECT 1"]
        );
    }

    #[test]
    fn emits_trailing_statement_without_semicolon() {
        assert_eq!(
            split_sql_script("CREATE TABLE a(id int);\nINSERT INTO a VALUES (1)").unwrap(),
            vec!["CREATE TABLE a(id int)", "INSERT INTO a VALUES (1)"]
        );
    }

    #[test]
    fn line_comment_openers_can_span_chunks() {
        let mut splitter = SqlStatementSplitter::default();

        assert_eq!(splitter.push_chunk("SELECT 1; -"), vec!["SELECT 1"]);
        assert_eq!(splitter.push_chunk("- comment ; ignored\nSELECT 2;"), vec!["-- comment ; ignored\nSELECT 2"]);
        assert_eq!(splitter.finish(), Vec::<String>::new());
    }

    #[test]
    fn block_comment_openers_can_span_chunks() {
        let mut splitter = SqlStatementSplitter::default();

        assert_eq!(splitter.push_chunk("SELECT 1; /"), vec!["SELECT 1"]);
        assert_eq!(splitter.push_chunk("* comment ; ignored */\nSELECT 2;"), vec!["/* comment ; ignored */\nSELECT 2"]);
        assert_eq!(splitter.finish(), Vec::<String>::new());
    }

    #[test]
    fn skips_comment_only_tail_after_statement() {
        assert_eq!(
            split_sql_script("CREATE TABLE a(id int); -- done\n/* no more sql */").unwrap(),
            vec!["CREATE TABLE a(id int)"]
        );
    }

    #[test]
    fn skips_comment_only_statement_with_semicolon() {
        assert_eq!(
            split_sql_script("-- insert sqluser.tb_a values (6,'006','测试6','无');").unwrap(),
            Vec::<String>::new()
        );
        assert!(!super::has_executable_sql("-- insert sqluser.tb_a values (6,'006','测试6','无');"));
    }

    #[test]
    fn keeps_mysql_executable_comments_as_statements() {
        assert_eq!(
            split_sql_script("/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;\nSELECT 1;",).unwrap(),
            vec!["/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */", "SELECT 1",]
        );
    }

    #[test]
    fn detects_result_set_keyword_after_comments() {
        assert!(starts_with_executable_sql_keyword("-- comment\nselect * from users;", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword(
            "/* comment */\nWITH rows AS (SELECT 1) SELECT * FROM rows;",
            &["WITH"]
        ));
        assert!(!starts_with_executable_sql_keyword("-- comment only\n", &["SELECT"]));
    }

    #[test]
    fn detects_mysql_executable_comment_keyword() {
        assert!(starts_with_executable_sql_keyword("/*!40101 SELECT 1 */", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword("/*M! SELECT 1 */", &["SELECT"]));
    }

    #[test]
    fn describe_keyword_detection_accepts_desc_shorthand() {
        assert!(starts_with_executable_sql_keyword("DESC users", &["DESCRIBE"]));
        assert!(starts_with_executable_sql_keyword("-- comment\nDESC users", &["DESCRIBE"]));
    }

    #[test]
    fn detects_keyword_after_parentheses() {
        assert!(starts_with_executable_sql_keyword("(SELECT 1)", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword("  (  SELECT 1  )", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword("((SELECT 1))", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword("(SELECT * FROM users)", &["SELECT"]));
        assert!(starts_with_executable_sql_keyword("(INSERT INTO users VALUES (1))", &["INSERT"]));
        assert!(starts_with_executable_sql_keyword("/* comment */(UPDATE users SET name = 'test')", &["UPDATE"]));
    }

    #[test]
    fn mysql_hash_comments_are_ignored_for_keyword_detection() {
        assert!(starts_with_executable_sql_keyword_for_database(
            "# comment only for mysql\nSELECT 1",
            &["SELECT"],
            DatabaseType::Mysql
        ));
        assert!(!starts_with_executable_sql_keyword("# comment only for mysql\nSELECT 1", &["SELECT"]));
    }

    #[test]
    fn prepares_mysql_executable_comments_for_mysql_compatible_imports() {
        assert_eq!(
            prepare_sql_file_statement("/*!40101 SET NAMES utf8mb4 */", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Execute("SET NAMES utf8mb4".to_string())
        );
    }

    #[test]
    fn skips_mysql_key_toggle_comments_for_mysql_compatible_imports() {
        assert_eq!(
            prepare_sql_file_statement(" /*!40000 ALTER TABLE `dd_admin` ENABLE KEYS */", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement("/*!40000 ALTER TABLE `dd_admin` DISABLE KEYS */", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
    }

    #[test]
    fn skips_mysql_lock_table_statements_for_mysql_compatible_imports() {
        assert_eq!(
            prepare_sql_file_statement("LOCK TABLES `dd_geo_json` WRITE", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement("UNLOCK TABLES", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement(
                "-- Dumping data for table `dd_geo_json`\nLOCK TABLES `dd_geo_json` WRITE",
                &DatabaseType::Mysql,
                None
            ),
            SqlFileStatementAction::Skip
        );
    }

    #[test]
    fn skips_mysql_session_restore_statements_for_mysql_compatible_imports() {
        assert_eq!(
            prepare_sql_file_statement(
                "/*!40101 SET character_set_client = @saved_cs_client */",
                &DatabaseType::Mysql,
                None
            ),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement("/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement("SET FOREIGN_KEY_CHECKS=@OLD_FOREIGN_KEY_CHECKS", &DatabaseType::Mysql, None),
            SqlFileStatementAction::Skip
        );
        assert_eq!(
            prepare_sql_file_statement(
                "/*!40101 SET @saved_cs_client = @@character_set_client */",
                &DatabaseType::Mysql,
                None
            ),
            SqlFileStatementAction::Execute("SET @saved_cs_client = @@character_set_client".to_string())
        );
    }

    #[test]
    fn keeps_existing_mysql_multi_row_insert_unchanged() {
        let statements = vec!["INSERT INTO users (id) VALUES (1), (2)".to_string()];

        let optimized = optimize_sql_file_import_statements(&statements, Some(DatabaseType::Mysql), None);

        assert_eq!(optimized.len(), 1);
        assert_eq!(optimized[0].source_statement_count, 1);
        assert_eq!(optimized[0].sql, statements[0]);
    }

    #[test]
    fn mysql_dump_import_keeps_skips_and_insert_statement_boundaries() {
        let statements = vec![
            "LOCK TABLES `users` WRITE".to_string(),
            "INSERT INTO `users` VALUES (1)".to_string(),
            "INSERT INTO `users` VALUES (2)".to_string(),
            "UNLOCK TABLES".to_string(),
        ];

        let optimized = optimize_sql_file_import_statements(&statements, Some(DatabaseType::Mysql), None);

        assert_eq!(optimized.len(), 4);
        assert_eq!(optimized[0].kind, super::SqlFileImportStatementKind::Skip);
        assert_eq!(optimized[1].source_statement_count, 1);
        assert_eq!(optimized[2].source_statement_count, 1);
        assert_eq!(optimized[3].kind, super::SqlFileImportStatementKind::Skip);
    }

    // --- DELIMITER support ---

    #[test]
    fn delimiter_basic_procedure() {
        let sql = "\
DELIMITER //
CREATE PROCEDURE foo()
BEGIN
  SELECT 1;
  SELECT 2;
END //
DELIMITER ;
SELECT 3;";
        assert_eq!(
            super::split_sql_statements(sql),
            vec!["CREATE PROCEDURE foo()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND", "SELECT 3",]
        );
    }

    #[test]
    fn delimiter_no_trailing_newline() {
        let sql = "DELIMITER //\nSELECT 1//";
        assert_eq!(super::split_sql_statements(sql), vec!["SELECT 1"]);
    }

    #[test]
    fn delimiter_no_space_before_delim() {
        let sql = "DELIMITER //\nCREATE PROCEDURE foo() BEGIN SELECT 1; END//\nDELIMITER ;";
        assert_eq!(super::split_sql_statements(sql), vec!["CREATE PROCEDURE foo() BEGIN SELECT 1; END"]);
    }

    #[test]
    fn delimiter_reset_without_whitespace() {
        let sql = "DELIMITER //\nCREATE PROCEDURE foo() BEGIN SELECT 1; END//\nDELIMITER;\nCALL foo();";
        assert_eq!(super::split_sql_statements(sql), vec!["CREATE PROCEDURE foo() BEGIN SELECT 1; END", "CALL foo()"]);
    }

    #[test]
    fn delimiter_case_insensitive() {
        let sql = "delimiter //\nSELECT 1//\ndelimiter ;\nSELECT 2;";
        assert_eq!(super::split_sql_statements(sql), vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn delimiter_double_dollar() {
        let sql = "DELIMITER $$\nCREATE FUNCTION f() RETURNS INT BEGIN RETURN 1; END $$\nDELIMITER ;";
        assert_eq!(super::split_sql_statements(sql), vec!["CREATE FUNCTION f() RETURNS INT BEGIN RETURN 1; END"]);
    }

    #[test]
    fn delimiter_semicolons_preserved_inside_body() {
        let sql = "\
DELIMITER //
CREATE TRIGGER t BEFORE INSERT ON tbl FOR EACH ROW
BEGIN
  SET NEW.a = 1;
  SET NEW.b = 2;
END //
DELIMITER ;";
        let stmts = super::split_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("SET NEW.a = 1;\n  SET NEW.b = 2;"));
    }

    #[test]
    fn delimiter_multiple_statements() {
        let sql = "\
DELIMITER //
CREATE PROCEDURE p1() BEGIN SELECT 1; END //
CREATE PROCEDURE p2() BEGIN SELECT 2; END //
DELIMITER ;";
        assert_eq!(
            super::split_sql_statements(sql),
            vec!["CREATE PROCEDURE p1() BEGIN SELECT 1; END", "CREATE PROCEDURE p2() BEGIN SELECT 2; END",]
        );
    }

    #[test]
    fn delimiter_after_comment_with_chinese() {
        let sql = "\
-- 判断字段是否存在
DELIMITER $$
DROP FUNCTION IF EXISTS isFieldExisting $$
CREATE FUNCTION isFieldExisting(s VARCHAR(100), t VARCHAR(100), f VARCHAR(100))
    RETURNS INT
    RETURN (SELECT COUNT(COLUMN_NAME)
            FROM INFORMATION_SCHEMA.columns
            WHERE TABLE_SCHEMA = s
              AND TABLE_NAME = t
              AND COLUMN_NAME = f)$$
DELIMITER ;";
        let stmts = super::split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("DROP FUNCTION"));
        assert!(stmts[1].starts_with("CREATE FUNCTION"));
    }

    #[test]
    fn delimiter_after_ascii_comment() {
        let sql = "\
-- check field existence
DELIMITER $$
SELECT 1 $$
DELIMITER ;";
        assert_eq!(super::split_sql_statements(sql), vec!["SELECT 1"]);
    }

    #[test]
    fn delimiter_after_statement() {
        let sql = "\
SELECT 1;
DELIMITER $$
SELECT 2 $$
DELIMITER ;";
        assert_eq!(super::split_sql_statements(sql), vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn mysql_routine_without_delimiter_keeps_body_together() {
        let sql = "\
CREATE PROCEDURE p()
BEGIN
  SELECT 1;
  SELECT 2;
END;
SELECT 3;";
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND", "SELECT 3"]
        );
    }

    #[test]
    fn mysql_routine_without_delimiter_handles_nested_end_suffixes() {
        let sql = "\
CREATE PROCEDURE p()
BEGIN
  IF 1 = 1 THEN
    SELECT 'ok';
  END IF;
END;
SELECT 2;";
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["CREATE PROCEDURE p()\nBEGIN\n  IF 1 = 1 THEN\n    SELECT 'ok';\n  END IF;\nEND", "SELECT 2"]
        );
    }

    #[test]
    fn mysql_routine_without_delimiter_handles_loop_end_suffixes() {
        let sql = "\
CREATE PROCEDURE p_loop()
BEGIN
  WHILE 1 = 0 DO
    SELECT 'while; still body';
  END WHILE;
  REPEAT
    SELECT 'repeat; still body';
  UNTIL 1 = 1 END REPEAT;
END;
SELECT 2;";
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec![
                "CREATE PROCEDURE p_loop()\nBEGIN\n  WHILE 1 = 0 DO\n    SELECT 'while; still body';\n  END WHILE;\n  REPEAT\n    SELECT 'repeat; still body';\n  UNTIL 1 = 1 END REPEAT;\nEND",
                "SELECT 2",
            ]
        );
    }

    #[test]
    fn mysql_routine_without_delimiter_handles_case_endings() {
        let expression = "\
CREATE PROCEDURE p_case()
BEGIN
  INSERT INTO audit_log (status_text)
  SELECT CASE WHEN active = 1 THEN 'active' ELSE 'inactive' END;
  DELETE FROM stale_rows WHERE expires_at < NOW();
END;
SELECT 2;";
        assert_eq!(
            split_sql_statements_for_database(expression, DatabaseType::Mysql),
            vec![
                "CREATE PROCEDURE p_case()\nBEGIN\n  INSERT INTO audit_log (status_text)\n  SELECT CASE WHEN active = 1 THEN 'active' ELSE 'inactive' END;\n  DELETE FROM stale_rows WHERE expires_at < NOW();\nEND",
                "SELECT 2",
            ]
        );

        let statement = "\
CREATE PROCEDURE p_case_statement()
BEGIN
  CASE WHEN active = 1 THEN SELECT 1; ELSE SELECT 0; END CASE;
  DELETE FROM stale_rows WHERE expires_at < NOW();
END;
SELECT 2;";
        assert_eq!(
            split_sql_statements_for_database(statement, DatabaseType::Mysql),
            vec![
                "CREATE PROCEDURE p_case_statement()\nBEGIN\n  CASE WHEN active = 1 THEN SELECT 1; ELSE SELECT 0; END CASE;\n  DELETE FROM stale_rows WHERE expires_at < NOW();\nEND",
                "SELECT 2",
            ]
        );
    }

    #[test]
    fn mysql_routine_without_delimiter_closes_nested_case_begin_blocks_in_order() {
        let sql = "\
CREATE PROCEDURE p_nested_case()
BEGIN
  CASE WHEN active = 1 THEN BEGIN SELECT 1; END; ELSE SELECT 0; END CASE;
  DELETE FROM stale_rows WHERE expires_at < NOW();
END;
SELECT 2;";
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec![
                "CREATE PROCEDURE p_nested_case()\nBEGIN\n  CASE WHEN active = 1 THEN BEGIN SELECT 1; END; ELSE SELECT 0; END CASE;\n  DELETE FROM stale_rows WHERE expires_at < NOW();\nEND",
                "SELECT 2",
            ]
        );
    }

    #[test]
    fn mysql_regular_begin_transaction_still_splits_without_delimiter() {
        assert_eq!(
            split_sql_statements_for_database("BEGIN; INSERT INTO t VALUES (1); COMMIT;", DatabaseType::Mysql),
            vec!["BEGIN", "INSERT INTO t VALUES (1)", "COMMIT"]
        );
    }

    #[test]
    fn finds_statement_at_cursor() {
        let sql = "SELECT 1; SELECT 2";

        assert_eq!(super::find_statement_at_cursor(sql, 3), "SELECT 1");
        assert_eq!(super::find_statement_at_cursor(sql, 12), "SELECT 2");
        assert_eq!(super::find_statement_at_cursor(sql, 18), "SELECT 2");
    }

    #[test]
    fn finds_statement_at_cursor_after_unicode_comment() {
        let sql = "-- 判断字段是否存在\nSELECT 1; SELECT 2";
        let cursor_byte = sql.find("SELECT 2").unwrap();
        let cursor = sql[..cursor_byte].encode_utf16().count();

        assert_eq!(super::find_statement_at_cursor(sql, cursor), "SELECT 2");
    }

    #[test]
    fn finds_statement_at_cursor_after_semicolon_with_blank_line_stays_on_previous_statement() {
        let sql = "SELECT 1;\n\nSELECT 2;";
        let cursor = sql[..sql.find(';').unwrap() + 1].encode_utf16().count();
        assert_eq!(super::find_statement_at_cursor(sql, cursor), "SELECT 1");
    }

    #[test]
    fn finds_statement_at_cursor_after_semicolon_same_line_moves_to_next_statement() {
        let sql = "SELECT 1; SELECT 2;";
        let cursor = sql[..sql.find("SELECT 2").unwrap()].encode_utf16().count();

        assert_eq!(super::find_statement_at_cursor(sql, cursor), "SELECT 2");
    }

    #[test]
    fn finds_statement_at_cursor_after_double_blank_line_without_semicolon() {
        let sql = "SELECT * FROM old_table\n\n\nCREATE VIEW v AS SELECT 1";
        let cursor = sql[..sql.find("CREATE VIEW").unwrap()].encode_utf16().count();

        assert_eq!(super::find_statement_at_cursor(sql, cursor), "CREATE VIEW v AS SELECT 1");
    }

    #[test]
    fn keeps_create_view_statement_together_across_double_blank_line() {
        let sql = "CREATE VIEW v AS\n\n\nSELECT 1";
        let cursor = sql[..sql.find("SELECT 1").unwrap()].encode_utf16().count();

        assert_eq!(super::find_statement_at_cursor(sql, cursor), "CREATE VIEW v AS\n\n\nSELECT 1");
    }

    #[test]
    fn finds_statement_with_dollar_quote() {
        let sql = "SELECT $$a;b$$; SELECT 2";

        assert_eq!(super::find_statement_at_cursor(sql, 3), "SELECT $$a;b$$");
        assert_eq!(super::find_statement_at_cursor(sql, 17), "SELECT 2");
    }

    #[test]
    fn finds_statement_with_custom_delimiter() {
        let sql = "\
DELIMITER //
CREATE PROCEDURE foo()
BEGIN
  SELECT 1;
END //
DELIMITER ;
SELECT 2;";
        let cursor = sql.find("SELECT 1").unwrap();
        let next_cursor = sql.rfind("SELECT 2").unwrap();

        assert_eq!(super::find_statement_at_cursor(sql, cursor), "CREATE PROCEDURE foo()\nBEGIN\n  SELECT 1;\nEND");
        assert_eq!(super::find_statement_at_cursor(sql, next_cursor), "SELECT 2");
    }

    #[test]
    fn mysql_hash_comments_split_statements_per_issue_428() {
        let sql = "SELECT 1; # mysql comment\n\nSELECT 2 # trailing comment";
        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["SELECT 1", "# mysql comment\n\nSELECT 2 # trailing comment"]
        );
    }

    #[test]
    fn mysql_delimiter_command_keeps_procedure_body_together_per_issue_1978() {
        let sql = "\
-- ----------------------------
-- Procedure structure for fix_collation
-- ----------------------------
DROP PROCEDURE IF EXISTS `fix_collation`;
delimiter ;;
CREATE PROCEDURE `fix_collation`()
BEGIN
DECLARE done INT DEFAULT FALSE;
DECLARE tbl_name VARCHAR(255);
DECLARE cur CURSOR FOR
SELECT TABLE_NAME FROM information_schema.TABLES
WHERE TABLE_SCHEMA = DATABASE() AND TABLE_COLLATION = 'utf8mb4_0900_ai_ci';
DECLARE CONTINUE HANDLER FOR NOT FOUND SET done = TRUE;

    OPEN cur;
    read_loop: LOOP
        FETCH cur INTO tbl_name;
        IF done THEN LEAVE read_loop; END IF;
        SET @sql = CONCAT('ALTER TABLE `', tbl_name, '` CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci');
        PREPARE stmt FROM @sql;
        EXECUTE stmt;
        DEALLOCATE PREPARE stmt;
    END LOOP;
    CLOSE cur;
END
;;
delimiter ;";

        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec![
                "-- ----------------------------\n-- Procedure structure for fix_collation\n-- ----------------------------\nDROP PROCEDURE IF EXISTS `fix_collation`",
                "CREATE PROCEDURE `fix_collation`()\nBEGIN\nDECLARE done INT DEFAULT FALSE;\nDECLARE tbl_name VARCHAR(255);\nDECLARE cur CURSOR FOR\nSELECT TABLE_NAME FROM information_schema.TABLES\nWHERE TABLE_SCHEMA = DATABASE() AND TABLE_COLLATION = 'utf8mb4_0900_ai_ci';\nDECLARE CONTINUE HANDLER FOR NOT FOUND SET done = TRUE;\n\n    OPEN cur;\n    read_loop: LOOP\n        FETCH cur INTO tbl_name;\n        IF done THEN LEAVE read_loop; END IF;\n        SET @sql = CONCAT('ALTER TABLE `', tbl_name, '` CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci');\n        PREPARE stmt FROM @sql;\n        EXECUTE stmt;\n        DEALLOCATE PREPARE stmt;\n    END LOOP;\n    CLOSE cur;\nEND",
            ]
        );
    }

    #[test]
    fn mysql_current_statement_ignores_delimiter_command_semicolon_per_issue_1978() {
        let sql = "\
delimiter ;;
CREATE PROCEDURE `fix_collation`()
BEGIN
    SET @sql = CONCAT('ALTER TABLE `', 't', '` CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci');
    PREPARE stmt FROM @sql;
    EXECUTE stmt;
END
;;
delimiter ;";
        let cursor = sql[..sql.find("PREPARE").unwrap()].encode_utf16().count();

        assert_eq!(
            find_statement_at_cursor_for_database(sql, cursor, DatabaseType::Mysql),
            "CREATE PROCEDURE `fix_collation`()\nBEGIN\n    SET @sql = CONCAT('ALTER TABLE `', 't', '` CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci');\n    PREPARE stmt FROM @sql;\n    EXECUTE stmt;\nEND"
        );
    }

    #[test]
    fn mysql_delimiter_command_skips_empty_custom_delimiter_statement_per_issue_1988() {
        let sql = "\
select COUNT(1) FROM your_table;
delimiter ;;
select COUNT(1) FROM your_table;

;;
delimiter ;";

        assert_eq!(
            split_sql_statements_for_database(sql, DatabaseType::Mysql),
            vec!["select COUNT(1) FROM your_table", "select COUNT(1) FROM your_table;"]
        );
    }

    #[test]
    fn mysql_delimiter_command_ranges_skip_empty_custom_delimiter_statement_per_issue_1988() {
        let sql = "\
select COUNT(1) FROM your_table;
delimiter ;;
select COUNT(1) FROM your_table;

;;
delimiter ;";

        let ranges =
            split_sql_statement_ranges_with_options(sql, SqlParsingOptions::for_database_type(DatabaseType::Mysql));

        assert_eq!(
            ranges.iter().map(|range| range.text.as_str()).collect::<Vec<_>>(),
            vec!["select COUNT(1) FROM your_table", "select COUNT(1) FROM your_table;"]
        );
    }

    #[test]
    fn mysql_current_statement_keeps_inline_hash_comment_per_issue_428() {
        let sql = "SELECT 1; # mysql comment\n\nSELECT 2 # trailing comment";
        let cursor = sql[..sql.find("SELECT 2").unwrap()].encode_utf16().count();
        assert_eq!(
            find_statement_at_cursor_for_database(sql, cursor, DatabaseType::Mysql),
            "SELECT 2 # trailing comment"
        );
    }

    #[test]
    fn mysql_single_statement_with_inline_comment_stays_executable_per_issue_428() {
        let sql = "SELECT 1 # mysql comment";
        let cursor = sql.encode_utf16().count();
        assert_eq!(find_statement_at_cursor_for_database(sql, cursor, DatabaseType::Mysql), "SELECT 1 # mysql comment");
    }
}

#[cfg(test)]
mod editor_encoding_tests {
    use super::*;
    #[test]
    fn utf16_bytes_and_round_trip() {
        for (encoding, expected) in [
            ("utf16le", vec![0xff, 0xfe, 0x41, 0, 0x2d, 0x4e, 0x3d, 0xd8, 0, 0xde]),
            ("utf16be", vec![0xfe, 0xff, 0, 0x41, 0x4e, 0x2d, 0xd8, 0x3d, 0xde, 0]),
        ] {
            let bytes = encode_sql_file_text("A中😀", encoding).unwrap();
            assert_eq!(bytes, expected);
            assert_eq!(decode_sql_file_bytes_with_encoding(&bytes, encoding).unwrap(), "A中😀");
        }
    }
}

/// The closing character of an Oracle alternative-quoting delimiter: brackets
/// pair up (`q'[a]'`) and every other character delimits itself (`q'!a!'`).
/// A quote or whitespace cannot delimit the literal.
fn oracle_q_quote_closer(opening: char) -> Option<char> {
    match opening {
        '[' => Some(']'),
        '{' => Some('}'),
        '(' => Some(')'),
        '<' => Some('>'),
        '\'' | ' ' | '\t' | '\n' | '\r' => None,
        other => Some(other),
    }
}

fn oracle_plsql_tokens(sql: &str) -> Vec<OraclePlSqlToken> {
    let dialect = OracleDialect {};
    if let Ok(tokens) = Tokenizer::new(&dialect, sql).tokenize() {
        return tokens.into_iter().filter_map(OraclePlSqlToken::from_sqlparser_token).collect();
    }

    oracle_plsql_tokens_fallback(sql)
}

fn oracle_plsql_tokens_fallback(sql: &str) -> Vec<OraclePlSqlToken> {
    let mut tokens = Vec::new();
    let mut iter = sql.char_indices().peekable();

    while let Some((_, ch)) = iter.next() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '-' && iter.peek().is_some_and(|(_, next)| *next == '-') {
            iter.next();
            for (_, comment_ch) in iter.by_ref() {
                if comment_ch == '\n' {
                    break;
                }
            }
            continue;
        }

        if ch == '/' && iter.peek().is_some_and(|(_, next)| *next == '*') {
            iter.next();
            let mut previous = '\0';
            for (_, comment_ch) in iter.by_ref() {
                if previous == '*' && comment_ch == '/' {
                    break;
                }
                previous = comment_ch;
            }
            continue;
        }

        if ch == '\'' {
            while let Some((_, quote_ch)) = iter.next() {
                if quote_ch == '\'' {
                    if iter.peek().is_some_and(|(_, next)| *next == '\'') {
                        iter.next();
                    } else {
                        break;
                    }
                }
            }
            continue;
        }

        if ch == '"' {
            for (_, ident_ch) in iter.by_ref() {
                if ident_ch == '"' {
                    break;
                }
            }
            tokens.push(OraclePlSqlToken::QuotedIdentifier);
            continue;
        }

        if ch == ';' {
            tokens.push(OraclePlSqlToken::Semicolon);
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut token = String::new();
            token.push(ch.to_ascii_uppercase());
            while let Some((_, next)) = iter.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' || next == '$' || next == '#' {
                    token.push(next.to_ascii_uppercase());
                    iter.next();
                } else {
                    break;
                }
            }
            tokens.push(OraclePlSqlToken::word(token));
        }
    }

    tokens
}
