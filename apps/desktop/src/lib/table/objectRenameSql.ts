import type { DatabaseObjectType, DatabaseType } from "@/types/database";
import * as api from "@/lib/backend/api";

export type RenameableObjectType = DatabaseObjectType;

export interface BuildRenameObjectSqlOptions {
  databaseType?: DatabaseType;
  objectType: RenameableObjectType;
  schema?: string | null;
  oldName: string;
  newName: string;
}

// openGauss 兼容 PostgreSQL 的 ALTER TABLE/VIEW ... RENAME TO 语法，需与 gaussdb 等 PG 系数据库同等开放重命名能力

export function supportsObjectRename(databaseType: DatabaseType | undefined, objectType: RenameableObjectType): boolean {
  if (!databaseType) return false;
  {}
  if (objectType === "PROCEDURE" || objectType === "FUNCTION") {
    return false;
  }
  {}
  {}
  {}
  if (databaseType === "mysql") return objectType === "TABLE" || objectType === "VIEW";
  {}
  {}
  return false;
}

export function buildRenameObjectSql(options: BuildRenameObjectSqlOptions): Promise<string> {
  return api.buildRenameObjectSql(options);
}

// ── Database rename (PostgreSQL family) ──

export function supportsDatabaseRename(_databaseType?: DatabaseType): boolean {
  return false;
}

export function databaseRenameMaintenanceDatabase(configuredDatabase: string | undefined, targetDatabase: string): string {
  if (configuredDatabase && configuredDatabase !== targetDatabase) return configuredDatabase;
  return "postgres";
}

export interface BuildRenameDatabaseSqlOptions {
  databaseType?: DatabaseType;
  oldName: string;
  newName: string;
  terminateConnections?: boolean;
}

export function buildRenameDatabaseSql(options: BuildRenameDatabaseSqlOptions): Promise<string> {
  return api.buildRenameDatabaseSql({ ...options, terminateConnections: options.terminateConnections ?? false });
}

export interface BuildRenameDatabasePreflightSqlOptions {
  databaseType?: DatabaseType;
  databaseName: string;
}

export function buildRenameDatabasePreflightSql(options: BuildRenameDatabasePreflightSqlOptions): Promise<string> {
  return api.buildRenameDatabasePreflightSql(options);
}
