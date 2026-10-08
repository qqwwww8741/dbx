import type { SqlExecutionCandidate } from "@/lib/sql/sqlExecutionTarget";
import { cursorBelongsToTrailingStatementDelimiter } from "@/lib/sql/statementDelimiter";

import { readSqlBracedParameterAt, type SqlParameterOptions } from "@/lib/sql/sqlParameters";
import { type DatabaseType } from "@/types/database";

/**
 * A contiguous range of SQL text expressed as document offsets plus the
 * extracted (original) substring.
 */
export interface SqlTextRange {
  from: number;
  to: number;
  sql: string;
}

export function elasticsearchRestRequestRanges(_sql: string, _databaseType?: DatabaseType): SqlTextRange[] {
  {
    return [];
  }
}

export function supportsExecutionTargetPicker(databaseType?: DatabaseType): boolean {
  return !!databaseType;
}

/** Remove the MySQL CLI's trailing vertical-output command before execution. */
export function stripMysqlClientDisplayCommand(sql: string): string {
  const trimmed = sql.trimEnd();
  const hasTrailingSemicolon = trimmed.endsWith(";");
  const withoutTrailingSemicolon = hasTrailingSemicolon ? trimmed.slice(0, -1).trimEnd() : trimmed;
  if (!withoutTrailingSemicolon.endsWith("\\G") && !withoutTrailingSemicolon.endsWith("\\g")) return sql;

  const markerStart = withoutTrailingSemicolon.length - 2;
  const lineStart = withoutTrailingSemicolon.lastIndexOf("\n", markerStart - 1) + 1;
  const linePrefix = withoutTrailingSemicolon.slice(lineStart, markerStart);
  if (linePrefix.includes("--") || linePrefix.includes("#")) return sql;

  const executableSql = withoutTrailingSemicolon.slice(0, markerStart).trimEnd();
  return `${executableSql}${hasTrailingSemicolon ? ";" : ""}${sql.slice(trimmed.length)}`;
}

export function hasMultipleExecutionTargets(sql: string, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  {}
  return splitSqlStatementRanges(sql, databaseType, parameterOptions).length > 1;
}

interface RawStatement {
  /** Start offset (inclusive) of whitespace that can still target this statement. */
  hitFrom: number;
  /** Start offset (inclusive) of the statement's first non-whitespace char. */
  from: number;
  /** End offset (exclusive) — up to and excluding the terminating semicolon. */
  to: number;
  /** The statement text, sliced from the source document. */
  sql: string;
}

export interface ElasticsearchRestRequestTarget {
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD";
  path: string;
}

function leadingElasticsearchPreambleEnd(value: string): number {
  let offset = 0;
  while (offset < value.length) {
    while (offset < value.length && /\s/.test(value[offset] ?? "")) offset += 1;
    if (offset >= value.length) return offset;

    if (value[offset] === "#" || value.startsWith("//", offset)) {
      const newline = value.indexOf("\n", offset);
      offset = newline < 0 ? value.length : newline + 1;
      continue;
    }

    if (value.startsWith("/*", offset)) {
      const close = value.indexOf("*/", offset + 2);
      offset = close < 0 ? value.length : close + 2;
      continue;
    }

    break;
  }
  return offset;
}

export function stripLeadingElasticsearchComments(value: string): string {
  return value.slice(leadingElasticsearchPreambleEnd(value)).trimStart();
}

export function parseElasticsearchRestRequestTarget(value: string): ElasticsearchRestRequestTarget | null {
  const requestLine = stripLeadingElasticsearchComments(value).split("\n", 1)[0]?.trim() ?? "";
  const match = requestLine.match(/^(GET|POST|PUT|PATCH|DELETE|HEAD)\s+(\S+)/i);
  if (!match) return null;
  return {
    method: match[1].toUpperCase() as ElasticsearchRestRequestTarget["method"],
    path: match[2].startsWith("/") ? match[2] : `/${match[2]}`,
  };
}

export function isElasticsearchRestRequestText(value: string): boolean {
  return parseElasticsearchRestRequestTarget(value) !== null;
}

type QuoteState = "none" | "single" | "double" | "backtick" | "bracket" | "dollar";

const COMMON_SOFT_STATEMENT_START_KEYWORDS = [
  "SELECT",
  "WITH",
  "CREATE",
  "ALTER",
  "DROP",
  "INSERT",
  "UPDATE",
  "DELETE",
  "MERGE",
  "REPLACE",
  "TRUNCATE",
  "GRANT",
  "REVOKE",
  "COMMENT",
  "EXPLAIN",
  "SHOW",
  "DESCRIBE",
  "DESC",
  "USE",
  "SET",
  "CALL",
  "EXEC",
  "EXECUTE",
  "BEGIN",
  "COMMIT",
  "ROLLBACK",
  "DECLARE",
  "ANALYZE",
  "VACUUM",
  "PRAGMA",
  "REFRESH",
  "COPY",
] as const;

const SOFT_STATEMENT_FUNCTION_KEYWORDS = new Set(["REPLACE", "TRUNCATE"]);

const DATABASE_SOFT_STATEMENT_KEYWORDS: Partial<Record<DatabaseType, readonly string[]>> = {
  mysql: ["HANDLER", "LOAD", "OPTIMIZE", "REPAIR"],
};

const WITH_MAIN_STATEMENT_KEYWORDS = new Set(["SELECT", "INSERT", "UPDATE", "DELETE", "MERGE"]);
const EXPLAIN_STATEMENT_KEYWORDS = new Set(["SELECT", "WITH", "INSERT", "UPDATE", "DELETE", "MERGE", "CREATE", "ALTER", "DROP"]);
const CREATE_BODY_KEYWORDS = new Set(["SELECT", "WITH", "BEGIN", "DECLARE"]);

const INSERT_BODY_KEYWORDS = new Set(["SELECT", "WITH"]);
const ALTER_BODY_KEYWORDS = new Set(["ADD", "ALTER", "COMMENT", "DROP", "MODIFY", "RENAME", "SET"]);

const SET_OPERATION_KEYWORDS = new Set(["UNION", "INTERSECT", "EXCEPT", "MINUS"]);
const SET_OPERATION_MODIFIER_KEYWORDS = new Set(["ALL", "DISTINCT"]);
// Mirrors the backend list in dbx-core/src/sql.rs is_oracle_like_database — keep both
// in sync. ArgoDB (Transwarp Hive/Inceptor fork) ships a PL/SQL-compatible procedure
// language (`CREATE [OR REPLACE] PROCEDURE ... IS BEGIN ... END;`), so its statement
// ranges must stay whole instead of splitting at every body semicolon.

const MYSQL_ROUTINE_BLOCK_DATABASES: ReadonlySet<DatabaseType> = new Set(["mysql"]);
// PostgreSQL/openGauss are also the connection types users pick for GaussDB/openGauss instances
// running in Oracle (A) compatibility mode, where a routine body is written in Oracle style
// (`CREATE PROCEDURE p AS DECLARE ... BEGIN ... END;`) and closed by a standalone `/` line.
// Mirrors SqlDialectProfile::postgres_family in dbx-sql — keep both in sync.

// Backslash escaping inside '...'/"..." strings is a MySQL-family extension; in standard SQL '\'
// is a complete one-char string and quotes are escaped by doubling (''). Treating backslash as an
// escape unconditionally makes ESCAPE '\' swallow its closing quote and the following statement
// boundary, so the next statement loses its run button (#8189). Gate it by dialect, matching the
// tokenizer/completion side.
export const BACKSLASH_ESCAPE_STRING_DIALECTS: ReadonlySet<DatabaseType> = new Set(["mysql"]);
function allowsBackslashStringEscape(databaseType?: DatabaseType): boolean {
  return !!databaseType && BACKSLASH_ESCAPE_STRING_DIALECTS.has(databaseType);
}
const MYSQL_CREATE_TABLE_OPTION_DATABASES: ReadonlySet<DatabaseType> = new Set(["mysql"]);
const MYSQL_ROUTINE_OBJECT_TYPES = new Set(["PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"]);
const MYSQL_NON_ROUTINE_CREATE_TYPES = new Set(["DATABASE", "INDEX", "LOGFILE", "ROLE", "SCHEMA", "SERVER", "SPATIAL", "TABLE", "TEMPORARY", "UNIQUE", "USER", "VIEW"]);
const MYSQL_CONTROL_BLOCK_SUFFIXES = new Set(["IF", "LOOP", "CASE", "REPEAT", "WHILE"]);

