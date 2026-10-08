import type { SQLDialect } from "@codemirror/lang-sql";
import type { DatabaseType } from "@/types/database";
import { driverProfileSqlBuiltinTerms } from "@/lib/database/driverProfileExtensions";

export type CodeMirrorSqlDialectName = "mysql" | "soql";

export function supportsQueryEditorSqlLanguage(_databaseType?: DatabaseType): boolean {
  return true;
}

type CodeMirrorSqlLanguageModule = Pick<typeof import("@codemirror/lang-sql"), "Cassandra" | "MSSQL" | "MySQL" | "PLSQL" | "PostgreSQL" | "SQLite" | "SQLDialect" | "StandardSQL">;

const MYSQL_CODEMIRROR_DATABASE_TYPES = new Set<DatabaseType>(["mysql"]);

// Non-MySQL-wire dialects whose servers still interpret backslash escapes in string
// literals; the mysql-family types are covered by isMysql at the define() site. Mirrors
// BACKSLASH_ESCAPE_STRING_DIALECTS in lib/sql/sqlStatementRanges.ts so the editor
// tokenizer and the statement splitter agree on escape semantics per dialect.

const CODEMIRROR_SQLITE_EXTENSION_KEYWORDS = new Set("abort analyze attach autoincrement conflict database detach exclusive fail glob ignore index indexed instead isnull notnull offset plan pragma query raise regexp reindex rename replace temp vacuum virtual".split(" "));
const STANDARD_SQL_TYPES = "array binary bit boolean char character clob date decimal double float int integer interval large national nchar nclob numeric object precision real smallint time timestamp varchar varying";

const DBX_COMMON_SQL_KEYWORDS = [
  "PIVOT",
  "UNPIVOT",
  "EXCLUDE",
  "REPLACE",
  "QUALIFY",
  "ASOF",
  "POSITIONAL",
  "ANTI",
  "SEMI",
  "SAMPLE",
  "TABLESAMPLE",
  "STRUCT",
  "MAP",
  "LIST",
  "ARRAY",
  "LAMBDA",
  "UNNEST",
  "LATERAL",
  "FILTER",
  "RECURSIVE",
  "SUMMARIZE",
  "MATERIALIZED",
  "PRAGMA",
  "READ_CSV",
  "READ_PARQUET",
  "READ_JSON",
  "DESCRIBE",
  "SHOW",
  "COPY",
  "EXPORT",
  "IMPORT",
].join(" ");

const POSTGRES_PLPGSQL_KEYWORDS = "PERFORM";
const POSTGRES_PLPGSQL_TYPES = "RECORD JSON JSONB";
const POSTGRES_PLPGSQL_BUILTIN = "SQLERRM TG_NAME TG_WHEN TG_LEVEL TG_OP TG_RELID TG_RELNAME TG_TABLE_NAME TG_TABLE_SCHEMA TG_NARGS TG_ARGV";
const POSTGRES_IDENTIFIER_LIKE_KEYWORDS = new Set("COMMENT COUNT DATA DAY HOUR ID KEY LEVEL MINUTE MONTH NAME OWNER PASSWORD POSITION ROLE SECOND TYPE USER VALUE YEAR".split(" "));

// SQL Server table-valued parameters require READONLY in procedure/function declarations.

// CodeMirror's MSSQL builtin list registers a few T-SQL clause words as functions:
// `set` arrives with the query hint terms, while `next`/`for` come from the
// `NEXT VALUE FOR` sequence expression being split into single words. Builtin terms are
// applied after keywords when the dialect vocabulary is built, so those entries shadow the
// reserved-keyword highlighting for statements like `UPDATE ... SET` and `SET NOCOUNT ON`.
// None of them is callable on its own, so drop them and let the keyword classification win.
const SQLSERVER_NON_FUNCTION_BUILTIN_TERMS = new Set(["set", "next", "for"]);

export function sqlServerBuiltinSyntaxTerms(builtin: string): string {
  return builtin
    .split(/\s+/)
    .filter((term) => term && !SQLSERVER_NON_FUNCTION_BUILTIN_TERMS.has(term.toLowerCase()))
    .join(" ");
}

