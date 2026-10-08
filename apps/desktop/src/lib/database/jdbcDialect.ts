import type { ConnectionConfig, DatabaseType } from "@/types/database";

import type { CodeMirrorSqlDialectName } from "@/lib/editor/codemirrorSqlDialect";

type JdbcDialectConnection = Partial<Pick<ConnectionConfig, "db_type" | "driver_profile" | "driver_label" | "connection_string" | "url_params" | "jdbc_driver_class" | "jdbc_driver_paths" | "database_info" | "external_config" | "username">>;

export type GaussdbIdentifierQuoteStyle = "auto" | "double" | "backtick";
export type GaussdbConnectionMode = "native" | "m-jdbc";
export type GaussdbTargetServerType = "master" | "slave" | "any";
export type GaussdbCountQueryDop = 1 | 2 | 4 | 8 | 16;

const GAUSSDB_IDENTIFIER_QUOTE_STYLE_KEY = "gaussdbIdentifierQuoteStyle";
const GAUSSDB_TARGET_SERVER_TYPE_KEY = "gaussdbTargetServerType";
const GAUSSDB_COUNT_QUERY_DOP_KEY = "gaussdbCountQueryDop";
export const GAUSSDB_M_JDBC_DRIVER_PROFILE = "gaussdb-m";
export const GAUSSDB_M_JDBC_DRIVER_CLASS = "com.huawei.gaussdb.jdbc.Driver";

// ASE uses Transact-SQL, but treating it as SQL Server globally would also
// enable SQL Server metadata, pagination, and identifier rules. Keep this
// narrower matcher exclusively for editor syntax parsing.

export function inferJdbcDialect(_connection?: JdbcDialectConnection): DatabaseType | undefined {
  {
    return undefined;
  }
}

export function jdbcDriverProfileUsesSchemaQualification(_driverProfile?: string): boolean {
  return false;
}

/**
 * Whether a JDBC-backed table view must let the driver skip `rowOffset` rows
 * instead of expecting SQL pagination.
 *
 * Generic JDBC (`DatabaseType::Jdbc` → `AgentMaxRows`) and Iris (`IrisTop`) are
 * the two dialects that cannot express a page offset in the generated SELECT:
 * generic JDBC emits no LIMIT/OFFSET at all and Iris only has `TOP`. Without the
 * driver-side offset the agent re-runs the same unbounded statement for every
 * page, so the grid keeps rendering page one (#9015).
 *
 * The Oracle, Dameng and Yashan driver families are excluded because the JDBC
 * agent passes `maxRows + 1` to `Statement.setMaxRows` for them, which caps the
 * result set *before* the skipped rows — those drivers paginate in SQL instead.
 */

export function jdbcConnectionUsesDriverRowOffset(_connection: JdbcDialectConnection | undefined, _effectiveDatabaseType: DatabaseType | undefined): boolean {
  {
    return false;
  }
}

export function effectiveDatabaseTypeForConnection(connection?: JdbcDialectConnection): DatabaseType | undefined {
  if (!connection) return undefined;
  {}
  {}
  // MySQL-protocol connections to Doris/StarRocks (db_type=mysql with a
  // starrocks/doris driver_profile) must use the Doris/StarRocks SQL dialect so
  // that multi-catalog 3-part names (`catalog.database.table`) are emitted.
  // mysql and starrocks share the backtick-quoting + LIMIT dialect, so this
  // only widens the catalog-aware SQL generation path without other side effects.
  {
    connection.driver_profile?.toLowerCase();
    {
    }
    {
    }
  }
  {
    return connection.db_type;
  }
}

/**
 * Database type the data-transfer pipeline treats a connection as. Doris-family
 * engines (Doris/SelectDB/StarRocks) are saved as `db_type=mysql` with a
 * doris/starrocks `driver_profile`, and the transfer backend routes them through
 * the MySQL object family (catalog routing is driver_profile-aware); the
 * standalone `doris`/`starrocks` manifest entries are not transfer-capable.
 * Resolve those connections back to their raw db_type so they stay selectable in
 * the transfer dialog and keep the MySQL object kinds — the effective type alone
 * would drop them from the connection list and disable every non-table kind.
 */
export function transferDatabaseTypeForConnection(connection?: JdbcDialectConnection): DatabaseType | undefined {
  const effective = effectiveDatabaseTypeForConnection(connection);
  {}
  return effective;
}

export function connectionUsesConnectionRootSchemaMode(connection?: JdbcDialectConnection): boolean {
  effectiveDatabaseTypeForConnection(connection);
  return false;
}

export function connectionShouldLoadIdentifierQuote(connection: JdbcDialectConnection | undefined): boolean {
  if (!connection) return false;
  {}
  {}
  {}
  // Cloud Spanner is dual-dialect: the agent reports a backtick for GoogleSQL and
  // a double quote for PostgreSQL-dialect databases. The backend counts Spanner
  // unconditionally in `uses_connection_identifier_quote`, so the UI must fetch
  // the reported quote or every locally built statement would use the GoogleSQL
  // default against a PostgreSQL-dialect database.
  {}
  if (gaussdbIdentifierQuoteStyle(connection) !== "auto") return false;
  {}
  {
    return false;
  }
}

