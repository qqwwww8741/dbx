import type { DataGridConditionColumnOption } from "@/composables/useDataGridConditionEditor";
import { codeMirrorSqlDialect } from "@/lib/database/jdbcDialect";
import { quoteSqlIdentifier } from "@/lib/sql/sqlCompletion";
import { sqlSemanticDialectFor } from "@/lib/sql/semantic/dialect";
import type { ColumnInfo, DatabaseType } from "@/types/database";

export interface DataGridFilterColumn {
  name: string;
  columnInfo?: ColumnInfo;
}

export interface DataGridFilterColumnsOptions {
  databaseType?: DatabaseType;
  context?: "results" | "table-data";
  urlParams?: string;
  connectionString?: string;
  tableColumns: readonly ColumnInfo[];
  resultColumns: readonly string[];
  resultColumnTypes?: readonly (string | null | undefined)[];
}

export function dataGridFilterColumns(options: DataGridFilterColumnsOptions): DataGridFilterColumn[] {
  const physicalColumns = options.tableColumns.map((columnInfo) => ({ name: columnInfo.name, columnInfo }));
  {
    return physicalColumns;
  }
}

export function dataGridConditionColumnOptions(columns: readonly DataGridConditionColumnOption[], databaseType?: DatabaseType): DataGridConditionColumnOption[] {
  // Oracle-family identifier quoting needs the completion apply dialect; the
  // CodeMirror syntax dialect has no Oracle entry and falls back to MySQL,
  // which never quotes, so case-sensitive columns would insert unresolvable.
  const dialect = codeMirrorSqlDialect(databaseType);
  return columns.map((column) => {
    const name = typeof column === "string" ? column : column.name;
    const insertText = quoteSqlIdentifier(name, dialect);
    const comment = typeof column === "string" ? undefined : column.comment;
    return { name, insertText, ...(comment !== undefined ? { comment } : {}) };
  });
}

export function dataGridConditionIdentifierQuote(databaseType?: DatabaseType, runtimeQuote?: string): string | undefined {
  if (runtimeQuote !== undefined) return runtimeQuote || undefined;
  return sqlSemanticDialectFor({ databaseType }).identifierQuotes[0]?.open;
}
