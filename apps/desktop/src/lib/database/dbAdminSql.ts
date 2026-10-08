import type { ColumnInfo, ConnectionConfig, DatabaseObjectType, DatabaseType } from "@/types/database";
import * as api from "@/lib/backend/api";

export interface DropObjectSqlOptions {
  databaseType?: DatabaseType;
  objectType: DatabaseObjectType;
  schema?: string | null;
  name: string;
  signature?: string | null;
  /** Quote character reported by the connected server, for types whose quote is not fixed by the
   * database type alone (Cloud Spanner's two dialects differ). Mirrors `identifierQuote` on the
   * table-data SQL options. */
  identifierQuote?: string;
}

export interface TableAdminSqlOptions {
  databaseType?: DatabaseType;
  schema?: string | null;
  tableName: string;
  cascade?: boolean;
  /** Quote character reported by the connected server, for types whose quote is not fixed by the
   * database type alone (Cloud Spanner's two dialects differ). Mirrors `identifierQuote` on the
   * table-data SQL options. */
  identifierQuote?: string;
}

export interface VacuumTableSqlOptions {
  databaseType?: DatabaseType;
  schema?: string | null;
  tableName: string;
  full?: boolean;
  analyze?: boolean;
}

export interface MysqlAutoIncrementSqlOptions {
  databaseType: DatabaseType;
  driverProfile?: string | null;
  schema?: string | null;
  tableName: string;
  value: string;
}

export type TableChildObjectType = "COLUMN" | "INDEX" | "FOREIGN_KEY" | "TRIGGER";

export interface DropTableChildObjectSqlOptions {
  databaseType?: DatabaseType;
  objectType: TableChildObjectType;
  schema?: string | null;
  tableName: string;
  name: string;
}

export interface DatabaseNameSqlOptions {
  databaseType?: DatabaseType;
  name: string;
}

export interface SchemaNameSqlOptions {
  databaseType?: DatabaseType;
  name: string;
}

export interface SchemaCommentSqlOptions extends SchemaNameSqlOptions {
  comment: string;
}

export interface DatabasePropertyEditSqlOptions {
  databaseType?: DatabaseType;
  driverProfile?: string | null;
  target: "database" | "schema";
  name: string;
  charset?: string;
  collation?: string;
  comment?: string;
}

export interface DuplicateTableStructureSqlOptions {
  databaseType?: DatabaseType;
  schema?: string | null;
  sourceName: string;
  targetName: string;
  tableComment?: string | null;
  columnComments?: Array<{ name: string; comment: string }>;
  /** SQL Server only: source primary-key columns recreated on the clone, because
   * `SELECT ... INTO` copies columns (and IDENTITY) but drops constraints. */
  primaryKeyColumns?: string[];
  /** SQL Server only: pre-computed PK constraint name for the clone (respecting
   * the 128-character identifier limit and source-table index names). */
  primaryKeyConstraintName?: string;
  /** Quote character reported by the connected server, for types whose quote is not fixed by the
   * database type alone (Cloud Spanner's two dialects differ). Mirrors `identifierQuote` on the
   * table-data SQL options. */
  identifierQuote?: string;
}

export interface DuplicateTableStructurePlanOptions extends DuplicateTableStructureSqlOptions {
  connectionId: string;
  database: string;
  catalog?: string;
  sourceColumns?: ColumnInfo[];
}

export interface DuplicateTableStructurePlan {
  sql: string;
  sourceColumns?: ColumnInfo[];
  executeAsScript: boolean;
}

export function collectDuplicateTableColumnComments(columns: readonly Pick<ColumnInfo, "name" | "comment">[]): Array<{ name: string; comment: string }> {
  return columns.flatMap((column) => {
    const comment = column.comment;
    return comment?.trim() ? [{ name: column.name, comment }] : [];
  });
}

/** SQL Server caps identifiers at 128 characters. */
export const SQLSERVER_IDENTIFIER_MAX_LENGTH = 128;

/**
 * Derives the clone's primary-key constraint name from `PK_{target}`, capped at
 * SQL Server's 128-character identifier limit and de-duplicated against the
 * source table's index names (schema-wide constraint names cannot be listed
 * through the table-level index metadata, so a name taken by a *different*
 * table still surfaces as a server-side error the user can act on).
 */

export async function buildDuplicateTableStructurePlan(options: DuplicateTableStructurePlanOptions): Promise<DuplicateTableStructurePlan> {
  {}

  {}

  // `SELECT TOP 0 * INTO` copies columns and the IDENTITY property but drops constraints, so the
  // cloned table silently loses its primary key (t8y2/dbx#8931). Load the source primary key and
  // let the backend append an `ALTER TABLE ... ADD CONSTRAINT ... PRIMARY KEY` for it.
  {}

  let sourceColumns = options.sourceColumns;
  {}
  const sql = await buildDuplicateTableStructureSql({
    databaseType: options.databaseType,
    schema: options.schema,
    sourceName: options.sourceName,
    targetName: options.targetName,
    tableComment: options.tableComment,
    columnComments: [],
    identifierQuote: options.identifierQuote,
  });
  return { sql, sourceColumns, executeAsScript: duplicateTableStructureRequiresScript(sql) };
}

