import type { ConnectionConfig, DatabaseType, QueryResult } from "@/types/database";
import { supportsDatabaseFeature } from "@/lib/database/databaseDriverManifest";
import { effectiveDatabaseTypeForConnection } from "@/lib/database/jdbcDialect";

export type UserAdminDialect = "mysql";
export type PrivilegeScope = "mysql" | "database" | "schema" | "table" | "role";
export type AuthorizationModel = "mysql";

export interface DatabaseUserIdentity {
  user: string;
  host: string;
  plugin?: string;
}

export interface CreatePrincipalInput extends DatabaseUserIdentity {
  password: string;
  canLogin?: boolean;
}

export interface PrivilegeChangeInput {
  user: DatabaseUserIdentity;
  privileges: string[];
  catalog?: string;
  database: string;
  table?: string;
  grantOption?: boolean;
  scope?: PrivilegeScope;
  role?: string;
}

export interface PrivilegeSelectionInput {
  grants: readonly string[];
  database: string;
  table?: string;
  availablePrivileges: readonly string[];
}

export interface PrivilegeSelection {
  privileges: string[];
  grantOption: boolean;
}

export interface DatabaseTablePrivilegeGrant {
  catalog?: string;
  database: string;
  schema?: string;
  table: string;
  privilege: string;
  grantOption: boolean;
}

export interface TableGrantParseContext {
  database?: string;
}

export interface DatabaseUserAdminProvider {
  dialect: UserAdminDialect;
  defaultScope: PrivilegeScope;
  authorizationModel?: AuthorizationModel;
  supportsTableGrantsOnCreate?: boolean;
  listUsersSql(): string;
  fallbackListUsersSql?: () => string;
  parseUsers(result: QueryResult): DatabaseUserIdentity[];
  parseFallbackUsers?: (result: QueryResult) => DatabaseUserIdentity[];
  showGrantsSql(user: DatabaseUserIdentity): string;
  parseGrants?(result: QueryResult): string[];
  createUserSql?(input: CreatePrincipalInput): string;
  renameUserSql?(user: DatabaseUserIdentity, newHost: string): string;
  supportsOldPassword?: boolean;
  alterPasswordSql?(user: DatabaseUserIdentity, password: string, oldPassword?: string): string;
  alterLoginSql?(user: DatabaseUserIdentity, enabled: boolean): string;
  dropUserSql?(user: DatabaseUserIdentity): string;
  grantPrivilegesSql?(input: PrivilegeChangeInput): string;
  revokePrivilegesSql?(input: PrivilegeChangeInput): string;
  label(user: DatabaseUserIdentity): string;
  detail(user: DatabaseUserIdentity): string | undefined;
  privilegesForScope?(scope: PrivilegeScope): readonly string[];
  defaultPrivilegesForScope?(scope: PrivilegeScope): string[];
  privilegeSelectionFromGrants?(input: PrivilegeSelectionInput): PrivilegeSelection;
  tableGrantsSql?(user: DatabaseUserIdentity): string;
  parseTableGrants?(result: QueryResult, context?: TableGrantParseContext): DatabaseTablePrivilegeGrant[];
  tableGrantsFromShowGrants?: boolean;
}

export const MYSQL_USER_ADMIN_TYPES = new Set<DatabaseType>(["mysql"]);

export const MYSQL_COMMON_PRIVILEGES = ["SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "INDEX", "REFERENCES", "EXECUTE", "SHOW VIEW", "CREATE VIEW", "CREATE ROUTINE", "ALTER ROUTINE", "TRIGGER", "EVENT", "CREATE TEMPORARY TABLES", "LOCK TABLES"] as const;

// StarRocks' v3.0+ privilege framework uses these table action names; legacy
// *_PRIV names belong to the incompatible pre-v3.0 framework.

// MAINTAIN is PostgreSQL 16+, so keep the picker on the cross-version set until
// provider capabilities can be negotiated from the connected server version.

export function quoteSqlString(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

export function quoteMySqlString(value: string): string {
  return `'${value.replace(/\\/g, "\\\\").replace(/'/g, "''")}'`;
}

export function quoteMySqlIdentifier(value: string): string {
  return `\`${value.replace(/`/g, "``")}\``;
}

export function mysqlUserAccount(user: DatabaseUserIdentity): string {
  return `${quoteMySqlString(user.user)}@${quoteMySqlString(user.host)}`;
}

export function mysqlUserLabel(user: DatabaseUserIdentity): string {
  return `${user.user}@${user.host}`;
}

export function mysqlListUsersSql(): string {
  return "SELECT User AS user, Host AS host, plugin AS plugin FROM mysql.user ORDER BY User, Host;";
}

export function mysqlListUsersFallbackSql(): string {
  return "SELECT DISTINCT GRANTEE AS grantee FROM information_schema.USER_PRIVILEGES ORDER BY GRANTEE;";
}

export function mysqlShowGrantsSql(user: DatabaseUserIdentity): string {
  return `SHOW GRANTS FOR ${mysqlUserAccount(user)};`;
}

