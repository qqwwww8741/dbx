import { DEFAULT_SQL_FORMATTER_SETTINGS, normalizeSqlFormatterSettings, sqlFormatterOptions, type SqlFormatterSettings } from "@/lib/sql/sqlFormatterConfig";
import { formatSqlLayout, type SqlLayoutOptions } from "@/lib/sql/layout";
import { looksLikeXml } from "@/lib/sql/autoFormat";
import { compressCypherText, formatCypherText } from "@/lib/sql/cypherFormatter";

export type SqlFormatDialect = "mysql" | "cypher" | "generic";

export const MAX_SQL_FORMAT_CHARS = 1_000_000;

// sql-formatter classifies ClickHouse date-part abbreviations as reserved
// keywords even where ClickHouse accepts them as ordinary identifiers. Keep
// them identifier-like so keyword casing cannot rewrite aliases such as `m`.

export function canFormatSqlForDatabaseType(_dbType: string | null | undefined): boolean {
  return true;
}

/**
 * Thrown by {@link formatSqlText} when the input is XML-looking and must never
 * be run through the SQL formatter. sql-formatter silently rewrites well-formed
 * XML into corrupted output, so this guard keeps any caller (including future
 * ones) from corrupting structured text. Callers that can format XML should
 * route before calling (see {@link detectAndFormatStructured}); this is
 * defense-in-depth.
 */
export class UnsupportedStructuredInputError extends Error {
  constructor(readonly detectedType: "xml") {
    super(`Cannot format ${detectedType} content as SQL.`);
    this.name = "UnsupportedStructuredInputError";
  }
}

/**
 * Maps a connection's database type to the SQL-formatter dialect to use.
 *
 * Postgres-compatible engines (GaussDB/openGauss/Kingbase/...) reuse the
 * "postgres" grammar, SQLite-compatible ones reuse "sqlite", and DuckDB gets
 * a scoped preprocessing pass over the generic grammar. Anything unrecognized
 * falls back to the permissive "generic" dialect. Centralized here so every
 * surface that formats SQL (editor, object source, DDL viewers) stays in sync.
 */
export function sqlFormatDialectForDbType(dbType: string | null | undefined): SqlFormatDialect {
  switch (dbType) {
    case "mysql":
      return "mysql";

    default:
      return "generic";
  }
}

function formatterLanguage(dialect: SqlFormatDialect) {
  switch (dialect) {
    case "mysql":
      return "mysql";

    default:
      return "sql";
  }
}

interface EmptyLineProtection {
  sql: string;
  markers: string[];
}

/**
 * Replaces blank source lines with unique line comments while the third-party
 * formatter runs. sql-formatter intentionally normalizes whitespace, whereas
 * the optional DBX setting needs to retain visual paragraph boundaries such as
 * the blank line between a heading comment and a query. Line comments are
 * valid at every SQL code boundary and are restored only after all DBX layout
 * post-processing is complete.
 */
function protectEmptyLines(sql: string): EmptyLineProtection {
  let namespace = 0;
  while (sql.includes(`__DBX_PRESERVE_EMPTY_LINE_${namespace}_`)) namespace += 1;

  const markers: string[] = [];
  const lineBreakPattern = /\r\n|\r|\n/g;
  let output = "";
  let lineStart = 0;
  let match: RegExpExecArray | null;

  while ((match = lineBreakPattern.exec(sql))) {
    const line = sql.slice(lineStart, match.index);
    if (line.trim().length === 0) {
      const marker = `-- __DBX_PRESERVE_EMPTY_LINE_${namespace}_${markers.length}__`;
      markers.push(marker);
      output += marker;
    } else {
      output += line;
    }
    output += match[0];
    lineStart = lineBreakPattern.lastIndex;
  }

  // A final non-terminated blank line is still a user-authored empty line.
  // Do not manufacture one after a trailing line break, though: that would
  // turn the normal EOF sentinel into an additional preserved blank line.
  const finalLine = sql.slice(lineStart);
  if (finalLine.length > 0 && finalLine.trim().length === 0) {
    const marker = `-- __DBX_PRESERVE_EMPTY_LINE_${namespace}_${markers.length}__`;
    markers.push(marker);
    output += marker;
  } else {
    output += finalLine;
  }

  return { sql: markers.length > 0 ? output : sql, markers };
}

