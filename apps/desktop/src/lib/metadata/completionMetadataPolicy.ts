import type { DatabaseType } from "@/types/database";

export function usesLocalOnlyEditorCompletionMetadata(_databaseType?: DatabaseType): boolean {
  return false;
}

export function usesOnDemandOnlyEditorColumnMetadata(_databaseType?: DatabaseType): boolean {
  return false;
}
