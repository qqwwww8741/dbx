import type { DatabaseType, TableInfoTab } from "@/types/database";

export interface TableMetadataCapabilities {
  columns: boolean;
  indexes: boolean;
  foreignKeys: boolean;
  constraints: boolean;
  triggers: boolean;
  partitions: boolean;
  ddl: boolean;
}

const defaultCapabilities: TableMetadataCapabilities = {
  columns: true,
  indexes: true,
  foreignKeys: true,
  // Structured constraint metadata (list_constraints) is only enabled for
  // drivers that implement it (PostgreSQL natively; Oracle/Xugu via agents);
  // leave it off by default so every other dialect doesn't grow a permanently
  // empty tab.
  constraints: false,
  triggers: true,
  // Declarative partitioning metadata (pg_partitioned_table / pg_get_partkeydef)
  // is PostgreSQL-only for now; other dialects leave the tab hidden.
  partitions: false,
  ddl: true,
};

const capabilityByType: Partial<Record<DatabaseType, Partial<TableMetadataCapabilities>>> = {
  // KingbaseES V9 shares PostgreSQL's declarative partition catalog and DDL.
  // PostgreSQL reports full pg_constraint metadata (PK/FK/UNIQUE/CHECK/
  // EXCLUDE/NOT NULL) through list_constraints.
  // SQL Server reports PK/UNIQUE/FOREIGN KEY/CHECK/DEFAULT constraints from the
  // sys.* catalog views through list_constraints.
  // A Salesforce object only has describe metadata: the driver lists fields, and
  // there is no index, foreign key, trigger or DDL surface behind an SObject, so
  // those structure tabs would render permanently empty.
};

export function getTableMetadataCapabilities(dbType?: DatabaseType): TableMetadataCapabilities {
  return { ...defaultCapabilities, ...(dbType ? capabilityByType[dbType] : undefined) };
}

export function firstStructureMetadataTab(capabilities: TableMetadataCapabilities, isCreateMode: boolean): TableInfoTab {
  // Structure editing should open on an editable metadata page; DDL remains a
  // read-only fallback for databases that do not expose editable metadata.
  if (capabilities.columns) return "columns";
  if (capabilities.indexes) return "indexes";
  if (capabilities.foreignKeys) return "foreignKeys";
  if (capabilities.constraints) return "constraints";
  if (capabilities.triggers) return "triggers";
  if (!isCreateMode && capabilities.ddl) return "ddl";
  return "columns";
}

export function isStructureMetadataTabSupported(tab: TableInfoTab, capabilities: TableMetadataCapabilities, isCreateMode: boolean): boolean {
  return (
    (tab === "columns" && capabilities.columns) ||
    (tab === "indexes" && capabilities.indexes) ||
    (tab === "foreignKeys" && capabilities.foreignKeys) ||
    (tab === "constraints" && capabilities.constraints) ||
    (tab === "triggers" && capabilities.triggers) ||
    (tab === "partitions" && capabilities.partitions) ||
    (tab === "ddl" && capabilities.ddl && !isCreateMode)
  );
}
