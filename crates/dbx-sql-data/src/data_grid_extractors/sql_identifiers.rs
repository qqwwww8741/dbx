use crate::models::connection::DatabaseType;
use crate::sql_dialect::quote_table_data_identifier;
use sqlparser::dialect::{Dialect, GenericDialect, MsSqlDialect};
use sqlparser::tokenizer::{Token, Tokenizer};
use std::collections::HashSet;
use std::sync::OnceLock;

pub(super) fn unquote_optional_identifiers(
    reference: String,
    database_type: Option<DatabaseType>,
    identifier_quote: Option<&str>,
) -> String {
    let dialect: &dyn Dialect = { &GenericDialect {} };
    let Ok(tokens) = Tokenizer::new(dialect, &reference).with_unescape(false).tokenize() else {
        return reference;
    };
    if tokens.iter().any(|token| !matches!(token, Token::Word(_) | Token::Period)) {
        return reference;
    }
    tokens
        .into_iter()
        .map(|token| match token {
            Token::Word(mut word) => {
                if word.quote_style.is_some() && can_unquote(&word.value, database_type, identifier_quote) {
                    word.quote_style = None;
                }
                word.to_string()
            }
            _ => token.to_string(),
        })
        .collect()
}

fn can_unquote(name: &str, database_type: Option<DatabaseType>, identifier_quote: Option<&str>) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    match database_type {
        _ => {
            let simple =
                |ch: char| ch.is_ascii_alphabetic() || matches!(ch, '_' | '$') || ('\u{80}'..='\u{ffff}').contains(&ch);
            let lower = name.to_ascii_lowercase();
            simple(first) && chars.all(|ch| simple(ch) || ch.is_ascii_digit()) && !mysql_reserved(&lower)
        }
    }
}

fn mysql_reserved(name: &str) -> bool {
    static KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    KEYWORDS
        .get_or_init(|| {
            concat!(
                "all analyse analyze and any array as asc asymmetric authorization binary both case cast check ",
                "collate collation column concurrently constraint create cross current_catalog current_date ",
                "current_role current_schema current_time current_timestamp current_user default deferrable desc ",
                "distinct do else end except false fetch for foreign freeze from full grant group having ilike in ",
                "initially inner intersect into is isnull join lateral leading left like limit localtime ",
                "localtimestamp natural not notnull null offset on only or order outer overlaps placing primary ",
                "references returning right select session_user similar some symmetric system_user table tablesample ",
                "then to trailing true union unique user using variadic verbose when where window with accessible ",
                "auto_increment change database databases delayed describe div dual enclosed escaped explain force ",
                "fulltext high_priority ignore index infile key keys kill linear lines load lock low_priority ",
                "master_ssl_verify_server_cert maxvalue mediumint mod no_write_to_binlog optimize optionally outfile ",
                "partition purge range read_write regexp release rename replace require rlike schema schemas ",
                "separator show spatial sql_big_result sql_calc_found_rows sql_small_result ssl starting ",
                "straight_join terminated tinyint unlock unsigned use utc_date utc_time utc_timestamp values ",
                "varbinary varchar write xor zerofill",
            )
            .split_ascii_whitespace()
            .collect()
        })
        .contains(name)
}
