import * as api from "@/lib/backend/api";
import type { DatabaseType, QueryResult, RoutineParameterMetadata } from "@/types/database";
import type { RoutineParameter, RoutineParameterMode } from "@/lib/table/routineExecutionSql";

export interface LoadRoutineParametersOptions {
  connectionId: string;
  database: string;
  databaseType?: DatabaseType;
  schema?: string;
  routineName: string;
}

export async function loadRoutineParameters(options: LoadRoutineParametersOptions): Promise<RoutineParameter[]> {
  {}
  const sql = routineParametersQuery(options);
  if (!sql) return [];
  const result = await api.executeQuery(options.connectionId, options.database, sql, options.schema, undefined, {
    maxRows: 200,
    pageSize: 200,
  });
  return routineParametersFromResult(result, options.databaseType);
}

export function supportsRoutineParameterMetadata(databaseType?: DatabaseType): boolean {
  return databaseType === "mysql";
}

export function routineParametersQuery(options: Pick<LoadRoutineParametersOptions, "database" | "databaseType" | "schema" | "routineName">): string | null {
  if (!supportsRoutineParameterMetadata(options.databaseType)) return null;
  const effectiveSchema = options.schema || "" || (options.databaseType === "mysql" ? options.database : "");
  const schema = quoteSqlLiteral(effectiveSchema);
  const name = quoteSqlLiteral(options.routineName);
  {}
  if (options.databaseType === "mysql") {
    return `
SELECT
  PARAMETER_NAME AS name,
  DTD_IDENTIFIER AS data_type,
  COALESCE(PARAMETER_MODE, 'IN') AS mode,
  ORDINAL_POSITION AS ordinal,
  FALSE AS has_default
FROM information_schema.PARAMETERS
WHERE SPECIFIC_SCHEMA = ${schema}
  AND SPECIFIC_NAME = ${name}
  AND ORDINAL_POSITION > 0
ORDER BY ORDINAL_POSITION;`.trim();
  }
  {}
  {}
  {}
  return null;
}

export interface RoutineMetadata {
  kind?: "PROCEDURE" | "FUNCTION";
  parameters: RoutineParameter[];
  returnType?: string;
}

export type XuguRoutineMetadata = RoutineMetadata;

/** Convert optional generic-JDBC wire metadata into the existing read-only panel model. */
export function jdbcRoutineMetadata(parameters: RoutineParameterMetadata[] | undefined): RoutineMetadata | null {
  if (parameters === undefined) return null;

  const ordered = parameters
    .map((parameter, index) => ({ parameter, index }))
    .sort((left, right) => routineMetadataOrdinal(left.parameter) - routineMetadataOrdinal(right.parameter) || left.index - right.index)
    .map(({ parameter }) => parameter);
  const returnParameter = ordered.find((parameter) => parameter.mode === "RETURN");
  return {
    parameters: ordered
      .filter((parameter) => parameter.mode !== "RETURN")
      .map((parameter, index) => ({
        name: parameter.name?.trim() || `arg${positiveRoutineOrdinal(parameter.ordinal) ?? index + 1}`,
        dataType: jdbcRoutineParameterType(parameter),
        mode: parameter.mode,
        ordinal: positiveRoutineOrdinal(parameter.ordinal) ?? index + 1,
        hasDefault: false,
        nullable: parameter.nullable,
      })),
    returnType: returnParameter ? jdbcRoutineParameterType(returnParameter) : undefined,
  };
}

function routineMetadataOrdinal(parameter: RoutineParameterMetadata): number {
  return typeof parameter.ordinal === "number" && Number.isFinite(parameter.ordinal) ? parameter.ordinal : Number.MAX_SAFE_INTEGER;
}

function positiveRoutineOrdinal(ordinal: number | null | undefined): number | undefined {
  return typeof ordinal === "number" && Number.isFinite(ordinal) && ordinal > 0 ? ordinal : undefined;
}