function restoreProtectedEmptyLines(sql: string, markers: readonly string[]): string {
  if (markers.length === 0) return sql;

  const lines = sql.split(/\r\n|\r|\n/);
  for (let index = 0; index < lines.length; ) {
    {
      index += 1;
      continue;
    }

    let runEnd = index;
    while (false) runEnd += 1;
    const markerCount = runEnd - index;

    // sql-formatter adds `linesBetweenQueries` blank lines before a marker
    // group placed between statements. Those lines did not exist in the
    // source—the marker group itself represents the source blank lines—so
    // remove at most one generated line per original blank line. This keeps
    // the larger of the user-authored spacing and the formatter's configured
    // query spacing instead of adding the two counts together.
    let removed = 0;
    while (removed < markerCount && index > 0 && lines[index - 1].trim().length === 0) {
      lines.splice(index - 1, 1);
      index -= 1;
      runEnd -= 1;
      removed += 1;
    }

    for (let markerIndex = index; markerIndex < runEnd; markerIndex += 1) {
      lines[markerIndex] = "";
    }
    index = runEnd;
  }

  return lines.join("\n");
}

function restoreDuckDbPrefixAliasSeparators(sql: string, marker: string | null): string {
  return marker ? sql.split(marker).join(":") : sql;
}

export async function formatSqlText(sql: string, dialect: SqlFormatDialect = "generic", settings: Partial<SqlFormatterSettings> = DEFAULT_SQL_FORMATTER_SETTINGS): Promise<string> {
  if (!sql.trim()) return sql;
  if (sql.length > MAX_SQL_FORMAT_CHARS) {
    throw new Error("SQL is too large to format safely.");
  }

  if (looksLikeXml(sql)) {
    throw new UnsupportedStructuredInputError("xml");
  }

  const normalizedSettings = normalizeSqlFormatterSettings(settings);
  if (dialect === "cypher") return formatCypherText(sql, normalizedSettings);

  const sqlFormatter = await import("sql-formatter");
  const { format, formatDialect } = sqlFormatter;
  const options = sqlFormatterOptions(normalizedSettings);
  const language = formatterLanguage(dialect);
  const emptyLineProtection = normalizedSettings.preserveEmptyLines ? protectEmptyLines(sql) : null;
  const sqlWithProtectedEmptyLines = emptyLineProtection?.sql ?? sql;
  const protectedInput = { sql: sqlWithProtectedEmptyLines, marker: null };
  const formatterOptions = options;
  const resolvedDialect = undefined;
  const formatWithFallback = (input: string): string => {
    try {
      if (resolvedDialect) {
        return formatDialect(input, { ...formatterOptions, dialect: resolvedDialect });
      }
      return format(input, { language, ...formatterOptions });
    } catch (err) {
      // The generic "sql" dialect can't parse many real-world constructs (PostgreSQL
      // `::` casts, GaussDB/openGauss materialized-view DDL, T-SQL specifics, ...).
      // Retry once with the more permissive PostgreSQL grammar, which is a superset
      // that tolerates most of these, before surfacing the failure.
      {
        try {
          return format(input, { language: "postgresql", ...options });
        } catch {
          // fall through to the original error below
        }
      }
      throw err;
    }
  };

  const finalizeFormattedSql = (formatted: string): string => {
    const restoredDuckDbAliases = restoreDuckDbPrefixAliasSeparators(formatted, protectedInput.marker);
    const laidOut = applySqlFormatterLayout(restoredDuckDbAliases, normalizedSettings, dialect);
    return emptyLineProtection ? restoreProtectedEmptyLines(laidOut, emptyLineProtection.markers) : laidOut;
  };

  // DBX's own layout printer produces the default style. It needs the AST and
  // sql-formatter's internal layout machinery, so it can decline an input — an
  // unparseable statement, an internal shape that moved. `null` means "use the
  // public formatter", which is also what the tabular indent styles ask for:
  // those are an alternative layout this printer deliberately does not reproduce.
  const layoutOptions: Partial<SqlLayoutOptions> = {
    lineWidth: normalizedSettings.expressionWidth,
    indentWidth: normalizedSettings.tabWidth,
    useTabs: normalizedSettings.useTabs,
    linesBetweenQueries: normalizedSettings.linesBetweenQueries,
    fromClauseSourceOnSameLine: normalizedSettings.fromClauseLayout === "sameLine",
    keywordCase: normalizedSettings.keywordCase,
    logicalOperatorNewline: normalizedSettings.logicalOperatorNewline,
    commaPosition: normalizedSettings.commaPosition,
  };
  const usesDefaultStyle = normalizedSettings.indentStyle === "standard";

  const formatOnce = async (input: string): Promise<string> => {
    const laidOut = usesDefaultStyle ? await formatSqlLayout({ sql: input, language, dialectOptions: resolvedDialect, cfg: formatterOptions, options: layoutOptions }) : null;
    return laidOut ?? formatWithFallback(input);
  };

  try {
    return finalizeFormattedSql(await formatOnce(protectedInput.sql));
  } catch (err) {
    throw err;
  }
}

