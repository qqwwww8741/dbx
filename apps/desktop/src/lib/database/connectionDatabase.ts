import type { ConnectionConfig } from "@/types/database";

export function normalizeStoredConnectionDatabase(_dbType: ConnectionConfig["db_type"], database: string | undefined): string | undefined {
  return database;
}
