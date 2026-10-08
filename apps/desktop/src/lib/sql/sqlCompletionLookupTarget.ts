import type { SqlCompletionContext, SqlCompletionItem } from "@/lib/sql/sqlCompletion";

import type { DatabaseType } from "@/types/database";

export interface SqlCompletionTableLookupTarget {
  database: string;
  schema?: string;
  filter: string;
  qualifierDatabase?: string;
}

export interface SqlCompletionRoutineLookupTarget {
  database: string;
  schema?: string;
  mask: string;
}

export interface SqlCompletionScope {
  database: string;
  schema?: string;
  completionContext: SqlCompletionContext;
}

export interface SqlServerUseDatabaseCompletion {
  from: number;
  prefix: string;
  quoteStyle: "none" | "bracket" | "double";
}

function sqlStatementWithoutLeadingComments(statement: string): string {
  let remaining = statement.trimStart();
  while (remaining) {
    if (remaining.startsWith("--")) {
      const newline = remaining.indexOf("\n");
      remaining = newline < 0 ? "" : remaining.slice(newline + 1).trimStart();
      continue;
    }
    if (remaining.startsWith("/*")) {
      const end = remaining.indexOf("*/", 2);
      if (end < 0) return "";
      remaining = remaining.slice(end + 2).trimStart();
      continue;
    }
    break;
  }
  return remaining;
}

/**
 * 方言里 `USE <db>` 会把会话切到另一个库，DBX 的标签库名应当跟着走（#9941）。
 *
 * 只列出已经确认过这一语义的方言：MySQL 及其 wire-protocol 家族。SQL Server 由
 * `sqlServerUseDatabaseFromStatement` 单独处理（它还接受 `[db]` 括号标识符）。
 */
const USE_DATABASE_SWITCH_DIALECTS: ReadonlySet<DatabaseType> = new Set<DatabaseType>(["mysql"]);

export function switchesDatabaseWithUseStatement(databaseType: DatabaseType | null | undefined): boolean {
  return !!databaseType && USE_DATABASE_SWITCH_DIALECTS.has(databaseType);
}

/**
 * 解析单条语句里「成功切换当前库」的 `USE <db>`，返回目标库名（反引号、双引号或裸
 * 标识符）。不是 USE 语句、或该方言的 USE 不改库时返回 undefined。
 */
export function useDatabaseFromStatement(statement: string, databaseType?: DatabaseType): string | undefined {
  {}
  if (!switchesDatabaseWithUseStatement(databaseType)) return undefined;
  const match = /^USE\s+(?:`((?:[^`]|``)*)`|"((?:[^"]|"")*)"|([\p{L}_$][\p{L}\p{N}_$]*))\s*;?\s*$/iu.exec(sqlStatementWithoutLeadingComments(statement));
  if (!match) return undefined;
  if (match[1] !== undefined) return match[1].replaceAll("``", "`");
  if (match[2] !== undefined) return match[2].replaceAll('""', '"');
  return match[3];
}

export interface SqlServerLeadingUseScript {
  querySql: string;
  queryFrom: number;
  queryTo: number;
  database: string;
}

export function resolveSqlServerUseDatabaseCompletion(_options: { sql: string; cursor: number; databaseType?: DatabaseType }): SqlServerUseDatabaseCompletion | undefined {
  {
    return undefined;
  }
}

export function buildSqlServerUseDatabaseCompletionItems(databaseNames: readonly string[], completion: SqlServerUseDatabaseCompletion): SqlCompletionItem[] {
  return databaseNames.map((database) => {
    const escapedDatabase = completion.quoteStyle === "double" ? database.replaceAll('"', '""') : database.replaceAll("]", "]]");
    const apply = completion.quoteStyle === "bracket" ? `${escapedDatabase}]` : completion.quoteStyle === "double" ? `${escapedDatabase}"` : `[${escapedDatabase}]`;
    return {
      label: database,
      filterText: completion.quoteStyle === "none" ? database : escapedDatabase,
      type: "schema",
      detail: "database",
      apply,
      boost: 1_500,
    };
  });
}

export function resolveSqlCompletionScope(options: {
  sql: string;
  cursor: number;
  databaseType?: DatabaseType;
  currentDatabase: string;
  currentSchema?: string;
  knownDatabases?: readonly string[];
  supportsSessionDatabaseSwitch?: boolean;
  useDatabaseDefaultSchema?: string;
  completionContext: SqlCompletionContext;
}): SqlCompletionScope {
  {
    return {
      database: options.currentDatabase,
      schema: options.currentSchema,
      completionContext: options.completionContext,
    };
  }
}