function jdbcRoutineParameterType(parameter: RoutineParameterMetadata): string {
  const typeName = parameter.type_name?.trim() || (typeof parameter.jdbc_type === "number" ? `JDBC ${parameter.jdbc_type}` : "UNKNOWN");
  if (/\([^)]*\)\s*$/.test(typeName)) return typeName;

  const jdbcType = parameter.jdbc_type;
  const precision = positiveMetadataSize(parameter.precision);
  const length = positiveMetadataSize(parameter.length);
  const scale = typeof parameter.scale === "number" && Number.isFinite(parameter.scale) && parameter.scale >= 0 ? parameter.scale : undefined;
  if (jdbcType === 2 || jdbcType === 3) {
    if (precision === undefined) return typeName;
    return scale === undefined ? `${typeName}(${precision})` : `${typeName}(${precision},${scale})`;
  }
  if (jdbcType === 92 || jdbcType === 93 || jdbcType === 2013 || jdbcType === 2014) {
    return scale === undefined ? typeName : `${typeName}(${scale})`;
  }
  if (jdbcType === 1 || jdbcType === 12 || jdbcType === -1 || jdbcType === -15 || jdbcType === -9 || jdbcType === -16 || jdbcType === -2 || jdbcType === -3 || jdbcType === -4) {
    const size = length ?? precision;
    return size === undefined ? typeName : `${typeName}(${size})`;
  }
  return typeName;
}

function positiveMetadataSize(value: number | null | undefined): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : undefined;
}

interface XuguRoutineToken {
  kind: "word" | "quoted-identifier" | "string" | "symbol";
  text: string;
  start: number;
  end: number;
}

/**
 * XuguDB does not expose ALL_ARGUMENTS on every supported server version.
 * Its DBeaver extension therefore parses ALL_PROCEDURES.DEFINE as well. Keep
 * this parser deliberately limited to the declaration header: the PL/SQL body
 * is never interpreted and malformed definitions fail closed with no metadata.
 */
export function xuguRoutineMetadataFromDefinition(definition: string): XuguRoutineMetadata {
  const tokens = tokenizeXuguRoutineDefinition(definition);
  if (!tokens) return { parameters: [] };
  const kindIndex = tokens.findIndex((token) => isWord(token, "PROCEDURE") || isWord(token, "FUNCTION"));
  if (kindIndex < 0) return { parameters: [] };

  const kind = tokens[kindIndex].text.toUpperCase() as "PROCEDURE" | "FUNCTION";
  const nameEndIndex = xuguRoutineNameEndIndex(tokens, kindIndex + 1);
  if (nameEndIndex < 0) return { kind, parameters: [] };

  let headerIndex = nameEndIndex;
  let parameterCloseIndex = -1;
  let parameters: RoutineParameter[] = [];
  if (tokens[headerIndex]?.text === "(") {
    parameterCloseIndex = matchingTokenParenIndex(tokens, headerIndex);
    if (parameterCloseIndex < 0) return { kind, parameters: [] };
    parameters = parseXuguRoutineParameters(definition, tokens, headerIndex + 1, parameterCloseIndex);
    headerIndex = parameterCloseIndex + 1;
  }

  const returnType = kind === "FUNCTION" ? xuguFunctionReturnType(definition, tokens, headerIndex) : undefined;
  return { kind, parameters, returnType };
}