// Salesforce SOQL is a distinct query language: no JOINs, no DDL/DML keywords, no
// identifier quoting, single-quoted strings only, and its own clause/operator set.
// CodeMirror's tokenizer lowercases each word before lookup (lang-sql keywords()),
// so every entry below is stored lowercase to guarantee a match. true/false/null are
// omitted on purpose — lang-sql pre-registers them as Bool/Null tokens.
const SOQL_KEYWORDS = [
  // Clauses & query structure
  "select",
  "from",
  "where",
  "and",
  "or",
  "not",
  "in",
  "like",
  "includes",
  "excludes",
  "order",
  "by",
  "group",
  "having",
  "limit",
  "offset",
  "asc",
  "desc",
  "nulls",
  "first",
  "last",
  "all",
  "distinct",
  // TYPEOF (polymorphic field selection)
  "typeof",
  "when",
  "then",
  "else",
  "end",
  // WITH clauses
  "with",
  "security_enforced",
  "user_mode",
  "system_mode",
  "fields",
  "standard",
  "custom",
  "data",
  "category",
  "using",
  "scope",
  "above",
  "below",
  "above_or_below",
  // FOR clauses (tracking / viewstat)
  "for",
  "view",
  "reference",
  "update",
  "tracking",
  "viewstat",
  // Date literals (the non-parameterized forms; :n variants highlight the stem)
  "yesterday",
  "today",
  "tomorrow",
  "last_week",
  "this_week",
  "next_week",
  "last_month",
  "this_month",
  "next_month",
  "last_90_days",
  "next_90_days",
  "last_n_days",
  "next_n_days",
  "last_quarter",
  "this_quarter",
  "next_quarter",
  "last_n_quarters",
  "next_n_quarters",
  "last_year",
  "this_year",
  "next_year",
  "last_n_years",
  "next_n_years",
  "last_fiscal_quarter",
  "this_fiscal_quarter",
  "next_fiscal_quarter",
  "last_n_fiscal_quarters",
  "next_n_fiscal_quarters",
  "last_fiscal_year",
  "this_fiscal_year",
  "next_fiscal_year",
  "last_n_fiscal_years",
  "next_n_fiscal_years",
].join(" ");

// SOQL aggregate + date/convert functions. Highlighted as builtins (callable names).
const SOQL_BUILTINS = [
  "count",
  "count_distinct",
  "sum",
  "avg",
  "min",
  "max",
  "grouping",
  "convertcurrency",
  "converttimezone",
  "tolabel",
  "format",
  "calendar_month",
  "calendar_quarter",
  "calendar_year",
  "day_in_month",
  "day_in_week",
  "day_in_year",
  "day_only",
  "fiscal_month",
  "fiscal_quarter",
  "fiscal_year",
  "hour_in_day",
  "week_in_month",
  "week_only",
  "month_in_year",
].join(" ");

// COUNT is stripped from Postgres keywords by postgresKeywordSyntaxTerms() (it's a
// valid Postgres identifier name), so it's re-added here as a builtin function instead.
const POSTGRES_BUILTINS = [
  "count",
  "coalesce",
  "nullif",
  "greatest",
  "least",
  "to_char",
  "to_date",
  "to_number",
  "to_timestamp",
  "extract",
  "date_trunc",
  "date_part",
  "now",
  "current_date",
  "current_timestamp",
  "array_agg",
  "string_agg",
  "lower",
  "upper",
  "length",
  "substring",
  "trim",
  "concat",
].join(" ");

const MYSQL_BUILTINS = ["ifnull", "date_format", "str_to_date", "date_add", "date_sub", "curdate", "curtime", "unix_timestamp", "from_unixtime", "group_concat", "concat_ws"].join(" ");

export function postgresKeywordSyntaxTerms(keywords: string): string {
  return keywords
    .split(/\s+/)
    .filter((keyword) => keyword && !POSTGRES_IDENTIFIER_LIKE_KEYWORDS.has(keyword.toUpperCase()))
    .join(" ");
}

