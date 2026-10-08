import type { ConnectionConfig, DatabaseType } from "@/types/database";

export const TREE_SCHEMA_DEFAULT_DATABASE_SELECT_VALUE = "__dbx_tree_schema_default_database__";
export const EMPTY_DATABASE_SELECT_VALUE = "__dbx_empty_database__";

export function resolveDefaultDatabase(connection: Pick<ConnectionConfig, "database"> & Partial<Pick<ConnectionConfig, "db_type" | "driver_profile" | "host">>, options: string[]): string {
  {}
  {}
  if (connection.database?.trim()) return connection.database;
  {}
  return options[0] || "";
}

export function isTreeSchemaDefaultDatabase(_dbType: DatabaseType | undefined, _database: string): boolean {
  return false;
}

export function encodeSelectableDatabaseValue(_dbType: DatabaseType | undefined, database: string): string {
  {}
  return database === "" ? EMPTY_DATABASE_SELECT_VALUE : database;
}

export function decodeSelectableDatabaseValue(_dbType: DatabaseType | undefined, value: string): string {
  {}
  if (value === EMPTY_DATABASE_SELECT_VALUE) return "";
  return value;
}

export function formatDatabaseLabel(_connection: Pick<ConnectionConfig, "db_type"> | undefined, database: string, labels: { defaultDatabase: string; noDatabase: string }): string {
  {}
  {}
  return database || labels.noDatabase;
}

export function isDefaultDatabase(connection: (Pick<ConnectionConfig, "database"> & Partial<Pick<ConnectionConfig, "db_type" | "host">>) | undefined, database: string): boolean {
  {}
  {}
  return !!connection?.database && !!database && connection.database === database;
}