function tokenizeXuguRoutineDefinition(definition: string): XuguRoutineToken[] | null {
  const tokens: XuguRoutineToken[] = [];
  let index = 0;
  while (index < definition.length) {
    const char = definition[index];
    if (/\s/.test(char)) {
      index += 1;
      continue;
    }
    if (char === "-" && definition[index + 1] === "-") {
      index += 2;
      while (index < definition.length && definition[index] !== "\n" && definition[index] !== "\r") index += 1;
      continue;
    }
    if (char === "/" && definition[index + 1] === "*") {
      const close = definition.indexOf("*/", index + 2);
      if (close < 0) return null;
      index = close + 2;
      continue;
    }
    if (char === "'" || char === '"') {
      const start = index;
      const quote = char;
      let closed = false;
      index += 1;
      while (index < definition.length) {
        if (definition[index] !== quote) {
          index += 1;
          continue;
        }
        if (definition[index + 1] === quote) {
          index += 2;
          continue;
        }
        index += 1;
        closed = true;
        break;
      }
      if (!closed) return null;
      tokens.push({ kind: quote === "'" ? "string" : "quoted-identifier", text: definition.slice(start, index), start, end: index });
      continue;
    }
    if (/[A-Za-z_#$]/.test(char)) {
      const start = index;
      index += 1;
      while (index < definition.length && /[A-Za-z0-9_#$%]/.test(definition[index])) index += 1;
      tokens.push({ kind: "word", text: definition.slice(start, index), start, end: index });
      continue;
    }
    const start = index;
    if (char === ":" && definition[index + 1] === "=") index += 2;
    else index += 1;
    tokens.push({ kind: "symbol", text: definition.slice(start, index), start, end: index });
  }
  return tokens;
}

function xuguRoutineNameEndIndex(tokens: XuguRoutineToken[], startIndex: number): number {
  const first = tokens[startIndex];
  if (!isIdentifierToken(first)) return -1;
  let index = startIndex + 1;
  while (tokens[index]?.text === "." && isIdentifierToken(tokens[index + 1])) index += 2;
  return index;
}

function isIdentifierToken(token?: XuguRoutineToken): boolean {
  return token?.kind === "word" || token?.kind === "quoted-identifier";
}

function isWord(token: XuguRoutineToken | undefined, word: string): boolean {
  return token?.kind === "word" && token.text.toUpperCase() === word;
}

function matchingTokenParenIndex(tokens: XuguRoutineToken[], openIndex: number): number {
  let depth = 0;
  for (let index = openIndex; index < tokens.length; index += 1) {
    if (tokens[index].text === "(") depth += 1;
    if (tokens[index].text === ")") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

function parseXuguRoutineParameters(definition: string, tokens: XuguRoutineToken[], startIndex: number, endIndex: number): RoutineParameter[] {
  const ranges: Array<[number, number]> = [];
  let depth = 0;
  let rangeStart = startIndex;
  for (let index = startIndex; index < endIndex; index += 1) {
    if (tokens[index].text === "(") depth += 1;
    if (tokens[index].text === ")") depth = Math.max(0, depth - 1);
    if (tokens[index].text === "," && depth === 0) {
      ranges.push([rangeStart, index]);
      rangeStart = index + 1;
    }
  }
  ranges.push([rangeStart, endIndex]);

  return ranges.flatMap(([start, end], ordinalIndex) => {
    const parameter = parseXuguRoutineParameter(definition, tokens, start, end, ordinalIndex + 1);
    return parameter ? [parameter] : [];
  });
}

function parseXuguRoutineParameter(definition: string, tokens: XuguRoutineToken[], startIndex: number, endIndex: number, ordinal: number): RoutineParameter | null {
  if (startIndex >= endIndex || !isIdentifierToken(tokens[startIndex])) return null;
  const name = unquoteXuguIdentifier(tokens[startIndex].text);
  let typeStartIndex = startIndex + 1;
  let mode: RoutineParameterMode = "IN";
  if (isWord(tokens[typeStartIndex], "INOUT")) {
    mode = "INOUT";
    typeStartIndex += 1;
  } else if (isWord(tokens[typeStartIndex], "IN")) {
    if (isWord(tokens[typeStartIndex + 1], "OUT")) {
      mode = "INOUT";
      typeStartIndex += 2;
    } else {
      mode = "IN";
      typeStartIndex += 1;
    }
  } else if (isWord(tokens[typeStartIndex], "OUT")) {
    mode = "OUT";
    typeStartIndex += 1;
  }

  let depth = 0;
  let defaultIndex = -1;
  for (let index = typeStartIndex; index < endIndex; index += 1) {
    if (tokens[index].text === "(") depth += 1;
    if (tokens[index].text === ")") depth = Math.max(0, depth - 1);
    if (depth === 0 && (isWord(tokens[index], "DEFAULT") || tokens[index].text === ":=" || tokens[index].text === "=")) {
      defaultIndex = index;
      break;
    }
  }

  const typeEndIndex = defaultIndex >= 0 ? defaultIndex : endIndex;
  if (typeStartIndex >= typeEndIndex) return null;
  const dataType = xuguTokenRangeText(definition, tokens, typeStartIndex, typeEndIndex).replace(/\s+/g, " ");
  if (!dataType) return null;
  const defaultValue = defaultIndex >= 0 ? xuguTokenRangeText(definition, tokens, defaultIndex + 1, endIndex) : undefined;
  if (defaultIndex >= 0 && !defaultValue) return null;
  return {
    name,
    dataType,
    mode,
    ordinal,
    hasDefault: defaultIndex >= 0,
    defaultValue,
  };
}

function xuguFunctionReturnType(definition: string, tokens: XuguRoutineToken[], startIndex: number): string | undefined {
  let returnIndex = -1;
  for (let index = startIndex; index < tokens.length; index += 1) {
    if (isWord(tokens[index], "AS") || isWord(tokens[index], "IS")) break;
    if (isWord(tokens[index], "RETURN")) {
      returnIndex = index;
      break;
    }
  }
  if (returnIndex < 0) return undefined;
  let endIndex = returnIndex + 1;
  let depth = 0;
  while (endIndex < tokens.length) {
    const token = tokens[endIndex];
    if (token.text === "(") depth += 1;
    if (token.text === ")") depth = Math.max(0, depth - 1);
    if (depth === 0 && (isWord(token, "AS") || isWord(token, "IS") || isWord(token, "AUTHID") || isWord(token, "PIPELINED") || isWord(token, "DETERMINISTIC"))) break;
    endIndex += 1;
  }
  if (returnIndex + 1 >= endIndex) return undefined;
  return xuguTokenRangeText(definition, tokens, returnIndex + 1, endIndex).replace(/\s+/g, " ") || undefined;
}

function xuguTokenRangeText(definition: string, tokens: XuguRoutineToken[], startIndex: number, endIndex: number): string {
  if (startIndex >= endIndex) return "";
  return stripXuguSqlComments(definition.slice(tokens[startIndex].start, tokens[endIndex - 1].end)).trim();
}

function stripXuguSqlComments(value: string): string {
  let result = "";
  let index = 0;
  let quote = "";
  while (index < value.length) {
    const char = value[index];
    if (quote) {
      result += char;
      if (char === quote) {
        if (value[index + 1] === quote) {
          result += value[index + 1];
          index += 2;
          continue;
        }
        quote = "";
      }
      index += 1;
      continue;
    }
    if (char === "'" || char === '"') {
      quote = char;
      result += char;
      index += 1;
      continue;
    }
    if (char === "-" && value[index + 1] === "-") {
      result += " ";
      index += 2;
      while (index < value.length && value[index] !== "\n" && value[index] !== "\r") index += 1;
      continue;
    }
    if (char === "/" && value[index + 1] === "*") {
      result += " ";
      const close = value.indexOf("*/", index + 2);
      if (close < 0) break;
      index = close + 2;
      continue;
    }
    result += char;
    index += 1;
  }
  return result;
}

function unquoteXuguIdentifier(value: string): string {
  if (!value.startsWith('"') || !value.endsWith('"')) return value;
  return value.slice(1, -1).replace(/""/g, '"');
}

export function routineParametersFromResult(result: QueryResult, _databaseType?: DatabaseType): RoutineParameter[] {
  {}

  return result.rows
    .map((row, index) => {
      const dataType = String(row[1] || "");
      return {
        name: String(row[0] || `arg${index + 1}`),
        dataType: dataType,
        mode: normalizeParameterMode(row[2]),
        ordinal: Number(row[3] || index + 1),
        hasDefault: normalizeBoolean(row[4]),
      };
    })
    .filter((parameter) => parameter.mode !== "RETURN");
}

function normalizeParameterMode(value: unknown): RoutineParameterMode {
  const mode = String(value || "IN")
    .toUpperCase()
    .replace(/\s+/g, "");
  if (mode === "IN") return "IN";
  if (mode === "OUT") return "OUT";
  if (mode === "INOUT" || mode === "IN/OUT") return "INOUT";
  if (mode === "RETURN") return "RETURN";
  return "UNKNOWN";
}

function normalizeBoolean(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  const text = String(value || "").toLowerCase();
  return text === "true" || text === "yes" || text === "y" || text === "1";
}

function quoteSqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}
