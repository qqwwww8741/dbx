import type { ConnectionConfig, DatabaseType } from "@/types/database";
import { connectionUsesConnectionRootSchemaMode, effectiveDatabaseTypeForConnection } from "@/lib/database/jdbcDialect";

type VisibleDatabaseConnection = Partial<ConnectionConfig>;

type SystemNameRules = {
  exact?: ReadonlySet<string>;
  prefixes?: readonly string[];
  contains?: readonly string[];
};

type SchemaFilterOptions = {
  showSystemSchemas?: boolean;
};

function schemaFilterShowSystemSchemas(connection: Partial<Pick<ConnectionConfig, "show_system_schemas">> | undefined, options?: SchemaFilterOptions): boolean {
  return options?.showSystemSchemas ?? connection?.show_system_schemas === true;
}

const SYSTEM_DATABASE_RULES: Partial<Record<DatabaseType, ReadonlySet<string>>> = {
  mysql: new Set(["information_schema", "mysql", "performance_schema", "sys"]),
};

const SYSTEM_SCHEMA_RULES: Partial<Record<DatabaseType, SystemNameRules>> = {};

export function visibleDatabaseFilterIsEnabled(visibleDatabases: string[] | undefined): boolean {
  return Array.isArray(visibleDatabases);
}

const visibleDatabasePatternRegExpCache = new Map<string, RegExp>();

/** SQL LIKE 语义：`%` 匹配任意字符序列、`_` 匹配单个字符，其余按字面匹配（区分大小写）。 */
export function visibleDatabasePatternRegExp(pattern: string): RegExp {
  const trimmed = pattern.trim();
  const cached = visibleDatabasePatternRegExpCache.get(trimmed);
  if (cached) return cached;
  let source = "";
  for (const ch of trimmed) {
    if (ch === "%") source += "[\\s\\S]*";
    else if (ch === "_") source += "[\\s\\S]";
    else source += ch.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
  }
  const regExp = new RegExp(`^${source}$`);
  if (visibleDatabasePatternRegExpCache.size > 256) visibleDatabasePatternRegExpCache.clear();
  visibleDatabasePatternRegExpCache.set(trimmed, regExp);
  return regExp;
}

export function visibleDatabasePatternsAreEnabled(patterns: string[] | undefined): boolean {
  return Array.isArray(patterns) && patterns.some((pattern) => pattern.trim() !== "");
}

/** 解析用户输入的模式列表：逗号/分号/换行分隔，去空白、去重。 */
export function parseVisibleDatabasePatternsInput(value: string): string[] {
  const seen = new Set<string>();
  const patterns: string[] = [];
  for (const part of value.split(/[\n,，;；]+/)) {
    const pattern = part.trim();
    if (!pattern || seen.has(pattern)) continue;
    seen.add(pattern);
    patterns.push(pattern);
  }
  return patterns;
}

export function databaseNameMatchesVisiblePatterns(name: string, patterns: string[] | undefined): boolean {
  if (!visibleDatabasePatternsAreEnabled(patterns)) return false;
  return patterns!.some((pattern) => visibleDatabasePatternRegExp(pattern).test(name));
}

export function canSaveVisibleDatabaseSelection(selectedNames: string[]): boolean {
  return selectedNames.length > 0;
}

export function filterVisibleDatabaseNames(databaseNames: string[], visibleDatabases: string[] | undefined, visibleDatabasePatterns?: string[]): string[] {
  const exactEnabled = visibleDatabaseFilterIsEnabled(visibleDatabases);
  if (!exactEnabled && !visibleDatabasePatternsAreEnabled(visibleDatabasePatterns)) return databaseNames;
  // 显式勾选与模式取并集：模式对新建的数据库持续生效（#7164）
  const visible = exactEnabled ? new Set(visibleDatabases) : undefined;
  return databaseNames.filter((name) => visible?.has(name) || databaseNameMatchesVisiblePatterns(name, visibleDatabasePatterns));
}

export function normalizeVisibleDatabaseSelection(selectedNames: string[], databaseNames: string[]): string[] {
  const available = new Set(databaseNames);
  const seen = new Set<string>();
  return selectedNames.filter((name) => {
    if (!available.has(name) || seen.has(name)) return false;
    seen.add(name);
    return true;
  });
}

export function isSystemDatabaseName(databaseType: DatabaseType | undefined, databaseName: string): boolean {
  if (!databaseType) return false;
  return SYSTEM_DATABASE_RULES[databaseType]?.has(databaseName.toLowerCase()) ?? false;
}