export function mysqlCreateUserSql(input: CreatePrincipalInput): string {
  return `CREATE USER ${mysqlUserAccount(input)} IDENTIFIED BY ${quoteMySqlString(input.password)};`;
}

export function mysqlRenameUserHostSql(user: DatabaseUserIdentity, newHost: string): string {
  return `RENAME USER ${mysqlUserAccount(user)} TO ${mysqlUserAccount({ ...user, host: newHost })};`;
}

export function mysqlAlterUserPasswordSql(user: DatabaseUserIdentity, password: string): string {
  return `ALTER USER ${mysqlUserAccount(user)} IDENTIFIED BY ${quoteMySqlString(password)};`;
}

export function mysqlAlterUserAccountLockSql(user: DatabaseUserIdentity, locked: boolean): string {
  return `ALTER USER ${mysqlUserAccount(user)} ACCOUNT ${locked ? "LOCK" : "UNLOCK"};`;
}

export function mysqlDropUserSql(user: DatabaseUserIdentity): string {
  return `DROP USER ${mysqlUserAccount(user)};`;
}

export function mysqlPrivilegeTargetSql(database: string, table = "*"): string {
  const db = database.trim() || "*";
  const tbl = table.trim() || "*";
  const dbSql = db === "*" ? "*" : quoteMySqlIdentifier(db);
  const tableSql = tbl === "*" ? "*" : quoteMySqlIdentifier(tbl);
  return `${dbSql}.${tableSql}`;
}

export function mysqlGrantPrivilegesSql(input: PrivilegeChangeInput): string {
  const privileges = normalizePrivileges(input.privileges).join(", ");
  const grantOption = input.grantOption ? " WITH GRANT OPTION" : "";
  return `GRANT ${privileges} ON ${mysqlPrivilegeTargetSql(input.database, input.table)} TO ${mysqlUserAccount(input.user)}${grantOption};`;
}

export function mysqlRevokePrivilegesSql(input: PrivilegeChangeInput): string {
  const privileges = normalizePrivileges(input.privileges).join(", ");
  return `REVOKE ${privileges} ON ${mysqlPrivilegeTargetSql(input.database, input.table)} FROM ${mysqlUserAccount(input.user)};`;
}

export function normalizePrivileges(privileges: string[], fallback = "SELECT"): string[] {
  const normalized = privileges.map((privilege) => privilege.trim().toUpperCase()).filter(Boolean);
  return Array.from(new Set(normalized.length > 0 ? normalized : [fallback]));
}

export const normalizeMySqlPrivileges = normalizePrivileges;

export function mysqlPrivilegeSelectionFromGrants(input: PrivilegeSelectionInput): PrivilegeSelection {
  const database = normalizeMySqlScopeIdentifier(input.database || "*");
  const table = normalizeMySqlScopeIdentifier(input.table || "*");
  const availableByName = new Map(input.availablePrivileges.map((privilege) => [normalizePrivilegeName(privilege), privilege]));
  const selected = new Set<string>();
  let grantOption = false;

  for (const grantSql of input.grants) {
    const grant = parseMySqlGrant(grantSql);
    if (!grant || grant.objectType === "FUNCTION" || grant.objectType === "PROCEDURE") continue;
    if (normalizeMySqlScopeIdentifier(grant.database) !== database || normalizeMySqlScopeIdentifier(grant.table) !== table) continue;

    grantOption ||= grant.grantOption;
    if (grant.allPrivileges) {
      input.availablePrivileges.forEach((privilege) => selected.add(privilege));
      continue;
    }
    grant.privileges.forEach((privilege) => {
      const available = availableByName.get(normalizePrivilegeName(privilege));
      if (available) selected.add(available);
    });
  }

  return {
    privileges: input.availablePrivileges.filter((privilege) => selected.has(privilege)),
    grantOption,
  };
}

export function usersFromMySqlUserResult(result: QueryResult): DatabaseUserIdentity[] {
  const userIndex = columnIndex(result, "user", "User");
  const hostIndex = columnIndex(result, "host", "Host");
  const pluginIndex = columnIndex(result, "plugin", "Plugin");
  if (userIndex < 0 || hostIndex < 0) return [];
  return result.rows
    .map((row) => ({
      user: String(row[userIndex] ?? ""),
      host: String(row[hostIndex] ?? ""),
      plugin: pluginIndex >= 0 && row[pluginIndex] != null ? String(row[pluginIndex]) : undefined,
    }))
    .filter((user) => user.user || user.host);
}

export function usersFromMySqlGranteeResult(result: QueryResult): DatabaseUserIdentity[] {
  const granteeIndex = columnIndex(result, "grantee", "GRANTEE");
  if (granteeIndex < 0) return [];
  return result.rows.flatMap((row) => {
    const parsed = parseMySqlGrantee(String(row[granteeIndex] ?? ""));
    return parsed ? [parsed] : [];
  });
}

export function grantsFromQueryResult(result: QueryResult): string[] {
  return result.rows.map((row) => String(row[0] ?? "")).filter(Boolean);
}

