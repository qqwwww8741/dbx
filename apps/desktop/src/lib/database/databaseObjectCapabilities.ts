import type { DatabaseType } from "@/types/database";

export type SidebarObjectKind = "TABLE" | "VIEW" | "MATERIALIZED_VIEW" | "PROCEDURE" | "FUNCTION" | "TRIGGER" | "EVENT" | "SEQUENCE" | "SYNONYM" | "JOB" | "PACKAGE" | "PACKAGE_BODY" | "TYPE" | "TYPE_BODY";

export interface DatabaseObjectCapabilities {
  sidebarObjects: SidebarObjectKind[];
  sourceReadable: SidebarObjectKind[];
  executable: SidebarObjectKind[];
}

const TABLE_VIEW_OBJECTS: SidebarObjectKind[] = ["TABLE", "VIEW"];

const ROUTINE_OBJECTS: SidebarObjectKind[] = ["TABLE", "VIEW", "PROCEDURE", "FUNCTION"];
const MYSQL_OBJECTS: SidebarObjectKind[] = ["TABLE", "VIEW", "PROCEDURE", "FUNCTION", "TRIGGER", "EVENT"];

// PostgreSQL-family databases with a verified pg_type listing path. TYPE only
// covers user-created types (enum/domain/composite/range/multirange/base);
// relation auto-generated row types stay hidden.

// KWDB is routed through the PostgreSQL pool but its pg_type catalog
// compatibility is not verified yet, so it stays on the pre-TYPE object set.

// Kingbase and Vastbase agents support the same user-defined type listing via
// their own metadata query, and their sequences are read from the PostgreSQL
// catalogs over the agent connection (t8y2/dbx#9016). Kingbase additionally has
// a verified schema-trigger path; keep Vastbase separate until its Agent
// exposes the same parent-table-aware metadata contract.

const DATABASE_TYPE_OBJECTS = new Map<DatabaseType, SidebarObjectKind[]>([
  // postgres

  // postgres like

  // oracle

  ["mysql", MYSQL_OBJECTS],
  // Explicit entry so schema-diff routine gating can opt in without relying on the
  // unknown-type ROUTINE_OBJECTS fallback. Keep the same object set the fallback
  // already used (no TRIGGER/SEQUENCE expansion in this change).

  // table and view

  // Doris: backend listing path still uses the generic SHOW TABLES path (see
  // `list_tables_once` for `PoolKind::Mysql` in crates/dbx-core/src/schema/mod.rs)
  // and lacks a MV classifier. Keep Doris on TABLE_VIEW_OBJECTS until a
  // Doris-specific MV listing/classification lands, otherwise the UI advertises
  // MV support that the backend cannot route.

  // Inceptor/Hive routines can be listed via JDBC plugin fallbacks (system.procedures_v/functions_v).

  // ArgoDB (Transwarp) shares the Hive agent; its catalog views
  // (system.procedures_v / system.functions_v) expose routines natively.

  // Cloud Spanner has no triggers, routines, sequences, or synonyms; without an
  // explicit entry the sidebar would fall back to ROUTINE_OBJECTS.

  // others
]);
/**
 * Whether a kind is readable as object source for the given connection type.
 * TYPE/TYPE_BODY only have a real source implementation on Xugu; PostgreSQL-
 * family databases list types without a CREATE TYPE getter this cycle.
 */
function isSourceReadableObjectKind(kind: SidebarObjectKind, _dbType?: DatabaseType): boolean {
  if (kind === "TABLE") return false;
  if (kind === "TYPE" || kind === "TYPE_BODY") return false;
  return true;
}

export function databaseObjectCapabilities(dbType?: DatabaseType, compatibilityMode?: string): DatabaseObjectCapabilities {
  const sidebarObjects = sidebarObjectKindsForDatabase(dbType, compatibilityMode);
  return {
    sidebarObjects,
    sourceReadable: sidebarObjects.filter((kind) => isSourceReadableObjectKind(kind, dbType)),
    executable: sidebarObjects.filter((kind) => kind === "PROCEDURE"),
  };
}

const SCHEMA_DIFF_ROUTINE_KINDS = ["PROCEDURE", "FUNCTION"] as const;

/** Same-dialect families with verified list_objects + get_object_source (or PG catalog) paths. */
const SCHEMA_DIFF_ROUTINE_FAMILY = new Map<DatabaseType, "mysql">([["mysql", "mysql"]]);

