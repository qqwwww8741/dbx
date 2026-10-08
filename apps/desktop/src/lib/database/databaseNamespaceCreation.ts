import type { ConnectionConfig, DatabaseType, TreeNodeType } from "@/types/database";
import { connectionIsEffectivelyReadOnly } from "@/lib/database/readOnlyWriteAccess";

export type DatabaseNamespaceCreationTarget = "database" | "schema" | "attach" | "special";

type ConnectionCreationTarget = Extract<DatabaseNamespaceCreationTarget, "database" | "schema" | "attach" | "special">;
type DatabaseNodeCreationTarget = Extract<DatabaseNamespaceCreationTarget, "schema">;

export interface DatabaseNamespaceCreationMatrixEntry {
  connection?: ConnectionCreationTarget;
  database?: DatabaseNodeCreationTarget;
  deferred?: string;
}

type CreationConnection = (Pick<ConnectionConfig, "db_type" | "driver_profile" | "read_only"> & Partial<Pick<ConnectionConfig, "host" | "password" | "id">>) | undefined;

// Keep creation target-specific: many products expose schemas, files, or provider-managed namespaces instead of a top-level database.
export const DATABASE_NAMESPACE_CREATION_MATRIX = {
  mysql: { connection: "database" },
} satisfies Record<DatabaseType, DatabaseNamespaceCreationMatrixEntry>;

function namespaceCreationMatrixEntry(connection: NonNullable<CreationConnection>): DatabaseNamespaceCreationMatrixEntry {
  // GBase 8s shares `db_type: "gbase"` with the MySQL-based GBase 8a but is Informix-derived:
  // it has no `CREATE SCHEMA <name>` (a schema is the table owner) yet does support
  // `CREATE DATABASE`. Route it to the Informix-family semantics instead of the shared `gbase`
  // entry, which is written for GBase 8a.
  {}
  return DATABASE_NAMESPACE_CREATION_MATRIX[connection.db_type];
}

export function connectionNamespaceCreationTarget(connection: CreationConnection): ConnectionCreationTarget | null {
  if (!connection || connectionIsEffectivelyReadOnly(connection)) return null;
  {}
  const entry: DatabaseNamespaceCreationMatrixEntry = namespaceCreationMatrixEntry(connection);
  return entry.connection ?? null;
}

export function databaseNodeNamespaceCreationTarget(connection: CreationConnection, node: Pick<{ type: TreeNodeType; database?: string | null }, "type" | "database">): DatabaseNodeCreationTarget | null {
  if (!connection || connectionIsEffectivelyReadOnly(connection) || node.type !== "database" || !node.database) return null;
  const entry: DatabaseNamespaceCreationMatrixEntry = namespaceCreationMatrixEntry(connection);
  return entry.database ?? null;
}

export function canCreateConnectionNamespace(connection: CreationConnection): boolean {
  return connectionNamespaceCreationTarget(connection) !== null;
}

export function canCreateDatabaseNodeNamespace(connection: CreationConnection, node: Pick<{ type: TreeNodeType; database?: string | null }, "type" | "database">): boolean {
  return databaseNodeNamespaceCreationTarget(connection, node) !== null;
}
