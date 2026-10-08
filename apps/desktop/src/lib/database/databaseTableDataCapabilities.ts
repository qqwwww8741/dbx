import type { DatabaseType } from "@/types/database";

export type SyntheticEditKey = "neo4j-element-id";

export interface TableDataCapability {
  insert: boolean;
  updateRequiresPrimaryKey: boolean;
  deleteRequiresPrimaryKey: boolean;
  keylessRowPredicate?: boolean;
  requiresTransactionalTableForExistingRows: boolean;
  existingRowsReadonly?: boolean;
  transaction: boolean;
  readonly?: boolean;
}

export interface DatabaseCapability {
  schemaAware: boolean;
  treeSchemaMode: boolean;
  tableData: TableDataCapability;
  syntheticKey?: SyntheticEditKey;
}

const DEFAULT_TABLE_DATA_CAPABILITY: TableDataCapability = {
  insert: false,
  updateRequiresPrimaryKey: true,
  deleteRequiresPrimaryKey: true,
  keylessRowPredicate: false,
  requiresTransactionalTableForExistingRows: false,
  transaction: true,
};

const NAVICAT_STYLE_TABLE_DATA_CAPABILITY: TableDataCapability = {
  insert: true,
  updateRequiresPrimaryKey: false,
  deleteRequiresPrimaryKey: false,
  keylessRowPredicate: true,
  requiresTransactionalTableForExistingRows: false,
  transaction: true,
};

const DEFAULT_CAPABILITY: DatabaseCapability = {
  schemaAware: false,
  treeSchemaMode: false,
  tableData: DEFAULT_TABLE_DATA_CAPABILITY,
};

const NAVICAT_STYLE_TABLE_DATA_TYPES = new Set<DatabaseType>(["mysql"]);

const DATABASE_CAPABILITY_OVERRIDES: Partial<Record<DatabaseType, Partial<DatabaseCapability>>> = {};

function defaultTableDataCapability(dbType?: DatabaseType): TableDataCapability {
  if (dbType && NAVICAT_STYLE_TABLE_DATA_TYPES.has(dbType)) return NAVICAT_STYLE_TABLE_DATA_CAPABILITY;
  return DEFAULT_TABLE_DATA_CAPABILITY;
}

export function getDatabaseCapability(dbType?: DatabaseType): DatabaseCapability {
  const override = dbType ? DATABASE_CAPABILITY_OVERRIDES[dbType] : undefined;
  const tableData = defaultTableDataCapability(dbType);
  return {
    ...DEFAULT_CAPABILITY,
    ...override,
    schemaAware: false,
    treeSchemaMode: false,
    tableData: {
      ...tableData,
      ...override?.tableData,
    },
  };
}