function columnIndex(result: QueryResult, ...names: string[]): number {
  const wanted = new Set(names.map((name) => name.toLowerCase()));
  return result.columns.findIndex((column) => wanted.has(column.toLowerCase()));
}

function parseMySqlGrantee(value: string): DatabaseUserIdentity | null {
  const match = /^'((?:''|[^'])*)'@'((?:''|[^'])*)'$/.exec(value.trim());
  if (!match) return null;
  return {
    user: match[1].replace(/''/g, "'"),
    host: match[2].replace(/''/g, "'"),
  };
}

interface ParsedMySqlGrant {
  privileges: string[];
  database: string;
  table: string;
  objectType?: "TABLE" | "FUNCTION" | "PROCEDURE";
  allPrivileges: boolean;
  grantOption: boolean;
}

function parseMySqlGrant(sql: string): ParsedMySqlGrant | null {
  const identifier = String.raw`(?:\x60(?:\x60\x60|[^\x60])*\x60|\*|[^\s.]+)`;
  const match = new RegExp(String.raw`^\s*GRANT\s+(.+?)\s+ON\s+(?:(TABLE|FUNCTION|PROCEDURE)\s+)?(${identifier})\s*\.\s*(${identifier})\s+TO\s+`, "i").exec(sql);
  if (!match) return null;

  const privileges = match[1].split(",").map(normalizePrivilegeName).filter(Boolean);
  return {
    privileges,
    database: unquoteMySqlIdentifier(match[3]),
    table: unquoteMySqlIdentifier(match[4]),
    objectType: match[2]?.toUpperCase() as ParsedMySqlGrant["objectType"],
    allPrivileges: privileges.some((privilege) => privilege === "ALL" || privilege === "ALL PRIVILEGES"),
    grantOption: /\s+WITH\s+GRANT\s+OPTION\s*;?\s*$/i.test(sql),
  };
}

function normalizePrivilegeName(value: string): string {
  return value.trim().replace(/\s+/g, " ").toUpperCase();
}

function normalizeMySqlScopeIdentifier(value: string): string {
  return unquoteMySqlIdentifier(value.trim()).toLocaleLowerCase("en-US");
}

function unquoteMySqlIdentifier(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length >= 2 && trimmed.startsWith("`") && trimmed.endsWith("`")) {
    return trimmed.slice(1, -1).replace(/``/g, "`");
  }
  return trimmed;
}

export const mysqlUserAdminProvider: DatabaseUserAdminProvider = {
  dialect: "mysql",
  defaultScope: "mysql",
  authorizationModel: "mysql",
  supportsTableGrantsOnCreate: true,
  listUsersSql: mysqlListUsersSql,
  fallbackListUsersSql: mysqlListUsersFallbackSql,
  parseUsers: usersFromMySqlUserResult,
  parseFallbackUsers: usersFromMySqlGranteeResult,
  showGrantsSql: mysqlShowGrantsSql,
  createUserSql: mysqlCreateUserSql,
  alterPasswordSql: mysqlAlterUserPasswordSql,
  alterLoginSql: (user, enabled) => mysqlAlterUserAccountLockSql(user, !enabled),
  dropUserSql: mysqlDropUserSql,
  grantPrivilegesSql: mysqlGrantPrivilegesSql,
  revokePrivilegesSql: mysqlRevokePrivilegesSql,
  label: mysqlUserLabel,
  detail: (user) => user.plugin,
  privilegesForScope: () => MYSQL_COMMON_PRIVILEGES,
  defaultPrivilegesForScope: () => ["SELECT"],
  privilegeSelectionFromGrants: mysqlPrivilegeSelectionFromGrants,
};

export const nativeMysqlUserAdminProvider: DatabaseUserAdminProvider = {
  ...mysqlUserAdminProvider,
  renameUserSql: mysqlRenameUserHostSql,
};

const DATABASE_USER_ADMIN_PROVIDER_BY_TYPE = new Map<DatabaseType, DatabaseUserAdminProvider>([["mysql", mysqlUserAdminProvider]]);

export function getDatabaseUserAdminProvider(dbType: DatabaseType | undefined, _connection?: ConnectionConfig): DatabaseUserAdminProvider | null {
  return dbType === "mysql" ? nativeMysqlUserAdminProvider : null;
}

export function supportsDatabaseUserAdmin(dbType: DatabaseType | undefined): boolean {
  return !!dbType && supportsDatabaseFeature(dbType, "userAdmin") && DATABASE_USER_ADMIN_PROVIDER_BY_TYPE.has(dbType);
}

export function resolveDatabaseUserAdminProviderForConnection(connection: ConnectionConfig | undefined): DatabaseUserAdminProvider | null {
  const dbType = effectiveDatabaseTypeForConnection(connection);
  return supportsDatabaseUserAdmin(dbType) ? getDatabaseUserAdminProvider(dbType, connection) : null;
}

export function connectionSupportsDatabaseUserAdmin(connection: ConnectionConfig | undefined): boolean {
  return resolveDatabaseUserAdminProviderForConnection(connection) !== null;
}