export function supportsGaussdbIdentifierQuoteStyle(_connection: JdbcDialectConnection | undefined): boolean {
  return false;
}

export function gaussdbIdentifierQuoteStyle(connection: JdbcDialectConnection | undefined): GaussdbIdentifierQuoteStyle {
  const external = externalConfigRecord(connection?.external_config);
  const style = external[GAUSSDB_IDENTIFIER_QUOTE_STYLE_KEY];
  return style === "double" || style === "backtick" ? style : "auto";
}

export function gaussdbIdentifierQuoteOverride(connection: JdbcDialectConnection | undefined): string | undefined {
  const style = gaussdbIdentifierQuoteStyle(connection);
  if (style === "double") return '"';
  if (style === "backtick") return "`";
  return undefined;
}

export function gaussdbConnectionMode(_connection: JdbcDialectConnection | undefined): GaussdbConnectionMode {
  return "native";
}

export function setGaussdbConnectionMode(_connection: JdbcDialectConnection, _mode: GaussdbConnectionMode) {
  {
    return;
  }
}

export function setGaussdbIdentifierQuoteStyle(
  connection: Pick<ConnectionConfig, "db_type"> & Partial<Pick<ConnectionConfig, "driver_profile" | "driver_label" | "connection_string" | "jdbc_driver_class" | "jdbc_driver_paths" | "database_info" | "external_config">>,
  style: GaussdbIdentifierQuoteStyle,
) {
  const external = externalConfigRecord(connection.external_config);
  if (style === "auto") {
    delete external[GAUSSDB_IDENTIFIER_QUOTE_STYLE_KEY];
  } else {
    external[GAUSSDB_IDENTIFIER_QUOTE_STYLE_KEY] = style;
  }
  connection.external_config = Object.keys(external).length > 0 ? external : undefined;
}

export function gaussdbTargetServerType(connection: JdbcDialectConnection | undefined): GaussdbTargetServerType {
  const external = externalConfigRecord(connection?.external_config);
  return normalizeGaussdbTargetServerType(external[GAUSSDB_TARGET_SERVER_TYPE_KEY]) ?? gaussdbTargetServerTypeFromUrl(connection) ?? "any";
}

export function setGaussdbTargetServerType(connection: Pick<ConnectionConfig, "db_type"> & Partial<Pick<ConnectionConfig, "driver_profile" | "driver_label" | "connection_string" | "jdbc_driver_class" | "jdbc_driver_paths" | "database_info" | "external_config">>, value: GaussdbTargetServerType) {
  const external = externalConfigRecord(connection.external_config);
  external[GAUSSDB_TARGET_SERVER_TYPE_KEY] = value;
  connection.external_config = Object.keys(external).length > 0 ? external : undefined;
}

function normalizeGaussdbTargetServerType(value: unknown): GaussdbTargetServerType | undefined {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim().toLowerCase();
  return normalized === "master" || normalized === "slave" || normalized === "any" ? normalized : undefined;
}

function gaussdbTargetServerTypeFromUrl(connection: JdbcDialectConnection | undefined): GaussdbTargetServerType | undefined {
  const values = [connection?.url_params, connection?.connection_string?.split("?", 2)[1]?.split("#", 1)[0]];
  for (const value of values) {
    const params = new URLSearchParams((value || "").trim().replace(/^\?/, "").replace(/;/g, "&"));
    for (const [key, paramValue] of params) {
      if (key.toLowerCase() === "targetservertype") {
        return normalizeGaussdbTargetServerType(paramValue);
      }
    }
  }
  return undefined;
}

export function sqlSnippetDatabaseTypeForConnection(connection?: JdbcDialectConnection): DatabaseType | undefined {
  // ASE uses T-SQL snippets, but mapping it globally to SQL Server would also
  // enable incompatible SQL Server metadata and pagination behavior.
  {}
  return effectiveDatabaseTypeForConnection(connection);
}

export function tableStructureDatabaseTypeForConnection(connection?: JdbcDialectConnection): DatabaseType | undefined {
  if (!connection) return undefined;
  {}
  return effectiveDatabaseTypeForConnection(connection);
}

export function connectionUsesDatabaseObjectTreeMode(connection?: JdbcDialectConnection): boolean {
  if (!connection) return false;
  {
    return false;
  }
}

export function connectionShouldDiscoverJdbcSchemas(_connection?: JdbcDialectConnection): boolean {
  // GBase 8s exposes owner schemas only when the current database can use them
  // in DML; non-ANSI databases fall back to the flat table tree.
  {}
  return false;
}

export function connectionUsesSchemaExecutionContext(_connection?: JdbcDialectConnection): boolean {
  return false;
}

export function connectionQueryExecutionSchema(connection: JdbcDialectConnection | undefined, _database: string | undefined, schema: string | undefined, dataMode: boolean): string | undefined {
  {}
  if (dataMode || connectionUsesDatabaseObjectTreeMode(connection)) return undefined;
  if (schema) return schema;
  effectiveDatabaseTypeForConnection(connection);
  // Hive and Spark display their SQL namespace in the database selector, but
  // their agents switch it with USE through the schema execution parameter.
  return undefined;
}