// Plain CREATE TYPE ... AS OBJECT (...); ends with ");" and is not a PL/SQL block.
// Only PACKAGE (spec), PACKAGE/TYPE BODY, and routine/trigger objects are PL/SQL blocks.

/**
 * Parse the SQL document into top-level statement ranges delimited by `;`.
 *
 * Delimiters inside string literals, double/backtick/bracket quoted
 * identifiers, dollar-quoted bodies (Postgres), line comments (`--`, `#`) and
 * block comments (`/* *​/`) are ignored, mirroring the backend splitter in
 * `dbx-core/src/sql.rs`. Ranges are returned as `[from, to)` offsets covering
 * only the statement text (the trailing semicolon and inter-statement
 * whitespace are excluded so editor highlights stay tight).
 */
export function splitSqlStatementRanges(sql: string, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): RawStatement[] {
  {}

  const statements: RawStatement[] = [];
  const len = sql.length;
  const supportsDelimiterCommands = databaseType === "mysql";
  const backslashEscapes = allowsBackslashStringEscape(databaseType);

  let statementStart = -1;
  let statementEnd = -1;
  let statementHitStart = 0;
  let pendingHintStart = -1;
  let pendingMysqlDirectiveStart = -1;
  let pendingMysqlDirectiveLineEnd = -1;
  let customDelimiter: string | null = null;
  let state: QuoteState = "none";
  let dollarTag = "";

  let i = 0;

  // Incremental cache for the MySQL routine-block check below: without it, every
  // semicolon inside a CREATE PROCEDURE/FUNCTION/TRIGGER body (which, unlike a
  // DELIMITER-wrapped body, still contains ordinary internal semicolons) would
  // re-tokenize the whole prefix back to statementStart from scratch, and re-walk
  // the whole BEGIN/CASE/END nesting from scratch, turning a single edit to an
  // N-statement routine body into O(N^2) work (see mysqlRoutineBlockCompleteness).
  let mysqlRoutineScan: {
    from: number;
    scannedTo: number;
    lexState: MysqlRoutineLexState;
    tokens: MysqlRoutineToken[];
    completeness: MysqlRoutineBlockCompleteness;
  } | null = null;
  const mysqlRoutineTokensUpTo = (to: number): MysqlRoutineToken[] => {
    if (!mysqlRoutineScan || mysqlRoutineScan.from !== statementStart) {
      mysqlRoutineScan = { from: statementStart, scannedTo: statementStart, lexState: "none", tokens: [], completeness: newMysqlRoutineBlockCompleteness() };
    }
    if (to > mysqlRoutineScan.scannedTo) {
      const { tokens: newTokens, endState } = mysqlRoutineTokens(sql, parameterOptions, mysqlRoutineScan.scannedTo, to, mysqlRoutineScan.lexState);
      mysqlRoutineScan.tokens.push(...newTokens);
      mysqlRoutineScan.lexState = endState;
      mysqlRoutineScan.scannedTo = to;
    }
    return mysqlRoutineScan.tokens;
  };
  const mysqlRoutineBlockIsCompleteUpTo = (to: number): boolean => {
    const tokens = mysqlRoutineTokensUpTo(to);
    return advanceMysqlRoutineBlockCompleteness(mysqlRoutineScan!.completeness, tokens);
  };

  const isWhitespace = (ch: string) => ch === " " || ch === "\t" || ch === "\r" || ch === "\n";
  const markContent = (pos: number) => {
    if (statementStart === -1) {
      // TDSQL accepts arbitrary leading block directives on the SQL line. The
      // exact /*proxy*/ directive also keeps its historical multiline support.
      const directiveStart = pendingMysqlDirectiveStart !== -1 && (sql.startsWith("/*proxy*/", pendingMysqlDirectiveStart) || pos < pendingMysqlDirectiveLineEnd) ? pendingMysqlDirectiveStart : -1;
      statementStart = directiveStart !== -1 ? directiveStart : pendingHintStart === -1 ? pos : pendingHintStart;
      pendingHintStart = -1;
      pendingMysqlDirectiveStart = -1;
      pendingMysqlDirectiveLineEnd = -1;
    }
    statementEnd = pos + 1;
  };

  const flush = (to = statementEnd) => {
    if (statementStart === -1) {
      statementEnd = -1;
      pendingHintStart = -1;
      pendingMysqlDirectiveStart = -1;
      pendingMysqlDirectiveLineEnd = -1;
      return;
    }
    const trimmedTo = trimRangeEnd(sql, statementStart, to);
    if (trimmedTo > statementStart) {
      statements.push({ hitFrom: statementHitStart, from: statementStart, to: trimmedTo, sql: sql.slice(statementStart, trimmedTo) });
    }
    statementStart = -1;
    statementEnd = -1;
    pendingHintStart = -1;
    pendingMysqlDirectiveStart = -1;
    pendingMysqlDirectiveLineEnd = -1;
    mysqlRoutineScan = null;
  };

  while (i < len) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "dollar") {
      // Inside a Postgres dollar-quoted body; look for the closing $tag$.
      if (ch === "$") {
        const closingTag = `$${dollarTag}$`;
        if (sql.startsWith(closingTag, i)) {
          markContent(i);
          for (let k = 0; k < closingTag.length; k += 1) {
            markContent(i + k);
          }
          i += closingTag.length;
          state = "none";
          dollarTag = "";
          continue;
        }
      }
      markContent(i);
      i += 1;
      continue;
    }

    if (state === "single") {
      markContent(i);
      // Only MySQL-family dialects treat backslash as an escape inside '...' (see
      // BACKSLASH_ESCAPE_STRING_DIALECTS); in standard SQL '\' is a literal char and must not
      // consume the next char, otherwise the closing quote is swallowed (#8189).
      if (ch === "\\" && next && backslashEscapes) {
        i += 2;
        continue;
      }
      if (ch === "'") {
        // Doubled single quote '' is an escaped quote, not a terminator.
        if (next === "'") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "double") {
      markContent(i);
      if (ch === '"') {
        if (next === '"') {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "backtick") {
      markContent(i);
      if (ch === "`") {
        if (next === "`") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "bracket") {
      markContent(i);
      if (ch === "]") {
        state = "none";
      }
      i += 1;
      continue;
    }

    // state === "none"
    if (supportsDelimiterCommands && isAtLineStart(sql, i) && startsDelimiterCommand(sql, i)) {
      const lineEnd = findLineEnd(sql, i);
      const delimiter = parseDelimiterCommand(sql.slice(i, lineEnd));
      if (delimiter !== null) {
        flush();
        customDelimiter = delimiter === ";" ? null : delimiter;
        i = nextLineStart(sql, lineEnd);
        statementHitStart = i;
        continue;
      }
    }
    {
    }

    // Line comments consume up to (and including) the newline.
    if (ch === "-" && next === "-") {
      pendingMysqlDirectiveStart = -1;
      pendingMysqlDirectiveLineEnd = -1;
      const newline = sql.indexOf("\n", i);
      i = newline === -1 ? len : newline + 1;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      pendingMysqlDirectiveStart = -1;
      pendingMysqlDirectiveLineEnd = -1;
      const newline = sql.indexOf("\n", i);
      i = newline === -1 ? len : newline + 1;
      continue;
    }
    // Block comments consume until the closing */.
    if (ch === "/" && next === "*") {
      const hintMarker = sql[i + 2];
      if (statementStart === -1 && pendingHintStart === -1 && (hintMarker === "+" || hintMarker === "@" || hintMarker === "&")) pendingHintStart = i;
      const close = sql.indexOf("*/", i + 2);
      if (statementStart === -1) {
        if (databaseType === "mysql") {
          if (pendingMysqlDirectiveStart === -1 || i > pendingMysqlDirectiveLineEnd) {
            pendingMysqlDirectiveStart = i;
            pendingMysqlDirectiveLineEnd = findLineEnd(sql, i);
          }
        } else {
          pendingMysqlDirectiveStart = -1;
          pendingMysqlDirectiveLineEnd = -1;
        }
      }
      i = close === -1 ? len : close + 2;
      continue;
    }

    {
    }

    if (ch === "'") {
      markContent(i);
      state = "single";
      i += 1;
      continue;
    }
    if (ch === '"') {
      markContent(i);
      state = "double";
      i += 1;
      continue;
    }
    if (ch === "`") {
      markContent(i);
      state = "backtick";
      i += 1;
      continue;
    }
    if (ch === "[") {
      markContent(i);
      state = "bracket";
      i += 1;
      continue;
    }
    // Postgres dollar quoting: $tag$ ... $tag$ (tag may be empty, i.e. $$)
    if (!customDelimiter && ch === "$") {
      const tagMatch = /^\$[A-Za-z_0-9]*\$/.exec(sql.slice(i));
      if (tagMatch) {
        markContent(i);
        {
        }
        dollarTag = tagMatch[0].slice(1, -1);
        i += tagMatch[0].length;
        state = "dollar";
        continue;
      }
    }

    if (customDelimiter) {
      if (sql.startsWith(customDelimiter, i)) {
        flush(i);
        i += customDelimiter.length;
        statementHitStart = i;
        continue;
      }
    } else if (ch === ";") {
      const routineTokensBeforeSemicolon = isMysqlRoutineBlockDatabase(databaseType) && statementStart !== -1 ? mysqlRoutineTokensUpTo(i) : null;
      const isMysqlRoutineBlock = routineTokensBeforeSemicolon !== null && isMysqlRoutineDdlStartFromWords(mysqlRoutineDdlStartWords(routineTokensBeforeSemicolon)) && mysqlRoutineTokensContainBegin(routineTokensBeforeSemicolon);
      if (isMysqlRoutineBlock) {
        if (!mysqlRoutineBlockIsCompleteUpTo(i + 1)) {
          markContent(i);
          i += 1;
          continue;
        }
        // The final semicolon is the client-side statement delimiter.
        // Internal semicolons remain part of the routine body.
        flush();
      } else {
        {
        }

        {
          flush();
        }
      }
      statementHitStart = i + 1;
      i += 1;
      continue;
    }

    if (!isWhitespace(ch)) {
      markContent(i);
    }
    i += 1;
  }

  // Flush any trailing statement that lacks a terminating semicolon.
  flush();

  {}
  return statements;
}

/**
 * Returns the statement that contains `cursorPos`, or `null` when the cursor
 * sits on a blank line or no statement can be resolved.
 *
 * The returned range covers only the statement's own text (no trailing `;`),
 * which lets the editor highlight a tight preview range.
 */
export function statementRangeAtCursor(sql: string, cursorPos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): SqlTextRange | null {
  const pos = clampCursor(sql, cursorPos);
  if (isCursorOnBlankLine(sql, pos)) return null;

  const statements = splitSqlStatementRanges(sql, databaseType, parameterOptions);
  for (let index = 0; index < statements.length; index += 1) {
    const statement = statements[index];
    const softRanges = splitStatementRangeAtSoftStarts(sql, statement, databaseType, parameterOptions);
    // Cursor inside the statement body, including the exact start/end.
    if (pos >= statement.from && pos <= statement.to) {
      return rangeForCursorInSoftRanges(sql, softRanges, pos) ?? rangeFor(statement, sql);
    }
    const next = statements[index + 1];
    // A caret after a statement's semicolon still belongs to that statement
    // until the next statement's text begins.
    if (pos > statement.to && (!next || pos < next.from) && isCursorInTrailingDelimiterGap(sql, statement.to, pos)) {
      return rangeForCursorInSoftRanges(sql, softRanges, pos) ?? rangeFor(softRanges[softRanges.length - 1] ?? statement, sql);
    }

    // Cursor in indentation or inter-statement whitespace immediately before
    // the statement should still target that statement, while the returned
    // execution range remains tight around the SQL text itself.
    if (pos >= statement.hitFrom && pos < statement.from && sql.slice(pos, statement.from).trim() === "") {
      const previous = statements[index - 1];
      if (previous && isCursorInTrailingDelimiterGap(sql, previous.to, pos)) {
        const previousSoftRanges = splitStatementRangeAtSoftStarts(sql, previous, databaseType, parameterOptions);
        return rangeForCursorInSoftRanges(sql, previousSoftRanges, pos) ?? rangeFor(previousSoftRanges[previousSoftRanges.length - 1] ?? previous, sql);
      }
      return rangeForCursorInSoftRanges(sql, softRanges, pos) ?? rangeFor(statement, sql);
    }

    if (pos > statement.to && (!next || pos < next.hitFrom)) {
      const softRange = rangeForCursorInSoftRanges(sql, softRanges, pos);
      if (softRange) return softRange;
      if (isCursorOnRangeEndLine(sql, pos, statement)) return rangeFor(statement, sql);
    }
  }

  // A MySQL `delimiter X` line is a client directive, not executable SQL, so it
  // never forms a statement range of its own. When the caret rests on such a
  // line, target the statement the directive introduces (or the closest
  // preceding statement when the directive ends the script) instead of
  // reporting that there is nothing to run. See issue #9485.
  const directiveRange = mysqlDelimiterDirectiveCursorRange(sql, pos, databaseType, parameterOptions, statements);
  if (directiveRange) return directiveRange;

  return null;
}

function isCursorInTrailingDelimiterGap(sql: string, previousStatementEnd: number, cursorPos: number): boolean {
  return cursorBelongsToTrailingStatementDelimiter(sql, previousStatementEnd, cursorPos);
}

function rangeForCursorInSoftRanges(sql: string, ranges: RawStatement[], pos: number): SqlTextRange | null {
  for (let index = 0; index < ranges.length; index += 1) {
    const range = ranges[index];
    if (pos >= range.from && pos <= range.to) {
      return rangeFor(range, sql);
    }
    if (pos >= range.hitFrom && pos < range.from && sql.slice(pos, range.from).trim() === "") {
      return rangeFor(range, sql);
    }

    const next = ranges[index + 1];
    if (pos > range.to && (!next || pos < next.hitFrom) && isCursorOnRangeEndLine(sql, pos, range)) {
      return rangeFor(range, sql);
    }
  }

  return null;
}

function splitStatementRangeAtSoftStarts(sql: string, statement: RawStatement, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): RawStatement[] {
  {}
  {}
  // Routine bodies contain top-level-looking SET/INSERT/SELECT lines that are not independent statements.
  if (isMysqlRoutineBlockDatabase(databaseType) && startsWithMysqlRoutineBlock(statement.sql, parameterOptions)) return [statement];
  // SQL Server control-flow batches use line-oriented BEGIN/EXEC tokens inside one IF/ELSE statement.
  {}

  const lineStarts = topLevelSoftStatementLineStarts(sql, statement, databaseType, parameterOptions);
  if (lineStarts.length <= 1) return [statement];

  const boundaries: Array<{ hitFrom: number; from: number; keyword: string }> = [];
  let currentKeyword = softStatementKeywordAt(sql, statement.from, databaseType, parameterOptions);
  let currentExplainTargetKeyword = explainLikeTargetKeywordAt(sql, statement.from, databaseType, parameterOptions);
  let currentBodyKeyword = currentExplainTargetKeyword ?? currentKeyword;
  let consumedWithMainStatement = false;
  let consumedExplainStatement = false;

  boundaries.push({ hitFrom: statement.hitFrom, from: statement.from, keyword: currentKeyword ?? "" });

  for (const lineStart of lineStarts) {
    if (lineStart.from <= statement.from) continue;

    if (currentBodyKeyword === "WITH" && !consumedWithMainStatement && WITH_MAIN_STATEMENT_KEYWORDS.has(lineStart.keyword)) {
      consumedWithMainStatement = true;
      // The CTE main statement also satisfies a pending EXPLAIN target, and its
      // own body rules (e.g. UPDATE ... SET) must take over from here.
      consumedExplainStatement = true;
      currentBodyKeyword = lineStart.keyword;
      continue;
    }

    if (isSetOperationQueryContinuation(sql, statement.from, lineStart.from, lineStart.keyword, databaseType, parameterOptions)) {
      continue;
    }

    if (!consumedExplainStatement && EXPLAIN_STATEMENT_KEYWORDS.has(lineStart.keyword) && (currentKeyword === "EXPLAIN" || currentExplainTargetKeyword !== null)) {
      consumedExplainStatement = true;
      currentBodyKeyword = lineStart.keyword;
      continue;
    }

    if (currentBodyKeyword === "CREATE" && CREATE_BODY_KEYWORDS.has(lineStart.keyword)) {
      continue;
    }

    {
    }

    if (currentBodyKeyword === "CREATE" && isMysqlCreateTableOptionContinuation(sql, statement.from, lineStart.from, lineStart.keyword, databaseType)) {
      continue;
    }

    if (currentBodyKeyword === "INSERT" && INSERT_BODY_KEYWORDS.has(lineStart.keyword)) {
      // Hand over to the source query's own rules so only its continuations
      // (CTE main statement, set operations) stay attached to the INSERT.
      currentBodyKeyword = lineStart.keyword;
      if (lineStart.keyword === "WITH") consumedWithMainStatement = false;
      continue;
    }

    if (currentBodyKeyword === "UPDATE" && lineStart.keyword === "SET") {
      continue;
    }

    if (currentBodyKeyword === "MERGE" && isMergeActionContinuation(sql, statement.from, lineStart.from, lineStart.keyword, databaseType, parameterOptions)) {
      continue;
    }

    {
    }

    if (currentBodyKeyword === "ALTER" && isMysqlAlterTableTruncatePartitionContinuation(sql, boundaries[boundaries.length - 1].from, lineStart.from, lineStart.keyword, databaseType)) {
      continue;
    }

    if (currentBodyKeyword === "ALTER" && ALTER_BODY_KEYWORDS.has(lineStart.keyword)) {
      continue;
    }

    boundaries.push(lineStart);
    currentKeyword = lineStart.keyword;
    currentExplainTargetKeyword = explainLikeTargetKeywordAt(sql, lineStart.from, databaseType, parameterOptions);
    currentBodyKeyword = currentExplainTargetKeyword ?? currentKeyword;
    consumedWithMainStatement = false;
    consumedExplainStatement = false;
  }

  if (boundaries.length <= 1) return [statement];

  const ranges: RawStatement[] = [];
  for (let index = 0; index < boundaries.length; index += 1) {
    const boundary = boundaries[index];
    const next = boundaries[index + 1];
    const to = next ? trimRangeEndBeforeNextBoundary(sql, boundary.from, next.from, databaseType, parameterOptions) : trimRangeEnd(sql, boundary.from, statement.to);
    if (to > boundary.from) {
      ranges.push({
        hitFrom: boundary.hitFrom,
        from: boundary.from,
        to,
        sql: sql.slice(boundary.from, to),
      });
    }
  }

  return ranges.length > 0 ? ranges : [statement];
}

// `BEGIN` opens a control-flow block only when it does not start a transaction or a
// conversation (`BEGIN TRAN`, `BEGIN DISTRIBUTED TRANSACTION`, `BEGIN DIALOG
// CONVERSATION`); those have no matching `END`.

// `END CONVERSATION`/`END DIALOG` close a conversation instead of a BEGIN/CASE block.

/**
 * Collect the control-flow facts of a T-SQL fragment: how many BEGIN/CASE blocks
 * it leaves open and whether it contains an `IF`-level `ELSE`. Tokens inside
 * parentheses, string literals and comments are ignored, and `CASE ... END`
 * nesting keeps a `CASE`'s own `ELSE` from looking like an `IF` branch.
 *
 * `initialOpenBlocks` is the depth the previous fragment of the same statement
 * left open. Scanning with the carried depth (instead of every fragment
 * restarting from zero) is what lets a fragment that begins by closing the
 * previous block — `END ELSE BEGIN SELECT 2`, or `END` followed by the next
 * statement — report where the block actually ends.
 */

/**
 * True when `statement` is a whole T-SQL control-flow batch: an `IF`/`WHILE`
 * whose body is a `BEGIN ... END` block or whose branches are plain statements
 * with an `ELSE`. Such a batch contains no independent statements, so it must
 * stay one execution range instead of being split at the line-oriented
 * `BEGIN`/`EXEC`/`END` tokens.
 */

/**
 * T-SQL does not end a `BEGIN ... END` block at a semicolon, so splitting on
 * every top-level `;` cuts `IF ... BEGIN ... ; ... END` batches into fragments
 * whose `BEGIN`/`EXEC`/`END` lines then look like independent statements
 * (#9336). Re-join the fragments of such a batch so it stays one range.
 */

/**
 * First offset in `sql[from, to)` that is neither whitespace nor a comment,
 * clamped to `to`. The `;`-split path keeps the comment between two statements
 * out of both of them; the control-flow remainder re-queue must do the same so
 * a comment after a closing `END` is not glued onto the next statement's range.
 */

function topLevelSoftStatementLineStarts(sql: string, statement: RawStatement, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): Array<{ hitFrom: number; from: number; keyword: string }> {
  const starts: Array<{ hitFrom: number; from: number; keyword: string }> = [];
  const len = statement.to;
  const explainOptionsStart = explainOptionsParenAt(sql, statement.from);
  // Recover soft statement boundaries while the user is still typing an
  // EXPLAIN option list; otherwise its unmatched opener hides every later line.
  const unclosedExplainOptionsStart = explainOptionsStart !== null && skipBalancedParens(sql, explainOptionsStart, databaseType, parameterOptions) === null ? explainOptionsStart : null;
  const backslashEscapes = allowsBackslashStringEscape(databaseType);
  let state: QuoteState | "lineComment" | "blockComment" = "none";
  let dollarTag = "";
  let parenDepth = 0;
  let lineStart = statement.from;
  let firstNonWhitespaceOnLine = -1;
  let i = statement.from;

  while (i < len) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "none" && firstNonWhitespaceOnLine === -1 && ch !== "\n" && ch !== "\r" && !isSqlWhitespace(ch) && !startsLineComment(sql, i, databaseType, parameterOptions) && !startsBlockComment(sql, i)) {
      firstNonWhitespaceOnLine = i;
      if (parenDepth === 0) {
        const keyword = softStatementKeywordAt(sql, i, databaseType, parameterOptions);
        if (keyword) {
          starts.push({ hitFrom: lineStart, from: i, keyword });
        }
      }
    }

    if (ch === "\n") {
      if (state === "lineComment") state = "none";
      lineStart = i + 1;
      firstNonWhitespaceOnLine = -1;
      i += 1;
      continue;
    }

    if (state === "lineComment") {
      i += 1;
      continue;
    }

    if (state === "blockComment") {
      if (ch === "*" && next === "/") {
        state = "none";
        i += 2;
        continue;
      }
      i += 1;
      continue;
    }

    if (state === "dollar") {
      if (ch === "$") {
        const closingTag = `$${dollarTag}$`;
        if (sql.startsWith(closingTag, i)) {
          i += closingTag.length;
          state = "none";
          dollarTag = "";
          continue;
        }
      }
      i += 1;
      continue;
    }

    if (state === "single") {
      if (ch === "\\" && next && backslashEscapes) {
        i += 2;
        continue;
      }
      if (ch === "'") {
        if (next === "'") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "double") {
      if (ch === '"') {
        if (next === '"') {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "backtick") {
      if (ch === "`") {
        if (next === "`") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "bracket") {
      if (ch === "]") state = "none";
      i += 1;
      continue;
    }

    // state === "none"
    if (ch === "-" && next === "-") {
      state = "lineComment";
      i += 2;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      state = "lineComment";
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      state = "blockComment";
      i += 2;
      continue;
    }
    if (ch === "'") {
      state = "single";
      i += 1;
      continue;
    }
    if (ch === '"') {
      state = "double";
      i += 1;
      continue;
    }
    if (ch === "`") {
      state = "backtick";
      i += 1;
      continue;
    }
    if (ch === "[") {
      state = "bracket";
      i += 1;
      continue;
    }
    if (ch === "$") {
      const tagMatch = /^\$[A-Za-z_0-9]*\$/.exec(sql.slice(i));
      if (tagMatch) {
        dollarTag = tagMatch[0].slice(1, -1);
        i += tagMatch[0].length;
        state = "dollar";
        continue;
      }
    }
    if (ch === "(" && i !== unclosedExplainOptionsStart) {
      parenDepth += 1;
    } else if (ch === ")" && parenDepth > 0) {
      parenDepth -= 1;
    }
    i += 1;
  }

  return starts;
}

function softStatementKeywordAt(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): string | null {
  const match = /^[A-Za-z_][\w$]*/.exec(sql.slice(pos));
  if (!match) return null;
  const keyword = match[0].toUpperCase();
  if (SOFT_STATEMENT_FUNCTION_KEYWORDS.has(keyword) && nextNonWhitespaceChar(sql, pos + match[0].length) === "(") return null;
  // COMMENT is also a common column name. Only COMMENT ON starts a standalone
  // SQL command; otherwise a line-start projection column must stay in SELECT.
  if (keyword === "COMMENT" && nextSqlWord(sql, pos + match[0].length, databaseType, parameterOptions) !== "ON") return null;
  // `WITH (` is a SQL Server table hint (`FROM t WITH (NOLOCK)`), not a CTE
  // opener — CTEs are always `WITH name AS (` / `WITH RECURSIVE name AS (`.
  // Formatters break table hints onto their own line; without this, the
  // following statement loses its run target (#10098).
  if (keyword === "WITH" && nextNonWhitespaceChar(sql, pos + match[0].length) === "(") return null;
  return softStatementStartKeywords(databaseType).has(keyword) ? keyword : null;
}

function softStatementStartKeywords(databaseType?: DatabaseType): Set<string> {
  return new Set([...COMMON_SOFT_STATEMENT_START_KEYWORDS, ...(databaseType ? (DATABASE_SOFT_STATEMENT_KEYWORDS[databaseType] ?? []) : [])]);
}

function isSetOperationQueryContinuation(sql: string, from: number, to: number, keyword: string, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  if (keyword !== "SELECT" && keyword !== "WITH") return false;
  const words = topLevelWordsBefore(sql, from, to, 3, databaseType, parameterOptions);
  const last = words[words.length - 1];
  if (last && SET_OPERATION_KEYWORDS.has(last)) return true;
  if (last && SET_OPERATION_MODIFIER_KEYWORDS.has(last)) {
    const previous = words[words.length - 2];
    return !!previous && SET_OPERATION_KEYWORDS.has(previous);
  }
  return false;
}

function isMysqlCreateTableOptionContinuation(sql: string, statementFrom: number, lineStartFrom: number, keyword: string, databaseType?: DatabaseType): boolean {
  if (databaseType && !MYSQL_CREATE_TABLE_OPTION_DATABASES.has(databaseType)) return false;
  if (keyword !== "COMMENT") return false;
  if (!startsWithMysqlCreateTable(sql, statementFrom)) return false;

  const next = nextNonWhitespaceChar(sql, lineStartFrom + keyword.length);
  return next === "=" || next === "'" || next === '"';
}

function isMysqlAlterTableTruncatePartitionContinuation(sql: string, statementFrom: number, lineStartFrom: number, keyword: string, databaseType?: DatabaseType): boolean {
  if (databaseType !== "mysql" || keyword !== "TRUNCATE") return false;
  if (!startsWithSqlWords(sql, statementFrom, ["ALTER", "TABLE"], databaseType)) return false;
  return nextSqlWord(sql, lineStartFrom + keyword.length, databaseType) === "PARTITION";
}

function isMergeActionContinuation(sql: string, statementFrom: number, lineStartFrom: number, keyword: string, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  // Oracle (and friends) allow each MERGE action on its own line after
  // `WHEN ... MATCHED THEN`, e.g. `UPDATE SET ...` (#9516); only INSERT was
  // recognized, so UPDATE/DELETE action lines split the statement in two.
  if (keyword !== "INSERT" && keyword !== "UPDATE" && keyword !== "DELETE" && keyword !== "SET") return false;
  if (!startsWithSqlWords(sql, statementFrom, ["MERGE"], databaseType, parameterOptions)) return false;
  topLevelWordsBefore(sql, statementFrom, lineStartFrom, 5, databaseType, parameterOptions);
  if (keyword === "SET") return false;
  return false;
}

function startsWithMysqlCreateTable(sql: string, statementFrom: number): boolean {
  const text = sql.slice(statementFrom, statementFrom + 256);
  return /^CREATE\s+(?:TEMPORARY\s+)?TABLE\b/i.test(text);
}

function topLevelWordsBefore(sql: string, from: number, to: number, limit: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): string[] {
  return topLevelWordsInRange(sql, from, to, databaseType, parameterOptions, limit).words;
}

/** Top-level (paren-depth 0) keywords of `sql[from, to)`, in order, each paired
 *  with the offset just after it.
 *
 * `tailLimit` keeps only the last N words (callers that only look at the words
 * right before a position, e.g. `MERGE ... THEN`), so scanning a long fragment
 * no longer allocates the whole word list. */
function topLevelWordsInRange(sql: string, from: number, to: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions, tailLimit?: number): { words: string[]; ends: number[] } {
  const words: string[] = [];
  const ends: number[] = [];
  const backslashEscapes = allowsBackslashStringEscape(databaseType);
  let state: QuoteState | "lineComment" | "blockComment" = "none";
  let dollarTag = "";
  let parenDepth = 0;
  let i = from;

  while (i < to) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "lineComment") {
      if (ch === "\n") state = "none";
      i += 1;
      continue;
    }

    if (state === "blockComment") {
      if (ch === "*" && next === "/") {
        state = "none";
        i += 2;
        continue;
      }
      i += 1;
      continue;
    }

    if (state === "dollar") {
      if (ch === "$") {
        const closingTag = `$${dollarTag}$`;
        if (sql.startsWith(closingTag, i)) {
          i += closingTag.length;
          state = "none";
          dollarTag = "";
          continue;
        }
      }
      i += 1;
      continue;
    }

    if (state === "single") {
      if (ch === "\\" && next && backslashEscapes) {
        i += 2;
        continue;
      }
      if (ch === "'") {
        if (next === "'") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "double") {
      if (ch === '"') {
        if (next === '"') {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "backtick") {
      if (ch === "`") {
        if (next === "`") {
          i += 2;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "bracket") {
      if (ch === "]") state = "none";
      i += 1;
      continue;
    }

    if (ch === "-" && next === "-") {
      state = "lineComment";
      i += 2;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      state = "lineComment";
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      state = "blockComment";
      i += 2;
      continue;
    }
    if (ch === "'") {
      state = "single";
      i += 1;
      continue;
    }
    if (ch === '"') {
      state = "double";
      i += 1;
      continue;
    }
    if (ch === "`") {
      state = "backtick";
      i += 1;
      continue;
    }
    if (ch === "[") {
      state = "bracket";
      i += 1;
      continue;
    }
    if (ch === "$") {
      const tagMatch = /^\$[A-Za-z_0-9]*\$/.exec(sql.slice(i));
      if (tagMatch) {
        dollarTag = tagMatch[0].slice(1, -1);
        i += tagMatch[0].length;
        state = "dollar";
        continue;
      }
    }
    if (ch === "(") {
      parenDepth += 1;
      i += 1;
      continue;
    }
    if (ch === ")") {
      if (parenDepth > 0) parenDepth -= 1;
      i += 1;
      continue;
    }
    if (parenDepth === 0) {
      const match = /^[A-Za-z_][\w$]*/.exec(sql.slice(i));
      if (match) {
        if (tailLimit !== undefined && words.length >= tailLimit) {
          words.shift();
          ends.shift();
        }
        words.push(match[0].toUpperCase());
        ends.push(i + match[0].length);
        i += match[0].length;
        continue;
      }
    }
    i += 1;
  }

  return { words, ends };
}

function nextNonWhitespaceChar(sql: string, pos: number): string | null {
  let i = pos;
  while (i < sql.length && isSqlWhitespace(sql[i])) i += 1;
  return i < sql.length ? sql[i] : null;
}

function nextSqlWord(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): string | null {
  return nextSqlWordToken(sql, pos, databaseType, parameterOptions)?.word ?? null;
}

function startsWithSqlWords(sql: string, pos: number, expectedWords: readonly string[], databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  let i = pos;
  for (const expectedWord of expectedWords) {
    const token = nextSqlWordToken(sql, i, databaseType, parameterOptions);
    if (token?.word !== expectedWord) return false;
    i = token.end;
  }
  return true;
}

function nextSqlWordToken(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): { word: string; end: number } | null {
  let i = pos;
  // SQL comments may legally appear anywhere whitespace can separate keywords.
  while (i < sql.length) {
    if (isSqlWhitespace(sql[i])) {
      i += 1;
      continue;
    }
    if (sql[i] === "-" && sql[i + 1] === "-") {
      i += 2;
      while (i < sql.length && sql[i] !== "\n" && sql[i] !== "\r") i += 1;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      i += 1;
      while (i < sql.length && sql[i] !== "\n" && sql[i] !== "\r") i += 1;
      continue;
    }
    if (sql[i] === "/" && sql[i + 1] === "*") {
      const commentEnd = sql.indexOf("*/", i + 2);
      if (commentEnd < 0) return null;
      i = commentEnd + 2;
      continue;
    }
    break;
  }

  const match = /^[A-Za-z_][\w$]*/.exec(sql.slice(i));
  if (!match) return null;
  return { word: match[0].toUpperCase(), end: i + match[0].length };
}

function isExplainLikeKeyword(keyword: string | null): boolean {
  return keyword === "EXPLAIN" || keyword === "DESCRIBE" || keyword === "DESC";
}

function explainOptionsParenAt(sql: string, pos: number): number | null {
  const prefixMatch = /^[A-Za-z_][\w$]*/.exec(sql.slice(pos));
  if (prefixMatch?.[0]?.toUpperCase() !== "EXPLAIN") return null;

  let i = pos + prefixMatch[0].length;
  while (i < sql.length && isSqlWhitespace(sql[i])) i += 1;
  return sql[i] === "(" ? i : null;
}

function explainLikeTargetKeywordAt(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): string | null {
  const prefixMatch = /^[A-Za-z_][\w$]*/.exec(sql.slice(pos));
  const prefix = prefixMatch?.[0]?.toUpperCase();
  if (!isExplainLikeKeyword(prefix ?? null)) return null;

  let i = pos + (prefixMatch?.[0].length ?? 0);
  while (i < sql.length && isSqlWhitespace(sql[i])) i += 1;
  // Parenthesized EXPLAIN options (e.g. Postgres `EXPLAIN (ANALYZE, BUFFERS) ...`)
  // sit between the keyword and its target statement. DESC/DESCRIBE take no
  // options — a paren there is a subquery (ClickHouse `DESCRIBE (SELECT ...)`).
  if (prefix === "EXPLAIN" && sql[i] === "(") {
    const optionsEnd = skipBalancedParens(sql, i, databaseType, parameterOptions);
    if (optionsEnd === null) return null;
    i = optionsEnd;
    while (i < sql.length && isSqlWhitespace(sql[i])) i += 1;
  }
  const targetMatch = /^[A-Za-z_][\w$]*/.exec(sql.slice(i));
  const targetKeyword = targetMatch?.[0]?.toUpperCase();
  return targetKeyword && EXPLAIN_STATEMENT_KEYWORDS.has(targetKeyword) ? targetKeyword : null;
}

function skipBalancedParens(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): number | null {
  let state: "none" | "single" | "double" | "lineComment" | "blockComment" = "none";
  let depth = 0;
  let i = pos;

  while (i < sql.length) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "lineComment") {
      if (ch === "\n") state = "none";
      i += 1;
      continue;
    }
    if (state === "blockComment") {
      if (ch === "*" && next === "/") {
        state = "none";
        i += 2;
        continue;
      }
      i += 1;
      continue;
    }
    if (state === "single") {
      if (ch === "'" && next === "'") {
        i += 2;
        continue;
      }
      if (ch === "'") state = "none";
      i += 1;
      continue;
    }
    if (state === "double") {
      if (ch === '"' && next === '"') {
        i += 2;
        continue;
      }
      if (ch === '"') state = "none";
      i += 1;
      continue;
    }

    if (ch === "-" && next === "-") {
      state = "lineComment";
      i += 2;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      state = "lineComment";
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      state = "blockComment";
      i += 2;
      continue;
    }
    if (ch === "'") {
      state = "single";
      i += 1;
      continue;
    }
    if (ch === '"') {
      state = "double";
      i += 1;
      continue;
    }
    if (ch === "(") depth += 1;
    if (ch === ")") {
      depth -= 1;
      if (depth === 0) return i + 1;
    }
    i += 1;
  }

  return null;
}

function startsLineComment(sql: string, pos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  return (sql[pos] === "-" && sql[pos + 1] === "-") || startsHashLineComment(sql, pos, databaseType, parameterOptions);
}

function startsHashLineComment(sql: string, pos: number, _databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): boolean {
  // `#` is a MySQL-family line-comment marker. Oracle-family engines also allow
  // it in unquoted identifiers (for example `V$DATAFILE.FILE#`), so treating it
  // as a comment there truncates otherwise valid statements.
  if (sql[pos] !== "#") return false;
  return readSqlBracedParameterAt(sql, pos, parameterOptions)?.syntax !== "mybatis";
}

function startsBlockComment(sql: string, pos: number): boolean {
  return sql[pos] === "/" && sql[pos + 1] === "*";
}

function trimRangeEnd(sql: string, from: number, to: number): number {
  let end = to;
  while (end > from && isSqlWhitespace(sql[end - 1])) {
    end -= 1;
  }
  return end;
}

function trimRangeEndBeforeNextBoundary(sql: string, from: number, nextBoundaryFrom: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): number {
  const backslashEscapes = allowsBackslashStringEscape(databaseType);
  let state: QuoteState | "lineComment" | "blockComment" = "none";
  let dollarTag = "";
  let lastContentEnd = from;
  let i = from;

  while (i < nextBoundaryFrom) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "lineComment") {
      if (ch === "\n") state = "none";
      i += 1;
      continue;
    }

    if (state === "blockComment") {
      if (ch === "*" && next === "/") {
        state = "none";
        i += 2;
        continue;
      }
      i += 1;
      continue;
    }

    if (state === "dollar") {
      lastContentEnd = i + 1;
      if (ch === "$") {
        const closingTag = `$${dollarTag}$`;
        if (sql.startsWith(closingTag, i)) {
          i += closingTag.length;
          lastContentEnd = i;
          state = "none";
          dollarTag = "";
          continue;
        }
      }
      i += 1;
      continue;
    }

    if (state === "single") {
      lastContentEnd = i + 1;
      if (ch === "\\" && next && backslashEscapes) {
        i += 2;
        lastContentEnd = i;
        continue;
      }
      if (ch === "'") {
        if (next === "'") {
          i += 2;
          lastContentEnd = i;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "double") {
      lastContentEnd = i + 1;
      if (ch === '"') {
        if (next === '"') {
          i += 2;
          lastContentEnd = i;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "backtick") {
      lastContentEnd = i + 1;
      if (ch === "`") {
        if (next === "`") {
          i += 2;
          lastContentEnd = i;
          continue;
        }
        state = "none";
      }
      i += 1;
      continue;
    }

    if (state === "bracket") {
      lastContentEnd = i + 1;
      if (ch === "]") state = "none";
      i += 1;
      continue;
    }

    if (ch === "-" && next === "-") {
      state = "lineComment";
      i += 2;
      continue;
    }
    if (startsHashLineComment(sql, i, databaseType, parameterOptions)) {
      state = "lineComment";
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      state = "blockComment";
      i += 2;
      continue;
    }
    if (ch === "'") {
      state = "single";
      lastContentEnd = i + 1;
      i += 1;
      continue;
    }
    if (ch === '"') {
      state = "double";
      lastContentEnd = i + 1;
      i += 1;
      continue;
    }
    if (ch === "`") {
      state = "backtick";
      lastContentEnd = i + 1;
      i += 1;
      continue;
    }
    if (ch === "[") {
      state = "bracket";
      lastContentEnd = i + 1;
      i += 1;
      continue;
    }
    if (ch === "$") {
      const tagMatch = /^\$[A-Za-z_0-9]*\$/.exec(sql.slice(i));
      if (tagMatch) {
        state = "dollar";
        dollarTag = tagMatch[0].slice(1, -1);
        i += tagMatch[0].length;
        lastContentEnd = i;
        continue;
      }
    }

    if (!isSqlWhitespace(ch)) {
      lastContentEnd = i + 1;
    }
    i += 1;
  }

  return trimRangeEnd(sql, from, lastContentEnd);
}

function isSqlWhitespace(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\r" || ch === "\n";
}

export function sqlStatementParameterOptionsForCompatibility(_databaseType?: DatabaseType, _compatibilityMode?: string): SqlParameterOptions | undefined {
  {
    return undefined;
  }
}

export function isOracleLikeDatabase(_databaseType?: DatabaseType, _options?: SqlParameterOptions): boolean {
  return false;
}

/**
 * Whether a statement must stay a single statement instead of being cut at its inner semicolons:
 * Oracle PL/SQL blocks on Oracle-like dialects, and the Oracle-style routine bodies that a
 * PostgreSQL-family connection reaches on a GaussDB/openGauss Oracle-compatibility server.
 */
export function keepsOracleStyleBlockTogether(_sql: string, _databaseType?: DatabaseType, _options?: SqlParameterOptions): boolean {
  {}
  return false;
}

export function isOraclePlSqlStatement(_sql: string, _databaseType?: DatabaseType, _options?: SqlParameterOptions): boolean {
  return false;
}

function isMysqlRoutineBlockDatabase(databaseType?: DatabaseType): boolean {
  return !!databaseType && MYSQL_ROUTINE_BLOCK_DATABASES.has(databaseType);
}

function startsWithMysqlRoutineBlock(sql: string, parameterOptions?: SqlParameterOptions): boolean {
  return isMysqlRoutineDdlStart(sql, parameterOptions) && mysqlRoutineTokensContainBegin(mysqlRoutineTokens(sql, parameterOptions).tokens);
}

function mysqlRoutineTokensContainBegin(tokens: readonly MysqlRoutineToken[]): boolean {
  return tokens.some((token) => token.kind === "word" && token.value === "BEGIN");
}

function isMysqlRoutineDdlStart(sql: string, parameterOptions?: SqlParameterOptions): boolean {
  return isMysqlRoutineDdlStartFromWords(mysqlRoutineWords(sql, parameterOptions).slice(0, 16));
}

/** First 16 word tokens, scanned without ever visiting the tail of `tokens` -
 * the routine-detection check only ever needs this small fixed-size prefix. */
function mysqlRoutineDdlStartWords(tokens: readonly MysqlRoutineToken[], limit = 16): string[] {
  const words: string[] = [];
  for (const token of tokens) {
    if (token.kind !== "word") continue;
    words.push(token.value);
    if (words.length >= limit) break;
  }
  return words;
}

function isMysqlRoutineDdlStartFromWords(words: readonly string[]): boolean {
  if (words[0] !== "CREATE") return false;

  for (const word of words.slice(1)) {
    if (MYSQL_ROUTINE_OBJECT_TYPES.has(word)) return true;
    if (MYSQL_NON_ROUTINE_CREATE_TYPES.has(word)) return false;
  }
  return false;
}

/**
 * Incremental, resumable equivalent of walking `mysqlRoutineTokens(...)` with a
 * fresh `blockStack`/`sawBegin` on every call. Driven token-by-token from
 * `mysqlRoutineBlockIsCompleteUpTo` in splitSqlStatementRanges, so a routine body
 * with N internal semicolons costs O(N) total instead of O(N^2) (re-walking the
 * whole BEGIN/CASE/END nesting from the top on every semicolon while typing).
 *
 * Semantics mirror the non-incremental token walk exactly: `lastWordToken` plays
 * the role of `previousWordToken(tokens, index)` (both reset to null once a
 * semicolon is crossed), and `pendingEndAwaitingSuffix` defers resolving an "END"
 * token's effect on `blockStack` until the following token arrives, which plays
 * the role of `nextWordToken(tokens, index)`'s lookahead without needing to
 * revisit already-processed tokens.
 */
interface MysqlRoutineBlockCompleteness {
  processedTokenCount: number;
  blockStack: Array<"BEGIN" | "CASE">;
  sawBegin: boolean;
  lastWordToken: string | null;
  pendingEndAwaitingSuffix: boolean;
  lastTokenKind: "word" | "semicolon" | null;
}

function newMysqlRoutineBlockCompleteness(): MysqlRoutineBlockCompleteness {
  return { processedTokenCount: 0, blockStack: [], sawBegin: false, lastWordToken: null, pendingEndAwaitingSuffix: false, lastTokenKind: null };
}

/** Folds any `tokens` not yet seen into `state` and returns whether the routine body is complete so far. */
function advanceMysqlRoutineBlockCompleteness(state: MysqlRoutineBlockCompleteness, tokens: readonly MysqlRoutineToken[]): boolean {
  while (state.processedTokenCount < tokens.length) {
    const token = tokens[state.processedTokenCount];
    state.processedTokenCount += 1;

    if (token.kind === "semicolon") {
      if (state.pendingEndAwaitingSuffix) {
        state.blockStack.pop();
        state.pendingEndAwaitingSuffix = false;
      }
      state.lastWordToken = null;
      state.lastTokenKind = "semicolon";
      continue;
    }

    const value = token.value;
    if (state.pendingEndAwaitingSuffix) {
      if (value === "CASE") {
        if (state.blockStack[state.blockStack.length - 1] === "CASE") state.blockStack.pop();
      } else if (!MYSQL_CONTROL_BLOCK_SUFFIXES.has(value)) {
        state.blockStack.pop();
      }
      state.pendingEndAwaitingSuffix = false;
    }

    if (value === "BEGIN") {
      if (state.lastWordToken !== "END") {
        state.sawBegin = true;
        state.blockStack.push("BEGIN");
      }
    } else if (value === "CASE") {
      if (state.lastWordToken !== "END") {
        state.blockStack.push("CASE");
      }
    } else if (value === "END" && state.sawBegin) {
      state.pendingEndAwaitingSuffix = true;
    }
    state.lastWordToken = value;
    state.lastTokenKind = "word";
  }

  return state.sawBegin && state.blockStack.length === 0 && state.lastTokenKind === "semicolon";
}

function mysqlRoutineWords(sql: string, parameterOptions?: SqlParameterOptions): string[] {
  return mysqlRoutineTokens(sql, parameterOptions)
    .tokens.filter((token): token is { kind: "word"; value: string } => token.kind === "word")
    .map((token) => token.value);
}

type MysqlRoutineToken = { kind: "word" | "semicolon"; value: string };
type MysqlRoutineLexState = QuoteState | "lineComment" | "blockComment";

const MYSQL_ROUTINE_WORD_RE = /[A-Za-z_][\w$]*/y;

/**
 * Tokenizes `sql` starting at `fromIndex` (resuming from `initialState`, the lexer
 * state left over from any earlier chunk) and stops at `toIndex`. Callers that
 * re-check an ever-growing prefix on every semicolon (see splitSqlStatementRanges)
 * rely on this being resumable so they only re-scan the newly typed suffix instead
 * of the whole prefix from scratch every time.
 */
function mysqlRoutineTokens(sql: string, parameterOptions?: SqlParameterOptions, fromIndex = 0, toIndex = sql.length, initialState: MysqlRoutineLexState = "none"): { tokens: MysqlRoutineToken[]; endState: MysqlRoutineLexState } {
  const tokens: MysqlRoutineToken[] = [];
  let state: MysqlRoutineLexState = initialState;
  let i = fromIndex;

  while (i < toIndex) {
    const ch = sql[i];
    const next = sql[i + 1] ?? "";

    if (state === "lineComment") {
      if (ch === "\n") state = "none";
      i += 1;
      continue;
    }
    if (state === "blockComment") {
      if (ch === "*" && next === "/") {
        state = "none";
        i += 2;
        continue;
      }
      i += 1;
      continue;
    }
    if (state === "single") {
      // mysqlRoutineTokens runs only for MYSQL_ROUTINE_BLOCK_DATABASES (all MySQL-family), so
      // backslash escaping here (and in the double branch below) is unconditionally correct.
      if (ch === "\\" && next) {
        i += 2;
        continue;
      }
      if (ch === "'" && next === "'") {
        i += 2;
        continue;
      }
      if (ch === "'") state = "none";
      i += 1;
      continue;
    }
    if (state === "double") {
      if (ch === "\\" && next) {
        i += 2;
        continue;
      }
      if (ch === '"' && next === '"') {
        i += 2;
        continue;
      }
      if (ch === '"') state = "none";
      i += 1;
      continue;
    }
    if (state === "backtick") {
      if (ch === "`" && next === "`") {
        i += 2;
        continue;
      }
      if (ch === "`") state = "none";
      i += 1;
      continue;
    }

    if (ch === "-" && next === "-") {
      state = "lineComment";
      i += 2;
      continue;
    }
    if (startsHashLineComment(sql, i, undefined, parameterOptions)) {
      state = "lineComment";
      i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      state = "blockComment";
      i += 2;
      continue;
    }
    if (ch === "'") {
      state = "single";
      i += 1;
      continue;
    }
    if (ch === '"') {
      state = "double";
      i += 1;
      continue;
    }
    if (ch === "`") {
      state = "backtick";
      i += 1;
      continue;
    }
    if (ch === ";") {
      tokens.push({ kind: "semicolon", value: ";" });
      i += 1;
      continue;
    }

    MYSQL_ROUTINE_WORD_RE.lastIndex = i;
    const word = MYSQL_ROUTINE_WORD_RE.exec(sql)?.[0];
    if (word) {
      tokens.push({ kind: "word", value: word.toUpperCase() });
      i += word.length;
      continue;
    }
    i += 1;
  }

  return { tokens, endState: state };
}

/**
 * Whether the statement is a routine whose body is written in Oracle PL/SQL syntax
 * (`CREATE [OR REPLACE] PROCEDURE|FUNCTION ... { AS | IS } { DECLARE | BEGIN }`, plus the
 * Oracle-only `PACKAGE BODY` / `TYPE BODY` forms).
 *
 * This is deliberately narrower than `startsWithOraclePlSqlBlock`: a PostgreSQL connection only
 * sees this shape when the server is a GaussDB/openGauss instance in Oracle (A) compatibility
 * mode, because PostgreSQL itself requires a dollar-quoted or string body (`AS $$ ... $$`,
 * `AS 'body'`) and has no `PACKAGE`/`TYPE BODY`.
 */

/**
 * The body introducer of an Oracle-style routine has to be the DECLARE/BEGIN keyword itself.
 * Declarations may follow `AS`/`IS` with or without the optional `DECLARE`, while a PostgreSQL
 * body opens with `$$`, `$tag$` or a quoted string there, so only a bare identifier matches.
 */

/** Skip OR REPLACE / FORCE / NOFORCE / EDITIONABLE modifiers after CREATE. */

/**
 * Classify CREATE programmable objects:
 * - body: PACKAGE BODY / TYPE BODY (outer END beyond nested routines)
 * - spec: PACKAGE specification only (declarations + END, no BEGIN)
 * - null: ordinary SQL / other objects (including plain CREATE TYPE ... AS OBJECT)
 */

function isAtLineStart(sql: string, pos: number): boolean {
  for (let i = pos - 1; i >= 0; i -= 1) {
    const ch = sql[i];
    if (ch === "\n" || ch === "\r") return true;
    if (ch !== " " && ch !== "\t") return false;
  }
  return true;
}

function startsDelimiterCommand(sql: string, pos: number): boolean {
  const prefix = sql.slice(pos, pos + 9);
  return prefix.toLowerCase() === "delimiter" && (sql[pos + 9] === " " || sql[pos + 9] === "\t" || sql[pos + 9] === ";");
}

function parseDelimiterCommand(line: string): string | null {
  const trimmed = line.trim();
  if (/^delimiter;$/i.test(trimmed)) return ";";
  const match = /^delimiter[ \t]+(.+)$/i.exec(trimmed);
  const delimiter = match?.[1]?.trim();
  return delimiter ? delimiter : null;
}

/**
 * Resolves a caret resting on a MySQL `delimiter X` client-directive line to
 * the nearest executable statement: the statement the directive introduces, or
 * the statement just before it when the directive has nothing after it.
 */
function mysqlDelimiterDirectiveCursorRange(sql: string, pos: number, databaseType: DatabaseType | undefined, parameterOptions: SqlParameterOptions | undefined, statements: RawStatement[]): SqlTextRange | null {
  if (databaseType !== "mysql" || statements.length === 0) return null;
  const lineStart = sql.lastIndexOf("\n", pos - 1) + 1;
  const lineEnd = findLineEnd(sql, pos);
  let directiveStart = lineStart;
  while (directiveStart < lineEnd && (sql[directiveStart] === " " || sql[directiveStart] === "\t")) directiveStart += 1;
  if (!startsDelimiterCommand(sql, directiveStart)) return null;
  if (parseDelimiterCommand(sql.slice(directiveStart, lineEnd)) === null) return null;

  const following = statements.find((statement) => statement.from >= lineEnd);
  if (following) {
    return rangeFor(splitStatementRangeAtSoftStarts(sql, following, databaseType, parameterOptions)[0] ?? following, sql);
  }
  const preceding = statements[statements.length - 1];
  const precedingSoftRanges = splitStatementRangeAtSoftStarts(sql, preceding, databaseType, parameterOptions);
  return rangeFor(precedingSoftRanges[precedingSoftRanges.length - 1] ?? preceding, sql);
}

function findLineEnd(sql: string, pos: number): number {
  for (let cursor = pos; cursor < sql.length; cursor += 1) {
    if (sql[cursor] === "\n" || sql[cursor] === "\r") return cursor;
  }
  return sql.length;
}

function nextLineStart(sql: string, lineEnd: number): number {
  if (sql[lineEnd] === "\r" && sql[lineEnd + 1] === "\n") return lineEnd + 2;
  if (sql[lineEnd] === "\n" || sql[lineEnd] === "\r") return lineEnd + 1;
  return lineEnd;
}

function rangeFor(statement: RawStatement, sql: string): SqlTextRange {
  return {
    from: statement.from,
    to: statement.to,
    sql: sql.slice(statement.from, statement.to),
  };
}

function clampCursor(sql: string, cursorPos: number): number {
  if (!Number.isFinite(cursorPos)) return 0;
  if (cursorPos < 0) return 0;
  if (cursorPos > sql.length) return sql.length;
  return cursorPos;
}

function isCursorOnBlankLine(sql: string, pos: number): boolean {
  const lineStart = sql.lastIndexOf("\n", pos - 1) + 1;
  let lineEnd = sql.indexOf("\n", pos);
  if (lineEnd === -1) lineEnd = sql.length;
  return sql.slice(lineStart, lineEnd).trim() === "";
}

function isCursorOnRangeEndLine(sql: string, pos: number, range: Pick<RawStatement, "to">): boolean {
  const lineStart = sql.lastIndexOf("\n", pos - 1) + 1;
  let lineEnd = sql.indexOf("\n", pos);
  if (lineEnd === -1) lineEnd = sql.length;
  return range.to >= lineStart && range.to <= lineEnd;
}

/**
 * Returns the full document as a range, or `null` when it is empty/whitespace.
 */
export function fullSqlRange(sql: string): SqlTextRange | null {
  const trimmed = sql.trim();
  if (!trimmed) return null;
  const from = sql.length - sql.trimStart().length;
  const to = from + trimmed.length;
  return { from, to, sql: sql.slice(from, to) };
}

function normalizeSql(sql: string): string {
  return sql.replace(/\s+/g, " ").replace(/;\s*$/, "").trim();
}

/**
 * Build the ordered list of execution candidates to show in the picker.
 *
 * Order is always `[cursor, all]` when both are available, except when the
 * cursor statement and the full document are effectively the same SQL — in
 * that case only a single candidate is returned to avoid duplicates.
 */
export function executableStatementRanges(sql: string, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): SqlTextRange[] {
  {}
  {}
  return splitSqlStatementRanges(sql, databaseType, parameterOptions).flatMap((statement) => splitStatementRangeAtSoftStarts(sql, statement, databaseType, parameterOptions).map((range) => rangeFor(range, sql)));
}

export function currentExecutableStatementRange(sql: string, cursorPos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): SqlTextRange | null {
  {}
  {}
  return statementRangeAtCursor(sql, cursorPos, databaseType, parameterOptions);
}

export function buildExecutionCandidates(sql: string, cursorPos: number, databaseType?: DatabaseType, parameterOptions?: SqlParameterOptions): SqlExecutionCandidate[] {
  const full = fullSqlRange(sql);
  const cursorStatement = currentExecutableStatementRange(sql, cursorPos, databaseType, parameterOptions);

  if (!full && !cursorStatement) return [];
  if (!full) {
    return cursorStatement ? [candidateFromRange(cursorStatement, "cursor", databaseType)] : [];
  }
  if (!cursorStatement) {
    return [candidateFromRange(full, "all", databaseType)];
  }

  const sameContent = normalizeSql(cursorStatement.sql) === normalizeSql(full.sql);
  if (sameContent) {
    return [candidateFromRange(full, "all", databaseType, ["cursor", "all"])];
  }

  return [candidateFromRange(cursorStatement, "cursor", databaseType), candidateFromRange(full, "all", databaseType)];
}

function candidateFromRange(range: SqlTextRange, kind: SqlExecutionCandidate["kind"], _databaseType?: DatabaseType, supportedKinds: SqlExecutionCandidate["supportedKinds"] = [kind]): SqlExecutionCandidate {
  return {
    kind,
    supportedKinds,
    label: kind === "cursor" ? "currentStatement" : "allStatements",
    sql: range.sql,
    from: range.from,
    to: range.to,
  };
}