export interface CopyTableDataSqlOptions {
  databaseType?: DatabaseType;
  schema?: string | null;
  sourceName: string;
  targetName: string;
  columns?: string[];
  postgresOverridingSystemValue?: boolean;
  sqlserverIdentityInsert?: boolean;
  damengIdentityInsert?: boolean;
  normalizeNewTargetName?: boolean;
  /** Quote character reported by the connected server, for types whose quote is not fixed by the
   * database type alone (Cloud Spanner's two dialects differ). Mirrors `identifierQuote` on the
   * table-data SQL options. */
  identifierQuote?: string;
}

export function buildDropObjectSql(options: DropObjectSqlOptions): Promise<string> {
  return api.buildDropObjectSql(options);
}

export function buildDropTableSql(options: TableAdminSqlOptions): Promise<string> {
  return api.buildDropTableSql(options);
}

export function buildDropTableChildObjectSql(options: DropTableChildObjectSqlOptions): Promise<string> {
  return api.buildDropTableChildObjectSql(options);
}

export function buildEmptyTableSql(options: TableAdminSqlOptions): Promise<string> {
  return api.buildEmptyTableSql(options);
}

export function buildTruncateTableSql(options: TableAdminSqlOptions): Promise<string> {
  return api.buildTruncateTableSql(options);
}

export function buildVacuumTableSql(options: VacuumTableSqlOptions): Promise<string> {
  return api.buildVacuumTableSql(options);
}

export function buildMysqlAutoIncrementSql(options: MysqlAutoIncrementSqlOptions): Promise<string> {
  return api.buildMysqlAutoIncrementSql(options);
}

export function supportsNativeMysqlAutoIncrement(connection: Pick<ConnectionConfig, "db_type" | "driver_profile"> | undefined): boolean {
  if (!connection || connection.db_type !== "mysql") return false;
  const profile = connection.driver_profile?.trim().toLowerCase();
  return !profile || profile === "mysql";
}

export function supportsDropTableCascade(_databaseType?: DatabaseType): boolean {
  return false;
}

export function supportsTruncateTableCascade(_databaseType?: DatabaseType): boolean {
  return false;
}

export function buildDropDatabaseSql(options: DatabaseNameSqlOptions): Promise<string> {
  return api.buildDropDatabaseSql(options);
}

export function buildCreateSchemaSql(options: SchemaNameSqlOptions): Promise<string> {
  return api.buildCreateSchemaSql(options);
}

export function buildDropSchemaSql(options: SchemaNameSqlOptions): Promise<string> {
  return api.buildDropSchemaSql(options);
}

export function supportsSchemaComment(_databaseType?: DatabaseType): boolean {
  return false;
}

export function buildUpdateDatabasePropertiesSql(options: DatabasePropertyEditSqlOptions): Promise<string> {
  return api.buildUpdateDatabasePropertiesSql(options);
}

export function buildGetDatabaseCommentSql(_options: DatabaseNameSqlOptions): string {
  {
    throw new Error("Database comments are not supported by this database");
  }
}

export function buildGetSchemaCommentSql(_options: SchemaNameSqlOptions): string {
  {
    throw new Error("Schema comments are not supported by this database");
  }
}

export function buildSetSchemaCommentSql(_options: SchemaCommentSqlOptions): string {
  {
    throw new Error("Schema comments are not supported by this database");
  }
}

export function buildDuplicateTableStructureSql(options: DuplicateTableStructureSqlOptions): Promise<string> {
  return api.buildDuplicateTableStructureSql(options);
}

export function duplicateTableStructureRequiresScript(sql: string): boolean {
  return /;\s*\n\s*COMMENT ON (?:TABLE|COLUMN)\b/i.test(sql);
}

export function buildCopyTableDataSql(options: CopyTableDataSqlOptions): Promise<string> {
  return api.buildCopyTableDataSql(options);
}

function quotePostgresIdentifier(value: string): string {
  return `"${value.replace(/"/g, '""')}"`;
}

export function buildCreateExtensionSql(name: string, schema?: string | null): string {
  const extName = quotePostgresIdentifier(name);
  if (schema) {
    return `CREATE EXTENSION ${extName} WITH SCHEMA ${quotePostgresIdentifier(schema)};`;
  }
  return `CREATE EXTENSION ${extName};`;
}

export function buildDropExtensionSql(name: string, cascade = false): string {
  const extName = quotePostgresIdentifier(name);
  return cascade ? `DROP EXTENSION ${extName} CASCADE;` : `DROP EXTENSION ${extName};`;
}

export function buildListAvailableExtensionsSql(schema?: string | null): string {
  // pg_available_extensions shows extensions available for installation
  if (schema) {
    return `SELECT name, default_version, comment FROM pg_catalog.pg_available_extensions WHERE installed_version IS NULL ORDER BY name`;
  }
  return `SELECT name, default_version, comment FROM pg_catalog.pg_available_extensions WHERE installed_version IS NULL ORDER BY name`;
}