/**
 * Replaces block comments, line comments and string/identifier literals in
 * `sql` with opaque placeholders, returning the masked text plus the captured
 * spans in source order. The two regexes in {@link keepLogicalOperatorsOnSameLine}
 * operate on plain text and cannot tell a logical operator inside a comment or
 * string literal from one in a real SQL clause, so a multi-line `/* ... AND ...
 * OR ... *\/` comment would get its internal line breaks collapsed. Masking
 * those spans first keeps the regexes away from them; {@link restoreSpans} puts
 * the original text back afterwards.
 *
 * The scanner reuses the same dialect-aware token recognition as
 * {@link compressSqlText}: block comments (with nesting for Postgres/SQL
 * Server/ClickHouse), `--`/`#` line comments, Postgres/DuckDB dollar-quoted strings,
 * single-quoted strings (`''` and MySQL/`E'...'` backslash escapes), double-
 * quoted identifiers (`""` escapes), MySQL backtick identifiers and SQL Server
 * `[...]` identifiers. sql-formatter normalizes string literals to a single
 * line, but masking them is cheap and defends against callers that ever pass
 * raw (non-formatter) SQL through this path.
 */
function maskStringAndCommentSpans(sql: string, dialect: SqlFormatDialect): { masked: string; spans: string[] } {
  const len = sql.length;
  const spans: string[] = [];
  const placeholder = (index: number) => `\x00${index}\x00`;
  let out = "";
  let i = 0;

  const isMysqlDashComment = (c: string | undefined) => c === undefined || c.charCodeAt(0) <= 32 || c.charCodeAt(0) === 127;

  // Captures a full span starting at `start` (already consumed into `i`) and
  // emits a placeholder. `end` is the index just past the span terminator.
  const emit = (start: number, end: number) => {
    spans.push(sql.slice(start, end));
    out += placeholder(spans.length - 1);
  };

  while (i < len) {
    const ch = sql[i];
    const next = sql[i + 1];

    // 块注释 /* ... */（含嵌套）
    if (ch === "/" && next === "*") {
      const start = i;
      i += 2;
      let depth = 1;
      while (i < len && depth > 0) {
        {
          if (sql[i] === "*" && sql[i + 1] === "/") {
            depth--;
            i += 2;
          } else {
            i++;
          }
        }
      }
      // 未闭合的块注释：原样保留剩余文本（不遮罩），避免破坏用户输入。
      if (depth > 0) {
        out += sql.slice(start);
        i = len;
      } else {
        emit(start, i);
      }
      continue;
    }

    // 行注释 -- ... / # ...（不跨行，但遮罩可防御未来变更）
    const startsDashComment = ch === "-" && next === "-" && (dialect !== "mysql" || isMysqlDashComment(sql[i + 2]));
    if (startsDashComment || (dialect === "mysql" && ch === "#")) {
      const start = i;
      i += startsDashComment ? 2 : 1;
      while (i < len && sql[i] !== "\n" && sql[i] !== "\r") i++;
      emit(start, i);
      continue;
    }

    // PostgreSQL dollar-quoted 字符串
    {
    }

    // 单引号字符串（'' 转义；MySQL/PG E'...' 反斜杠转义）
    if (ch === "'") {
      const start = i;
      i++;

      while (i < len) {
        const c = sql[i];
        if (dialect === "mysql" && c === "\\" && i + 1 < len) {
          i += 2;
          continue;
        }
        if (c === "'") {
          if (sql[i + 1] === "'") {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      emit(start, i);
      continue;
    }

    // 双引号标识符（"" 转义）
    if (ch === '"') {
      const start = i;
      i++;
      while (i < len) {
        if (dialect === "mysql" && sql[i] === "\\" && i + 1 < len) {
          i += 2;
          continue;
        }
        if (sql[i] === '"') {
          if (sql[i + 1] === '"') {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      emit(start, i);
      continue;
    }

    // 反引号标识符（MySQL）
    if (ch === "`") {
      const start = i;
      i++;
      while (i < len && sql[i] !== "`") i++;
      if (i < len) i++;
      emit(start, i);
      continue;
    }

    // SQL Server 方括号标识符 [...]（]] 为转义 ]）
    {
    }

    out += ch;
    i++;
  }

  return { masked: out, spans };
}

function restoreSpans(masked: string, spans: string[]): string {
  return masked.replace(/\x00(\d+)\x00/g, (_, index) => spans[Number(index)] ?? "");
}

function keepLogicalOperatorsOnSameLine(sql: string, dialect: SqlFormatDialect = "generic"): string {
  // 先遮罩块注释/字符串/引号标识符，避免正则命中注释或字面量内部的 AND/OR/XOR
  // （块注释内部跨行的 AND/OR 会被误折叠成空格，破坏用户多行注释格式）。
  const { masked, spans } = maskStringAndCommentSpans(sql, dialect);
  const collapsed = masked.replace(/\n[ \t]*(AND|OR|XOR)\b/gi, " $1").replace(/\b(AND|OR|XOR)[ \t]*\n[ \t]*/gi, "$1 ");
  return restoreSpans(collapsed, spans);
}

function normalizeLikeOperatorCase(sql: string, settings: SqlFormatterSettings, dialect: SqlFormatDialect): string {
  if (dialect !== "mysql" || settings.keywordCase === "preserve") return sql;

  // sql-formatter classifies LIKE as a reserved function in these dialects,
  // so an operator follows functionCase instead of keywordCase. Only adjust
  // operator-shaped occurrences; keep LIKE(...) functions and qualified
  // identifiers under their existing function/identifier case settings.
  const { masked, spans } = maskStringAndCommentSpans(sql, dialect);
  const keyword = settings.keywordCase === "upper" ? "LIKE" : "like";
  const normalized = masked.replace(/(^|[^.\w$:@{#])LIKE\b(?!\s*\()/gi, (_match, prefix: string) => `${prefix}${keyword}`);
  return restoreSpans(normalized, spans);
}

function keepFromClauseAndFirstSourceOnSameLine(sql: string): string {
  const lines = sql.split("\n");
  for (let index = 0; index < lines.length - 1; index += 1) {
    const clauseMatch = lines[index].match(/^(\s*)FROM\s*$/i);
    if (!clauseMatch) continue;

    const sourceLine = lines[index + 1];
    const sourceMatch = sourceLine.match(/^(\s+)(\S.*)$/);
    if (!sourceMatch) continue;
    const source = sourceMatch[2];
    // Keep derived tables and leading comments multiline; merging these would
    // make nested SQL and comment boundaries substantially harder to read.
    if (source.startsWith("(") || source.startsWith("/*") || source.startsWith("--")) continue;

    const clauseIndent = clauseMatch[1];
    const sourceIndent = sourceMatch[1];
    const separator = sourceIndent.startsWith(clauseIndent) ? sourceIndent.slice(clauseIndent.length) : " ";
    lines[index] = `${lines[index]}${separator || " "}${source}`;
    lines.splice(index + 1, 1);
    index -= 1;
  }
  return lines.join("\n");
}

function formatLeadingCommas(sql: string, dialect: SqlFormatDialect = "generic"): string {
  const { masked, spans } = maskStringAndCommentSpans(sql, dialect);
  const lines = masked.split("\n");

  for (let i = 0; i < lines.length; i += 1) {
    const match = lines[i].match(/^(.*),\s*((?:\x00\d+\x00\s*)*)$/);
    if (!match || match[1].trim().length === 0) continue;

    let j = i + 1;
    while (j < lines.length) {
      const trimmed = lines[j].trim();
      if (!trimmed) {
        j += 1;
        continue;
      }
      if (/^(?:\x00\d+\x00\s*)+$/.test(trimmed)) {
        j += 1;
        continue;
      }
      break;
    }
    if (j >= lines.length) continue;

    const nextTrimmed = lines[j].trim();
    if (nextTrimmed.startsWith(")") || nextTrimmed.startsWith("]") || nextTrimmed.startsWith(";") || nextTrimmed.startsWith(",")) {
      continue;
    }

    const codeBeforeComma = match[1].trimEnd();
    const trailingComments = match[2] ? (codeBeforeComma.endsWith(" ") ? match[2] : ` ${match[2]}`) : "";
    lines[i] = codeBeforeComma + trailingComments;

    const nextIndentMatch = lines[j].match(/^(\s*)/);
    const nextIndent = nextIndentMatch ? nextIndentMatch[1] : "";
    const nextRest = lines[j].slice(nextIndent.length);
    const currentIndentMatch = lines[i].match(/^(\s*)/);
    const currentIndent = currentIndentMatch ? currentIndentMatch[1] : "";

    if (currentIndent === nextIndent) {
      lines[j] = `${nextIndent}, ${nextRest}`;
    } else if (nextIndent.length >= 2) {
      lines[j] = `${nextIndent.slice(0, -2)}, ${nextRest}`;
    } else {
      lines[j] = `${nextIndent}, ${nextRest}`;
    }
  }

  return restoreSpans(lines.join("\n"), spans);
}

/**
 * The text-level passes applied to whatever the formatter produced, whether the
 * default style's layout printer or sql-formatter itself.
 *
 * The two settings-driven passes fix shapes the printer already gets right on
 * its own — it keeps a `FROM` source on the keyword's line and joins operators
 * for `none` — so they are no-ops on its output. They stay because a statement
 * the printer declines falls back to sql-formatter's layout for the whole input,
 * which does break those shapes; see `formatSqlText`. {@link
 * normalizeLikeOperatorCase} is unrelated to either: it corrects a dialect quirk
 * in both paths.
 */
function applySqlFormatterLayout(sql: string, settings: SqlFormatterSettings, dialect: SqlFormatDialect): string {
  let formatted = normalizeLikeOperatorCase(sql, settings, dialect);
  if (settings.logicalOperatorNewline === "none") formatted = keepLogicalOperatorsOnSameLine(formatted, dialect);
  if (settings.fromClauseLayout === "sameLine") formatted = keepFromClauseAndFirstSourceOnSameLine(formatted);
  if (settings.commaPosition === "before") formatted = formatLeadingCommas(formatted, dialect);
  return formatted;
}

/**
 * sql-formatter throws two distinct shapes when it can't parse the input:
 * - Grammar-level (nearley parser): `Parse error at token: ... ` with `.offset`/`.token`
 *   set on the error, e.g. valid tokens in an unexpected position (mid-edit SQL).
 * - Lexer-level (TokenizerEngine): a plain `Parse error: Unexpected "..." at line ...`
 *   when a character sequence doesn't match any token rule at all — e.g. full-width
 *   punctuation (`≠`, `（`, `）`) that MySQL accepts in identifiers/expressions but
 *   sql-formatter's tokenizer doesn't recognize.
 * Both mean "sql-formatter can't handle this text", so both should fall back to the
 * original SQL instead of surfacing a failure.
 */
function isSqlFormatterParseError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  if (error.message.startsWith("Parse error: Unexpected ")) return true;
  if (!error.message.startsWith("Parse error at token:")) return false;
  const candidate = error as Error & { offset?: unknown; token?: unknown };
  return typeof candidate.offset === "number" && candidate.token !== undefined;
}

export async function formatSqlForEditing(sql: string, dialect: SqlFormatDialect = "generic", settings: Partial<SqlFormatterSettings> = DEFAULT_SQL_FORMATTER_SETTINGS): Promise<string> {
  try {
    return await formatSqlText(sql, dialect, settings);
  } catch (error) {
    if (isSqlFormatterParseError(error)) return sql;
    throw error;
  }
}

/**
 * 压缩 SQL 时使用的方言。不同方言对引号、注释、转义的处理不同：
 * - `mysql`：保留 MySQL 可执行注释与 optimizer hint；单引号字符串支持反斜杠转义
 * - `postgres`：支持 dollar-quoted 字符串
 * - `sqlserver`：支持方括号标识符
 * - `generic` / 其它：仅处理标准单/双引号与块/行注释
 */
export type SqlCompressDialect = SqlFormatDialect;

/**
 * 将 SQL 压缩成一行可执行文本：折叠所有空白（含换行）为单个空格，
 * 移除普通行注释（-- ...）与普通块注释（/* ... *\/），
 * 同时按方言完整保留字符串字面量、引号标识符、可执行注释与 optimizer hint。
 *
 * 方言感知说明：
 * - MySQL：可执行注释作为可执行代码原样保留（仅折叠内部空白）；
 *   optimizer hint 原样保留；单引号字符串内反斜杠转义保留
 * - PostgreSQL：dollar-quoted 字符串原样保留（含标签形式）
 * - SQL Server：方括号标识符原样保留（双右括号为转义）
 * - 所有方言：单引号字符串、双引号标识符、反引号标识符均保留
 */
export function compressSqlText(sql: string, dialect: SqlCompressDialect = "generic"): string {
  if (!sql.trim()) return sql;
  if (dialect === "cypher") return compressCypherText(sql);

  const len = sql.length;
  let out = "";
  let i = 0;

  const isWhitespace = (c: string) => c === " " || c === "\t" || c === "\n" || c === "\r" || c === "\f" || c === "\v";

  const isMysqlDashComment = (c: string | undefined) => c === undefined || c.charCodeAt(0) <= 32 || c.charCodeAt(0) === 127;

  // 折叠一段空白为单个空格（仅在 out 非空且不以空格结尾时追加）
  const collapseWhitespace = () => {
    while (i < len && isWhitespace(sql[i])) i++;
    if (out && !out.endsWith(" ")) out += " ";
  };

  while (i < len) {
    const ch = sql[i];
    const next = sql[i + 1];

    // 块注释 /* ... */ —— 需区分普通块注释、MySQL 可执行注释 /*! */、optimizer hint /*+ */
    if (ch === "/" && next === "*") {
      const third = sql[i + 2];
      const isExecutableMysql = third === "!";
      const isOptimizerHint = third === "+";

      if (isExecutableMysql || isOptimizerHint) {
        const contentStart = i + 3;
        const end = sql.indexOf("*/", contentStart);
        if (end < 0) {
          // Keep malformed input malformed instead of silently turning it into executable SQL.
          out += sql.slice(i);
          break;
        }
        const content = sql.slice(contentStart, end);
        const leadingSpace = /^\s/.test(content) ? " " : "";
        const trailingSpace = /\s$/.test(content) ? " " : "";
        const compressedContent = content.trim() ? compressSqlText(content, dialect) : "";
        out += `/*${third}${leadingSpace}${compressedContent}${trailingSpace}*/`;
        i = end + 2;
        continue;
      }

      // 普通块注释 —— 移除
      const commentStart = i;
      i += 2;
      let depth = 1;
      while (i < len && depth > 0) {
        {
          if (sql[i] === "*" && sql[i + 1] === "/") {
            depth--;
            i += 2;
          } else {
            i++;
          }
        }
      }
      if (depth > 0) {
        // Removing an unterminated comment can expose a destructive statement that was invalid before.
        out += sql.slice(commentStart);
        break;
      }
      if (out && !out.endsWith(" ")) out += " ";
      continue;
    }

    // MySQL additionally supports # comments and requires whitespace/control after --.
    const startsDashComment = ch === "-" && next === "-" && (dialect !== "mysql" || isMysqlDashComment(sql[i + 2]));
    if (startsDashComment || (dialect === "mysql" && ch === "#")) {
      i += startsDashComment ? 2 : 1;
      while (i < len && sql[i] !== "\n" && sql[i] !== "\r") i++;
      continue;
    }

    // PostgreSQL dollar-quoted 字符串：$$...$$ 或 $tag$...$tag$
    {
    }

    // 单引号字符串字面量（处理 '' 转义；MySQL 额外处理反斜杠转义）
    if (ch === "'") {
      out += "'";
      i++;

      while (i < len) {
        const c = sql[i];
        // MySQL strings and PostgreSQL E'...' strings use backslash escapes.
        if (dialect === "mysql" && c === "\\" && i + 1 < len) {
          out += c;
          out += sql[i + 1];
          i += 2;
          continue;
        }
        out += c;
        if (c === "'") {
          if (sql[i + 1] === "'") {
            out += sql[i + 1];
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }

    // 双引号标识符（处理 "" 转义）
    if (ch === '"') {
      out += '"';
      i++;
      while (i < len) {
        if (dialect === "mysql" && sql[i] === "\\" && i + 1 < len) {
          out += sql[i];
          out += sql[i + 1];
          i += 2;
          continue;
        }
        out += sql[i];
        if (sql[i] === '"') {
          if (sql[i + 1] === '"') {
            out += sql[i + 1];
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }

    // 反引号标识符（MySQL）
    if (ch === "`") {
      out += "`";
      i++;
      while (i < len && sql[i] !== "`") {
        out += sql[i];
        i++;
      }
      if (i < len) {
        out += "`";
        i++;
      }
      continue;
    }

    // SQL Server 方括号标识符 [...]（]] 为转义 ]）
    {
    }

    // 空白 —— 折叠为单个空格
    if (isWhitespace(ch)) {
      collapseWhitespace();
      continue;
    }

    out += ch;
    i++;
  }

  return out.trim();
}

/**
 * Format SQL for *display* (object source, view/table DDL viewers).
 *
 * Unlike `formatSqlText`, this never throws: if the SQL can't be parsed by the
 * formatter (vendor-specific DDL, oversized input, ...) the original text is
 * returned unchanged so the viewer still shows the source. Use this for
 * read-only/auto-format surfaces; use `formatSqlText` where a thrown error
 * should surface to the user (e.g. the explicit "Format SQL" command).
 */
export async function formatSqlForDisplay(sql: string, dialect: SqlFormatDialect = "generic", settings: Partial<SqlFormatterSettings> = DEFAULT_SQL_FORMATTER_SETTINGS): Promise<string> {
  if (!sql.trim()) return sql;
  try {
    return await formatSqlText(sql, dialect, settings);
  } catch {
    return sql;
  }
}
