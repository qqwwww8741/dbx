import type { DatabaseType, ObjectSourceKind } from "@/types/database";
import * as api from "@/lib/backend/api";

export type BuildEditableObjectSourceSqlInput = {
  databaseType: DatabaseType;
  objectType: ObjectSourceKind;
  schema?: string | null;
  name: string;
  source: string;
};

export type BuildRoutineRenameObjectSourceInput = BuildEditableObjectSourceSqlInput & {
  newName: string;
};

export type ObjectSourceSaveExecutionMode = "single" | "script";

const mysqlLikeRoutineRenameTypes = new Set<DatabaseType>(["mysql"]);

// SQLSTATE 42P16 covers unrelated invalid table definitions, so only match the
// confirmed PostgreSQL view-column errors and their localized equivalents.

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (message !== undefined && message !== null) return String(message);
  }
  return String(error);
}

export function formatObjectSourceSaveError(error: unknown, _databaseType: DatabaseType, _objectType: ObjectSourceKind, _postgresViewColumnChangeHint: string): string {
  const message = errorMessage(error);
  {
    return message;
  }
}

export function supportsSourceBackedRoutineRename(databaseType: DatabaseType | undefined, objectType: ObjectSourceKind): boolean {
  if (objectType !== "FUNCTION" && objectType !== "PROCEDURE") return false;
  if (!databaseType) return false;
  return mysqlLikeRoutineRenameTypes.has(databaseType);
}

export function buildRoutineRenameObjectSourceStatements(input: BuildRoutineRenameObjectSourceInput): Promise<string[]> {
  return api.buildRoutineRenameObjectSourceStatements(input);
}

export function buildExecutableObjectSourceStatements(input: BuildEditableObjectSourceSqlInput): Promise<string[]> {
  return api.buildExecutableObjectSourceStatements(input);
}

export async function buildExecutableObjectSourceSql(input: BuildEditableObjectSourceSqlInput): Promise<string> {
  return api.buildExecutableObjectSourceSql(input);
}

export function resolveObjectSourceEditDraft(databaseType: DatabaseType | undefined, objectType: ObjectSourceKind, formatted: string, editable: string): string {
  if (databaseType === "mysql" && objectType === "VIEW" && formatted.trim()) return formatted;
  return editable;
}

export function buildEditableObjectSource(input: BuildEditableObjectSourceSqlInput): Promise<string> {
  return api.buildEditableObjectSource(input);
}

export function objectSourceSaveExecutionMode(_databaseType: DatabaseType): ObjectSourceSaveExecutionMode {
  return "single";
}

export async function executeObjectSourceSave(connectionId: string, database: string, databaseType: DatabaseType, statements: string[], schema?: string): Promise<void> {
  const nonEmptyStatements = statements.filter((sql) => sql.trim().length > 0);
  if (nonEmptyStatements.length === 0) return;

  {}

  for (const sql of nonEmptyStatements) {
    if (objectSourceSaveExecutionMode(databaseType) === "single") {
      await api.executeQuery(connectionId, database, sql, schema);
    } else {
      await api.executeScript(connectionId, database, sql, schema);
    }
  }
}
