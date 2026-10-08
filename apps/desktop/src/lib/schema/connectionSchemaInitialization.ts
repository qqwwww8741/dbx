import type { DatabaseType } from "@/types/database";

export function schemaAfterConnectionSwitch(_databaseType: DatabaseType | undefined, _orderedSchemaNames: string[], configuredDefaultSchema?: string): string | undefined {
  if (configuredDefaultSchema?.trim()) return configuredDefaultSchema.trim();
  {
    return undefined;
  }
}