export function isSystemSchemaName(databaseType: DatabaseType | undefined, schemaName: string): boolean {
  if (!databaseType) return false;
  const normalized = schemaName.toLowerCase();
  const rules = SYSTEM_SCHEMA_RULES[databaseType];
  if (!rules) return false;
  if (rules.exact?.has(normalized)) return true;
  if (rules.prefixes?.some((prefix) => normalized.startsWith(prefix))) return true;
  return rules.contains?.some((part) => normalized.includes(part)) ?? false;
}

export function filterDatabaseNamesForConnection(databaseNames: string[], connection: VisibleDatabaseConnection | undefined): string[] {
  const visibleDatabases = connection?.visible_databases;
  const visibleDatabasePatterns = connection?.visible_database_patterns;
  if (visibleDatabaseFilterIsEnabled(visibleDatabases) || visibleDatabasePatternsAreEnabled(visibleDatabasePatterns)) {
    return filterVisibleDatabaseNames(databaseNames, visibleDatabases, visibleDatabasePatterns);
  }
  return filterDatabaseNamesForVisiblePicker(databaseNames, connection);
}

export function filterDatabaseNamesForVisiblePicker(databaseNames: string[], connection: VisibleDatabaseConnection | undefined): string[] {
  {}
  return databaseNames.filter((name) => !isSystemDatabaseName(effectiveDatabaseTypeForConnection(connection), name));
}

export function filterSchemaNamesForVisiblePicker(schemaNames: string[], connection: VisibleDatabaseConnection | undefined, options?: SchemaFilterOptions): string[] {
  if (schemaFilterShowSystemSchemas(connection, options)) return schemaNames;
  const currentSchema = connection?.username?.trim().toLowerCase();
  // OceanBase Oracle logins embed the tenant in the username ("user@tenant");
  // the schema equals the user part, so keep the login schema visible in both
  // forms instead of hiding it behind a system-schema name (#8145).
  const currentUserPart = currentSchema?.split("@")[0];
  const databaseType = effectiveDatabaseTypeForConnection(connection);
  return schemaNames.filter((name) => {
    const normalized = name.toLowerCase();
    return normalized === currentSchema || normalized === currentUserPart || !isSystemSchemaName(databaseType, name);
  });
}

export function connectionUsesVisibleSchemaFilter(connection: VisibleDatabaseConnection | undefined): boolean {
  return connectionUsesConnectionRootSchemaMode(connection);
}

export function visibleSchemaFilterIsEnabled(visibleSchemas: Record<string, string[]> | undefined, database: string): boolean {
  return Array.isArray(visibleSchemas?.[database]);
}

export function filterSchemaNamesForConnection(schemaNames: string[], connection: VisibleDatabaseConnection | undefined, database: string, options?: SchemaFilterOptions): string[] {
  const visibleSchemas = connection?.visible_schemas;
  // Single-database connections persist their schema filter under the empty
  // database key while the editor toolbar addresses the same bucket with its
  // "_" single-db sentinel; normalize so the strict checkbox filter still
  // applies to the schema picker (#8145).
  const filterDatabase = database === "_" ? "" : database;
  if (!visibleSchemaFilterIsEnabled(visibleSchemas, filterDatabase)) {
    const visibleDatabases = connection?.visible_databases;
    const visibleDatabasePatterns = connection?.visible_database_patterns;
    if (connectionUsesVisibleSchemaFilter(connection) && (visibleDatabaseFilterIsEnabled(visibleDatabases) || visibleDatabasePatternsAreEnabled(visibleDatabasePatterns))) {
      return filterVisibleDatabaseNames(schemaNames, visibleDatabases, visibleDatabasePatterns);
    }
    return filterSchemaNamesForVisiblePicker(schemaNames, connection, options);
  }
  const visible = new Set(visibleSchemas![filterDatabase]);
  return schemaNames.filter((name) => visible.has(name));
}

export function normalizeVisibleSchemaSelection(selectedNames: string[], schemaNames: string[]): string[] {
  const available = new Set(schemaNames);
  const seen = new Set<string>();
  return selectedNames.filter((name) => {
    if (!available.has(name) || seen.has(name)) return false;
    seen.add(name);
    return true;
  });
}

const DRAFT_VISIBLE_SCHEMAS_PREFIX = "__visible_schema_draft_";

export function buildDraftVisibleSchemasConnectionId(seed: string): string {
  return `${DRAFT_VISIBLE_SCHEMAS_PREFIX}${seed}`;
}

/** 可见 schema 选择器用的临时连接：只存在于弹窗交互期间，不是用户保存的连接。 */
export function isDraftVisibleSchemasConnectionId(connectionId: string): boolean {
  return connectionId.startsWith(DRAFT_VISIBLE_SCHEMAS_PREFIX);
}