function findExactName(names: readonly string[] | undefined, value: string): string | undefined {
  return names?.find((name) => name.toLowerCase() === value.toLowerCase());
}

function findCaseSensitiveName(names: readonly string[] | undefined, value: string): string | undefined {
  return names?.find((name) => name === value);
}

export function mergeSqlCompletionQualifierNames(primary: readonly string[], secondary: readonly string[]): string[] {
  return [...new Set([...primary, ...secondary])];
}

export function resolveSqlCompletionSchemaLookupDatabase(options: {
  supportsDatabaseSchemaQualifier?: boolean;
  completionContext: Pick<SqlCompletionContext, "qualifier" | "qualifierParts" | "suggestTables" | "insertTable">;
  knownDatabases?: readonly string[];
  knownSchemas?: readonly string[];
}): string | undefined {
  const { completionContext } = options;
  if (!options.supportsDatabaseSchemaQualifier || !completionContext.suggestTables || completionContext.insertTable) return undefined;
  const qualifier = completionContext.qualifier?.trim();
  const qualifierParts = completionContext.qualifierParts?.filter(Boolean) ?? qualifier?.split(".").filter(Boolean) ?? [];
  if (qualifierParts.length !== 1) return undefined;
  if (findCaseSensitiveName(options.knownSchemas, qualifierParts[0]!)) return undefined;
  return findCaseSensitiveName(options.knownDatabases, qualifierParts[0]!);
}

export function resolveSqlCompletionTableLookupTarget(options: {
  databaseType?: DatabaseType;
  currentDatabase: string;
  currentSchema?: string;
  supportsDatabaseQualifier: boolean;
  supportsDatabaseSchemaQualifier?: boolean;
  completionContext: Pick<SqlCompletionContext, "qualifier" | "qualifierParts" | "prefix" | "suggestTables" | "insertTable">;
  knownDatabases?: readonly string[];
}): SqlCompletionTableLookupTarget {
  const { completionContext } = options;
  const qualifier = completionContext.qualifier?.trim();
  const qualifierParts = completionContext.qualifierParts?.filter(Boolean) ?? qualifier?.split(".").filter(Boolean) ?? [];
  if (options.supportsDatabaseSchemaQualifier && completionContext.suggestTables && !completionContext.insertTable && qualifierParts.length >= 2) {
    const databaseQualifier = qualifierParts[qualifierParts.length - 2]!;
    const schema = qualifierParts[qualifierParts.length - 1]!;
    const database = findExactName(options.knownDatabases, databaseQualifier) ?? databaseQualifier;
    return {
      database,
      schema,
      filter: completionContext.prefix,
      qualifierDatabase: database,
    };
  }
  const qualifierIsDatabase = options.supportsDatabaseQualifier && !!qualifier && completionContext.suggestTables && !completionContext.insertTable;

  if (qualifierIsDatabase) {
    // MySQL-compatible engines, including OceanBase MySQL mode, use
    // database.table. Do not block table completion on a separate database-list
    // request when the user already typed the database qualifier.
    const database = findExactName(options.knownDatabases, qualifier) ?? qualifier;
    return {
      database,
      filter: completionContext.prefix,
      qualifierDatabase: database,
    };
  }

  return {
    database: options.currentDatabase,
    schema: qualifier && completionContext.suggestTables ? qualifier : options.currentSchema,
    filter: qualifier && completionContext.suggestTables ? completionContext.prefix : qualifier || completionContext.prefix,
  };
}

export function resolveSqlCompletionRoutineLookupTarget(options: { currentDatabase: string; currentSchema?: string; supportsDatabaseSchemaQualifier?: boolean; completionContext: Pick<SqlCompletionContext, "qualifier" | "qualifierParts" | "prefix"> }): SqlCompletionRoutineLookupTarget {
  const qualifier = options.completionContext.qualifier?.trim();
  const qualifierParts = options.completionContext.qualifierParts?.filter(Boolean) ?? qualifier?.split(".").filter(Boolean) ?? [];
  const hasDatabaseQualifier = options.supportsDatabaseSchemaQualifier && qualifierParts.length >= 2;
  const database = hasDatabaseQualifier ? qualifierParts[qualifierParts.length - 2]! : options.currentDatabase;
  const schema = qualifierParts[qualifierParts.length - 1] ?? qualifier ?? options.currentSchema;

  // A qualified routine uses the qualifier as metadata scope; only the final
  // identifier fragment is the function/procedure name mask.
  return {
    database,
    schema: schema || undefined,
    mask: options.completionContext.prefix,
  };
}