function standardSqlKeywordSyntaxTerms(langSql: CodeMirrorSqlLanguageModule): string {
  // CodeMirror keeps StandardSQL's default vocabulary internal and exposes an
  // empty StandardSQL.spec. SQLite is its smallest public standard-SQL
  // superset, so remove SQLite-only terms to retain the standard vocabulary.
  return (langSql.SQLite.spec.keywords || "")
    .split(/\s+/)
    .filter((keyword) => keyword && !CODEMIRROR_SQLITE_EXTENSION_KEYWORDS.has(keyword))
    .join(" ");
}

function codeMirrorBaseDialect(langSql: CodeMirrorSqlLanguageModule, _dialectName: CodeMirrorSqlDialectName, databaseType?: DatabaseType): SQLDialect {
  if (databaseType) {
    {
    }
    if (MYSQL_CODEMIRROR_DATABASE_TYPES.has(databaseType)) return langSql.MySQL;
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    return langSql.StandardSQL;
  }
  {}
  return langSql.MySQL;
}

export function createDbxCodeMirrorSqlDialect(langSql: CodeMirrorSqlLanguageModule, dialectName: CodeMirrorSqlDialectName = "mysql", databaseType?: DatabaseType, driverProfile?: string): SQLDialect {
  // SOQL is its own language, not a SQL superset: define it from a precise keyword
  // set with no identifier quoting, single-quoted strings, and backslash escapes.
  if (dialectName === "soql") {
    return langSql.SQLDialect.define({
      keywords: SOQL_KEYWORDS,
      builtin: [SOQL_BUILTINS, driverProfileSqlBuiltinTerms(driverProfile)].filter(Boolean).join(" ") || undefined,
      backslashEscapes: true,
      doubleQuotedStrings: false,
      caseInsensitiveIdentifiers: true,
      identifierQuotes: "",
      doubleDollarQuotedStrings: false,
    });
  }
  const baseDialect = codeMirrorBaseDialect(langSql, dialectName, databaseType);
  const isPostgres = baseDialect === langSql.PostgreSQL;
  const isMysql = baseDialect === langSql.MySQL;

  const isPlsql = baseDialect === langSql.PLSQL;

  // StandardSQL.spec exposes no vocabulary, so every StandardSQL-based dialect
  // (generic JDBC, IRIS/Caché, H2, DB2, …) needs the reconstructed standard
  // keyword set — without it SELECT/WHERE/AND highlight as plain identifiers.
  const isStandardSql = baseDialect === langSql.StandardSQL;
  const baseKeywords = isStandardSql ? standardSqlKeywordSyntaxTerms(langSql) : isPostgres ? postgresKeywordSyntaxTerms(baseDialect.spec.keywords || "") : baseDialect.spec.keywords || "";
  const baseTypes = isStandardSql ? STANDARD_SQL_TYPES : baseDialect.spec.types || "";
  const commonKeywords = DBX_COMMON_SQL_KEYWORDS;
  const baseBuiltin = baseDialect.spec.builtin || "";

  return langSql.SQLDialect.define({
    ...baseDialect.spec,
    keywords: [baseKeywords, commonKeywords, "", isPostgres ? POSTGRES_PLPGSQL_KEYWORDS : "", ""].filter(Boolean).join(" "),
    types: [baseTypes, "", isPostgres ? POSTGRES_PLPGSQL_TYPES : ""].filter(Boolean).join(" ") || undefined,
    builtin: [baseBuiltin, "", isPostgres ? `${POSTGRES_BUILTINS} ${POSTGRES_PLPGSQL_BUILTIN}` : "", isMysql ? MYSQL_BUILTINS : "", driverProfileSqlBuiltinTerms(driverProfile)].filter(Boolean).join(" ") || undefined,
    // T-SQL temp tables (#local / ##global) otherwise tokenize the leading
    // `#` as a parser error, breaking highlighting for the whole name. The
    // specialVar scanner natively handles the doubled prefix and already
    // covers @@variables, so # joins the same channel for SQL Server (#8267).
    ...{},
    ...{},
    ...(isMysql
      ? {
          backslashEscapes: true,
        }
      : {}),
    ...(isPlsql ? { doubleQuotedStrings: false } : {}),
    doubleDollarQuotedStrings: false,
  });
}
