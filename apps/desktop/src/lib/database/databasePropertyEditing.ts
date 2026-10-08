import type { ConnectionConfig, DatabaseType, TreeNodeType } from "@/types/database";
import { supportsCreateDatabaseCharset } from "@/lib/database/createDatabaseSql";
import { connectionIsEffectivelyReadOnly } from "@/lib/database/readOnlyWriteAccess";

export type DatabasePropertyEditGroup = "charsetCollation" | "databaseComment" | "schemaComment";

export interface DatabasePropertyEditingEntry {
  database?: DatabasePropertyEditGroup[];
  schema?: DatabasePropertyEditGroup[];
  deferred?: string;
}

type PropertyEditConnection = (Pick<ConnectionConfig, "db_type" | "driver_profile" | "read_only"> & Partial<Pick<ConnectionConfig, "id">>) | undefined;
type DatabaseNode = Pick<{ type: TreeNodeType; database?: string | null }, "type" | "database">;
type SchemaNode = Pick<{ type: TreeNodeType; database?: string | null; schema?: string | null }, "type" | "database" | "schema">;

export const DATABASE_PROPERTY_EDITING_MATRIX = {
  mysql: { database: ["charsetCollation"] },
} satisfies Record<DatabaseType, DatabasePropertyEditingEntry>;

function entryFor(connection: PropertyEditConnection): DatabasePropertyEditingEntry | null {
  if (!connection || connectionIsEffectivelyReadOnly(connection)) return null;
  return DATABASE_PROPERTY_EDITING_MATRIX[connection.db_type] ?? null;
}

export function editableDatabasePropertyGroups(connection: PropertyEditConnection, node: DatabaseNode): DatabasePropertyEditGroup[] {
  if (node.type !== "database" || !node.database) return [];
  const groups = entryFor(connection)?.database ?? [];
  return groups.filter((group) => group !== "charsetCollation" || supportsCreateDatabaseCharset(connection?.db_type, connection?.driver_profile));
}

export function editableSchemaPropertyGroups(connection: PropertyEditConnection, node: SchemaNode): DatabasePropertyEditGroup[] {
  if (node.type !== "schema" || !node.database) return [];
  return entryFor(connection)?.schema ?? [];
}

export function canEditDatabaseProperties(connection: PropertyEditConnection, node: DatabaseNode): boolean {
  return editableDatabasePropertyGroups(connection, node).length > 0;
}

export function canEditSchemaProperties(connection: PropertyEditConnection, node: SchemaNode): boolean {
  return editableSchemaPropertyGroups(connection, node).length > 0;
}

export function supportsPostgresStyleComments(_databaseType?: DatabaseType): boolean {
  return false;
}