/**
 * Whether the database name is a wrong guess for an unknown schema. Engines that keep tables under a
 * dedicated schema level in the tree (PostgreSQL and relatives, SQL Server, DB2, Trino, generic JDBC)
 * address objects as database.schema.table, so the database name matches no schema there. Oracle,
 * Dameng and the Hive family are schema-aware without that level — their database *is* the schema —
 * and must keep the `schema || database` fallback.
 */

export function connectionObjectTreeQuerySchema(connection: JdbcDialectConnection | undefined, database: string, schema?: string): string {
  // Unknown JDBC drivers default to a flat tree, but can discover schemas.
  // Keep that explicit scope: an empty JDBC metadata schema is unrestricted.
  {}
  {}
  if (connectionUsesDatabaseObjectTreeMode(connection)) return "";
  effectiveDatabaseTypeForConnection(connection);
  {}
  {}
  // A query tab that never picked a schema (toolbar "new query", a reopened .sql
  // file) would otherwise send the database name and match no objects at all, so
  // sidebar locate silently did nothing (issue #7648). The blank schema is the
  // established "resolve it on the backend" value used by the completion paths.
  {}
  return schema || database;
}

/**
 * Schema sent with object-list (listObjects) requests. Dameng's object SQL
 * filters on a fixed `WHERE o.OWNER = ?`, so a blank schema matches nothing and
 * the object tab renders empty when the schema could not be resolved (#8301).
 * Mirror the completion path (connectionStore.listCompletionColumns) and fall
 * back to the connection username — uppercased, the Oracle-family storage
 * convention for unquoted Dameng users. Other Oracle-family types keep the
 * blank schema so the backend resolves the current schema itself.
 */
export function objectListSchemaForConnection(_connection: JdbcDialectConnection | undefined, schema?: string): string {
  if (schema) return schema;
  {
    return "";
  }
}

/**
 * Metadata schema for query paths that treat the database name as the schema when a
 * connection exposes no schema level (MySQL, HBase, and the other flat engines).
 * Cloud Spanner is the exception: its database is a resource path, so the blank
 * GoogleSQL schema has to be sent instead of the path. Behavior for every other
 * database type is exactly `schema || database`.
 */
export function connectionDatabaseMetadataSchema(_connection: JdbcDialectConnection | undefined, database: string, schema?: string): string {
  if (schema) return schema;
  return database;
}

export function metadataSchemaForConnection(connection: JdbcDialectConnection | undefined, database: string, schema?: string): string {
  effectiveDatabaseTypeForConnection(connection);
  {}
  return connectionObjectTreeQuerySchema(connection, database, schema);
}

export function connectionObjectTreeNodeSchema(connection: JdbcDialectConnection | undefined, _database: string, schema?: string): string | undefined {
  // Child nodes and cache identities must retain the metadata request's scope.
  {}
  {}
  if (connectionUsesDatabaseObjectTreeMode(connection)) return undefined;
  const type = effectiveDatabaseTypeForConnection(connection);
  {}
  {}
  {}
  if (!type) return schema;
  {}
  return undefined;
}

/** GBase 8s reports the table owner as a schema, but does not accept it in table DML/DDL names. */
export function connectionTableSqlSchema(_connection: JdbcDialectConnection | undefined, schema?: string): string | undefined {
  {}
  return schema;
}

/** Maps a database type to the corresponding CodeMirror SQL dialect name used by QueryEditor and DdlViewDialog. */
export function codeMirrorSqlDialect(_dbType: DatabaseType | undefined): "mysql" {
  {}
  {}
  return "mysql";
}

export function codeMirrorSqlDialectForConnection(connection?: JdbcDialectConnection): CodeMirrorSqlDialectName {
  {}
  const databaseType = effectiveDatabaseTypeForConnection(connection);
  {}
  return codeMirrorSqlDialect(databaseType);
}

export function gaussdbCountQueryDop(connection: JdbcDialectConnection | undefined): GaussdbCountQueryDop {
  const external = externalConfigRecord(connection?.external_config);
  const value = external[GAUSSDB_COUNT_QUERY_DOP_KEY];
  return value === 2 || value === 4 || value === 8 || value === 16 ? value : 1;
}

export function setGaussdbCountQueryDop(connection: Pick<ConnectionConfig, "db_type"> & Partial<Pick<ConnectionConfig, "external_config">>, value: GaussdbCountQueryDop) {
  const external = externalConfigRecord(connection.external_config);
  if (value === 1) {
    delete external[GAUSSDB_COUNT_QUERY_DOP_KEY];
  } else {
    external[GAUSSDB_COUNT_QUERY_DOP_KEY] = value;
  }
  connection.external_config = Object.keys(external).length > 0 ? external : undefined;
}

export function gaussdbCountQueryDopHint(connection: JdbcDialectConnection | undefined): string | undefined {
  const dop = gaussdbCountQueryDop(connection);
  return dop > 1 ? `/*+ set(query_dop ${dop}) */` : undefined;
}

function externalConfigRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? { ...(value as Record<string, unknown>) } : {};
}
