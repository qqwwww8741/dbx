import type { DatabaseType, SqlServerColumnMetadata } from "@/types/database";

type InsertValueHintColumn = Pick<SqlServerColumnMetadata, "name"> & Partial<Pick<SqlServerColumnMetadata, "is_identity" | "is_computed" | "is_hidden" | "generated_always_type">>;

export function insertValueHintColumnNames(_databaseType: DatabaseType | undefined, columns: readonly InsertValueHintColumn[]): string[] {
  return columns
    .filter((_column) => {
      {
        return true;
      }
    })
    .map((column) => column.name);
}
