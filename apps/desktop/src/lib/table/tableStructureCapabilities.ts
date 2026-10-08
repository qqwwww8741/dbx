import type { DatabaseType } from "@/types/database";
import type { EditableStructureIndex } from "@/lib/table/tableStructureEditorSql";

export type TableStructureDialect = "mysql" | "unsupported";
export type TableStructureAlterStrategy = "none" | "direct" | "sqlite-rebuild";

export interface TableStructureCapabilities {
  dialect: TableStructureDialect;
  alterStrategy: TableStructureAlterStrategy;
  createTable: boolean;
  addColumn: boolean;
  dropColumn: boolean;
  renameColumn: boolean;
  alterExistingColumn: boolean;
  alterType: boolean;
  alterNullability: boolean;
  alterDefault: boolean;
  addPrimaryKey: boolean;
  alterPrimaryKey: boolean;
  reorderColumn: boolean;
  comment: boolean;
  createIndex: boolean;
  dropIndex: boolean;
  rebuildIndex: boolean;
  indexType: boolean;
  indexInclude: boolean;
  indexFilter: boolean;
  indexComment: boolean;
  indexConcurrent: boolean;
  foreignKey: boolean;
}

const unsupportedCapabilities: TableStructureCapabilities = {
  dialect: "unsupported",
  alterStrategy: "none",
  createTable: false,
  addColumn: false,
  dropColumn: false,
  renameColumn: false,
  alterExistingColumn: false,
  alterType: false,
  alterNullability: false,
  alterDefault: false,
  addPrimaryKey: false,
  alterPrimaryKey: false,
  reorderColumn: false,
  comment: false,
  createIndex: false,
  dropIndex: false,
  rebuildIndex: false,
  indexType: false,
  indexInclude: false,
  indexFilter: false,
  indexComment: false,
  indexConcurrent: false,
  foreignKey: false,
};

function capabilities(overrides: Partial<TableStructureCapabilities>): TableStructureCapabilities {
  const resolved = { ...unsupportedCapabilities, ...overrides };
  if (overrides.alterStrategy === undefined && resolved.alterExistingColumn) {
    resolved.alterStrategy = "direct";
  }
  if (resolved.alterPrimaryKey) {
    resolved.addPrimaryKey = true;
  }
  return resolved;
}

const mysqlCapabilities = capabilities({
  dialect: "mysql",
  createTable: true,
  addColumn: true,
  dropColumn: true,
  renameColumn: true,
  alterExistingColumn: true,
  alterType: true,
  alterNullability: true,
  alterDefault: true,
  reorderColumn: true,
  comment: true,
  createIndex: true,
  dropIndex: true,
  rebuildIndex: true,
  indexType: true,
  indexComment: true,
  alterPrimaryKey: true,
  foreignKey: true,
});

// Dameng (DM8): ALTER TABLE ... DROP PRIMARY KEY / ADD PRIMARY KEY is official DDL.
// Keep separate from Oracle-compatible engines so UI cannot enable PK edits without verified DDL.

// Inceptor accepts ADD COLUMNS and CHANGE. DROP COLUMN and index/constraint DDL
// remain disabled until a safe server-supported form is verified.

const capabilityByType: Partial<Record<DatabaseType, TableStructureCapabilities>> = {
  mysql: mysqlCapabilities,
};

export function getTableStructureCapabilities(dbType?: DatabaseType, _connectionDbType?: DatabaseType, _productVersion?: string): TableStructureCapabilities {
  {}
  {}
  return dbType ? (capabilityByType[dbType] ?? unsupportedCapabilities) : unsupportedCapabilities;
}

export function sanitizeStructureIndexesForCapabilities(indexes: EditableStructureIndex[], capabilities: Pick<TableStructureCapabilities, "indexInclude">): EditableStructureIndex[] {
  if (capabilities.indexInclude || indexes.every((index) => index.includedColumns.length === 0)) return indexes;
  return indexes.map((index) => (index.includedColumns.length === 0 ? index : { ...index, includedColumns: [] }));
}

export function canEditTableStructure(dbType?: DatabaseType): boolean {
  const caps = getTableStructureCapabilities(dbType);
  return caps.createTable || caps.addColumn || caps.alterExistingColumn || caps.createIndex || caps.dropIndex;
}

export function supportsLocalTableColumnReorder(dbType?: DatabaseType, connectionDbType?: DatabaseType): boolean {
  const caps = getTableStructureCapabilities(dbType, connectionDbType);
  return canEditTableStructure(dbType) && !caps.reorderColumn;
}

export function isPhysicalTableColumnOrderChange(dbType: DatabaseType | undefined, connectionDbType: DatabaseType | undefined, originalPosition: number | undefined, currentPosition: number): boolean {
  return getTableStructureCapabilities(dbType, connectionDbType).reorderColumn && originalPosition !== currentPosition;
}

export function hasLocalTableColumnOrderChange(columns: readonly { originalPosition?: number; original?: unknown; markedForDrop?: boolean }[]): boolean {
  const activeColumns = columns.filter((column) => !column.markedForDrop);
  // Databases without physical reorder support keep existing columns in ordinal order
  // and append newly added columns, so compare against that post-save layout.
  const databaseOrder = [...activeColumns.filter((column) => column.original).sort((left, right) => (left.originalPosition ?? Number.MAX_SAFE_INTEGER) - (right.originalPosition ?? Number.MAX_SAFE_INTEGER)), ...activeColumns.filter((column) => !column.original)];
  return activeColumns.some((column, index) => column !== databaseOrder[index]);
}

export function canAddTableStructureColumn(dbType: DatabaseType | undefined, isCreateMode: boolean): boolean {
  const caps = getTableStructureCapabilities(dbType);
  return isCreateMode ? caps.createTable : caps.addColumn;
}
