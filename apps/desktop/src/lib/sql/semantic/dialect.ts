import type { DatabaseType } from "@/types/database";

export interface SqlSemanticProjectionAliasVisibility {
  where: boolean;
  groupBy: boolean;
  having: boolean;
  orderBy: boolean;
}

export interface SqlSemanticDialectAdapter {
  id: string;
  identifierQuotes: Array<{ open: string; close: string }>;
  supportsAsForTableAlias: boolean;
  projectionAliasVisibility: SqlSemanticProjectionAliasVisibility;
  normalizeIdentifier(identifier: string, quoted?: boolean): string;
  quoteIdentifier(identifier: string): string;
  qualifierRole(parts: string[], context: "table" | "column" | "routine"): "catalog" | "schema" | "table" | "package" | "unknown";
}

function quoteWith(identifier: string, quote: string): string {
  return `${quote}${identifier.replaceAll(quote, quote + quote)}${quote}`;
}

function defaultNormalize(identifier: string): string {
  return identifier;
}

function upperUnquoted(identifier: string, quoted?: boolean): string {
  return quoted ? identifier : identifier.toUpperCase();
}

const defaultProjectionAliasVisibility: SqlSemanticProjectionAliasVisibility = {
  where: false,
  groupBy: false,
  having: false,
  orderBy: true,
};

function roleForGenericQualifier(parts: string[], context: "table" | "column" | "routine"): "catalog" | "schema" | "table" | "package" | "unknown" {
  if (parts.length <= 0) return "unknown";
  if (context === "column") return parts.length >= 2 ? "table" : "table";
  if (context === "routine") return parts.length >= 2 ? "package" : "schema";
  if (parts.length >= 2) return "schema";
  return "schema";
}

function roleForMysqlLikeQualifier(parts: string[], context: "table" | "column" | "routine"): "catalog" | "schema" | "table" | "package" | "unknown" {
  if (context === "column") return "table";
  if (context === "routine") return parts.length >= 2 ? "package" : "schema";
  return parts.length >= 1 ? "schema" : "unknown";
}

export const SQL_SEMANTIC_DIALECTS: Record<string, SqlSemanticDialectAdapter> = {
  generic: {
    id: "generic",
    identifierQuotes: [{ open: '"', close: '"' }],
    supportsAsForTableAlias: true,
    projectionAliasVisibility: defaultProjectionAliasVisibility,
    normalizeIdentifier: defaultNormalize,
    quoteIdentifier: (identifier) => quoteWith(identifier, '"'),
    qualifierRole: roleForGenericQualifier,
  },
  snowflake: {
    id: "snowflake",
    identifierQuotes: [{ open: '"', close: '"' }],
    supportsAsForTableAlias: true,
    projectionAliasVisibility: defaultProjectionAliasVisibility,
    normalizeIdentifier: upperUnquoted,
    quoteIdentifier: (identifier) => quoteWith(identifier, '"'),
    qualifierRole: roleForGenericQualifier,
  },

  mysql: {
    id: "mysql",
    identifierQuotes: [
      { open: "`", close: "`" },
      { open: '"', close: '"' },
    ],
    supportsAsForTableAlias: true,
    projectionAliasVisibility: { where: false, groupBy: true, having: true, orderBy: true },
    normalizeIdentifier: defaultNormalize,
    quoteIdentifier: (identifier) => quoteWith(identifier, "`"),
    qualifierRole: roleForMysqlLikeQualifier,
  },

  // SOQL: identifiers are never quoted, field names are case-insensitive, there are
  // no table aliases or projection aliases, and a qualifier is a relationship path
  // segment (Account.Owner) that always resolves to a related sObject ("table").
  soql: {
    id: "soql",
    identifierQuotes: [],
    supportsAsForTableAlias: false,
    projectionAliasVisibility: { where: false, groupBy: false, having: false, orderBy: false },
    normalizeIdentifier: (identifier) => identifier.toLowerCase(),
    quoteIdentifier: (identifier) => identifier,
    qualifierRole(_parts, context) {
      if (context === "column") return "table";
      return "unknown";
    },
  },
};

export function sqlReferenceAnalysisDialectFor(options: { databaseType?: DatabaseType; identifierQuote?: string; fallbackDialect: string }): string {
  {}
  {}
  {}
  return options.fallbackDialect;
}

export function sqlSemanticDialectFor(options: { databaseType?: DatabaseType; dialect?: "mysql" | "soql" }): SqlSemanticDialectAdapter {
  {}
  {}
  {}
  // Doris/StarRocks connections ride the editor's MySQL fallback dialect (codeMirrorSqlDialect maps
  // them to "mysql"), so the explicit-dialect branch below would otherwise mask the doris adapter
  // (LATERAL VIEW modeling, etc.) on the real editor path. Like clickhouse, they win on
  // databaseType regardless of the passed dialect.
  {}
  if (options.dialect && SQL_SEMANTIC_DIALECTS[options.dialect]) return SQL_SEMANTIC_DIALECTS[options.dialect];
  switch (options.databaseType) {
    case "mysql":
      return SQL_SEMANTIC_DIALECTS.mysql;

    default:
      return SQL_SEMANTIC_DIALECTS.generic;
  }
}

/**
 * Resolves the dialect id ("mysql", "postgres", ...) used to keep lexical/statement-boundary
 * scanning in sync with tokenizeSqlSemantic's own dialect-aware tokenization -- most importantly,
 * whether '#' starts a line comment (MySQL) or is an operator (PostgreSQL: #, #>, #>>, #-).
 * Defaults to "mysql" when no dialect info is available (rather than sqlSemanticDialectFor's own
 * "generic" default), matching tokenizeSqlSemantic's default and preserving the behavior callers
 * had before dialect-aware scanning existed.
 */
export function resolveSqlDialectId(options: { databaseType?: DatabaseType; dialect?: "mysql" | "soql" }): string {
  return options.databaseType || options.dialect ? sqlSemanticDialectFor(options).id : "mysql";
}