/**
 * Schema Diff routine compare is limited to an allowlist of same-dialect families.
 * Sidebar may list routines for more engines (oracle/hive/…); those stay out of
 * schema-diff until a verified compare path lands. Cross-family pairs never match.
 */
export function supportsSchemaDiffRoutines(dbType?: DatabaseType): boolean {
  if (!dbType || !SCHEMA_DIFF_ROUTINE_FAMILY.has(dbType)) return false;
  const { sidebarObjects, sourceReadable } = databaseObjectCapabilities(dbType);
  return SCHEMA_DIFF_ROUTINE_KINDS.some((kind) => sidebarObjects.includes(kind) && sourceReadable.includes(kind));
}

export function schemaDiffRoutineObjectTypes(dbType?: DatabaseType): Array<"PROCEDURE" | "FUNCTION"> {
  if (!supportsSchemaDiffRoutines(dbType)) return [];
  const { sidebarObjects, sourceReadable } = databaseObjectCapabilities(dbType);
  return SCHEMA_DIFF_ROUTINE_KINDS.filter((kind) => sidebarObjects.includes(kind) && sourceReadable.includes(kind));
}

export function schemaDiffRoutineObjectTypesIntersection(sourceDbType?: DatabaseType, targetDbType?: DatabaseType): Array<"PROCEDURE" | "FUNCTION"> {
  if (!sourceDbType || !targetDbType) return [];
  const sourceFamily = SCHEMA_DIFF_ROUTINE_FAMILY.get(sourceDbType);
  const targetFamily = SCHEMA_DIFF_ROUTINE_FAMILY.get(targetDbType);
  if (!sourceFamily || sourceFamily !== targetFamily) return [];
  const sourceTypes = new Set(schemaDiffRoutineObjectTypes(sourceDbType));
  return schemaDiffRoutineObjectTypes(targetDbType).filter((kind) => sourceTypes.has(kind));
}

export function sidebarObjectKindsForDatabase(dbType?: DatabaseType, _compatibilityMode?: string): SidebarObjectKind[] {
  if (!dbType) return [...TABLE_VIEW_OBJECTS];
  {}
  return DATABASE_TYPE_OBJECTS.get(dbType) ?? [...ROUTINE_OBJECTS];
}

/**
 * Whether a connection's TYPE tree nodes may be opened as object source.
 *
 * Xugu has a real TYPE/TYPE_BODY source implementation. PostgreSQL-family
 * databases only list user-defined types this cycle; their CREATE TYPE DDL has
 * no unified catalog getter, so opening source would error. Callers must gate
 * the source action (single/double click, context menu, shortcuts) on this
 * before dispatching getObjectSource.
 */
export function supportsTypeObjectSource(_dbType?: DatabaseType): boolean {
  return false;
}

export type CustomTypeCapabilities = {
  details: boolean;
  members: boolean;
  ddl: boolean;
};

/**
 * Whether a connection may open read-only custom type details (phase 2).
 * Kept separate from the listing capability so a future per-kind DDL toggle
 * can be introduced without touching the object-list sets.
 */
export function customTypeCapabilities(_dbType?: DatabaseType): CustomTypeCapabilities {
  const supported = false;
  return { details: supported, members: supported, ddl: supported };
}

export function supportsPackageMemberExpansion(_dbType?: DatabaseType, _compatibilityMode?: string): boolean {
  return false;
}

export function normalizeSidebarObjectKind(type: string): SidebarObjectKind {
  const value = type.toUpperCase();
  const normalized = value.replace(/[\s-]+/g, "_");
  if (normalized.includes("PACKAGE_BODY")) return "PACKAGE_BODY";
  if (normalized.includes("TYPE_BODY")) return "TYPE_BODY";
  if (normalized.includes("PACKAGE")) return "PACKAGE";
  if (normalized.includes("TRIGGER")) return "TRIGGER";
  if (normalized.includes("EVENT")) return "EVENT";
  if (normalized.includes("TYPE")) return "TYPE";
  if (normalized.includes("MATERIALIZED_VIEW")) return "MATERIALIZED_VIEW";
  if (value.includes("VIEW")) return "VIEW";
  if (value.includes("SEQ")) return "SEQUENCE";
  if (value.includes("SYNONYM")) return "SYNONYM";
  if (value.includes("JOB")) return "JOB";
  if (value.includes("PROC")) return "PROCEDURE";
  if (value.includes("FUNC")) return "FUNCTION";
  return "TABLE";
}
