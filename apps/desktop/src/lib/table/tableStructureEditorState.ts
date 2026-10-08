import type { ColumnInfo, DatabaseConnectionInfo, DatabaseType, ForeignKeyInfo, IndexInfo, TriggerInfo } from "@/types/database.ts";
import type { ColumnExtra, EditableStructureColumn, EditableStructureForeignKey, EditableStructureIndex, EditableStructureTrigger } from "@/lib/table/tableStructureEditorSql.ts";

export interface CopySourceColumnDetails {
  /** Column default as shown in the copy-fields dialog, or null when there is none. */
  defaultValue: string | null;
  /** Column comment as shown in the copy-fields dialog, or null when there is none. */
  comment: string | null;
}

/**
 * Read-only summary rendered under a column name in the "copy fields from another
 * table" dialog. The source table is not open while copying, so the comment and
 * default value are the only hint about what an unfamiliar field means.
 */
export function copySourceColumnDetails(column: Pick<ColumnInfo, "column_default" | "comment" | "data_type">, databaseType?: DatabaseType): CopySourceColumnDetails {
  // Match the main grid and the editor drafts so the dialog, the grid, and the
  // copied result render the same normalized default for every database.
  const defaultValue = column.column_default == null ? "" : columnDefaultForEditor(column, databaseType);
  const rawComment = column.comment ?? "";
  return {
    defaultValue: defaultValue.trim() ? defaultValue.trim() : null,
    comment: rawComment.trim() ? rawComment.trim() : null,
  };
}

/** Copy-dialog search matches comments and default values on top of name and type. */
export function matchesCopySourceColumnSearch(column: Pick<ColumnInfo, "name" | "data_type" | "column_default" | "comment">, search: string, databaseType?: DatabaseType): boolean {
  const query = search.trim().toLowerCase();
  if (!query) return true;
  const details = copySourceColumnDetails(column, databaseType);
  return [column.name, column.data_type, details.defaultValue ?? "", details.comment ?? ""].some((value) => value.toLowerCase().includes(query));
}

/**
 * Column names offered by the structure editor's "copy all column names" action.
 * Fields marked for drop disappear on save, so they are not offered.
 */
export function structureColumnNamesForCopy(columns: readonly Pick<EditableStructureColumn, "name" | "markedForDrop">[]): string[] {
  return columns.filter((column) => !column.markedForDrop && column.name.trim()).map((column) => column.name.trim());
}

/** Comment lookup for the same action, keyed by the trimmed column name. */
export function structureColumnCommentsForCopy(columns: readonly Pick<EditableStructureColumn, "name" | "comment" | "markedForDrop">[]): Map<string, string> {
  const comments = new Map<string, string>();
  for (const column of columns) {
    if (column.markedForDrop) continue;
    const name = column.name.trim();
    const comment = column.comment?.trim();
    if (name && comment) comments.set(name, comment);
  }
  return comments;
}

/**
 * Column name handed to the DDL builder for a draft column.
 *
 * MySQL rejects identifiers that end with a space (ERROR 1166 "Incorrect column
 * name"), so a pasted name carrying a stray trailing space produced an
 * unexecutable `ALTER TABLE ... CHANGE COLUMN ...` statement that the editor
 * still previewed as executable. Only the trailing whitespace is dropped:
 * leading spaces are legal in a backtick-quoted identifier and are kept, both
 * for a name the user typed (`#9654`) and for a metadata name, which is passed
 * through byte-exact so that an unrelated edit never turns into a bogus rename.
 */
export function draftColumnNameForSql(name: string, originalName?: string | null): string {
  return originalName === name ? name : name.trimEnd();
}

export function hasExistingColumnTypeChange(columns: readonly EditableStructureColumn[]): boolean {
  return columns.some((column) => !!column.original && !column.markedForDrop && column.dataType !== column.original.data_type);
}

export function resolveColumnSelectionActiveId(columns: readonly Pick<EditableStructureColumn, "id" | "markedForDrop">[], selectedIds: ReadonlySet<string>, preferredId: string): string | null {
  if (selectedIds.has(preferredId)) return preferredId;
  for (let index = columns.length - 1; index >= 0; index -= 1) {
    const column = columns[index];
    if (column && !column.markedForDrop && selectedIds.has(column.id)) return column.id;
  }
  return null;
}

export function isSyntheticContextMenuClick(contextMenuButton: number | null, contextMenuCtrlKey: boolean, clickButton: number): boolean {
  return contextMenuButton === 2 && contextMenuCtrlKey && clickButton === 0;
}

type TableStructureIdentifierCaseInfo = Pick<DatabaseConnectionInfo, "unquotedIdentifierCase" | "quotedIdentifierCase">;

function defaultTableStructureIdentifierCaseInfo(_databaseType?: DatabaseType): Required<TableStructureIdentifierCaseInfo> {
  {}
  {}
  {}
  return { unquotedIdentifierCase: "lower", quotedIdentifierCase: "lower" };
}

function applyIdentifierCase(value: string, identifierCase: NonNullable<DatabaseConnectionInfo["unquotedIdentifierCase"]>): string {
  if (identifierCase === "lower") return value.toLowerCase();
  if (identifierCase === "upper") return value.toUpperCase();
  return value;
}

export function tableStructureIdentifierComparisonKey(name: string, databaseType?: DatabaseType, databaseInfo?: TableStructureIdentifierCaseInfo): string {
  const value = name.trim();
  const defaults = defaultTableStructureIdentifierCaseInfo(databaseType);
  const unquotedIdentifierCase = databaseInfo?.unquotedIdentifierCase ?? defaults.unquotedIdentifierCase;
  const quotedIdentifierCase = databaseInfo?.quotedIdentifierCase ?? defaults.quotedIdentifierCase;
  const normalizedUnquoted = applyIdentifierCase(value, unquotedIdentifierCase);

  if (unquotedIdentifierCase === "mixed") return `exact:${value}`;
  if (quotedIdentifierCase === "mixed" && value !== normalizedUnquoted) return `quoted:${value}`;
  return `unquoted:${normalizedUnquoted}`;
}

/** Plain-identifier rule for newly created Oracle names: an ASCII letter first,
 * then letters/digits/`_`/`$`/`#`. Anything else is emitted quoted by the DDL
 * generator and keeps its exact spelling. */

/** Plain-identifier rule for newly created Informix-family names: an ASCII
 * letter or `_` first, then letters/digits/`_`/`$`. */

/**
 * Storage name of a newly created table, mirroring how the CREATE DDL
 * generator quotes new identifiers. Plain (unquoted) Oracle names fold to
 * upper case on the server; plain Informix-family names fold to lower case.
 * Every other dialect quotes new names — or does not fold them — so the
 * as-typed spelling is preserved exactly. Names the DDL would have to quote
 * (leading digit, special characters, spaces) keep their exact spelling on
 * the folding dialects too. Boundary: a reserved-word name typed in mixed
 * case is quoted by the generator and keeps its spelling, while this helper
 * still folds it — accepted because the reserved-word vocabulary lives with
 * the SQL builder, not the frontend.
 */
export function foldCreatedTableName(name: string, _databaseType?: DatabaseType): string {
  const value = name.trim();
  {}
  {}
  return value;
}

export const DATA_TYPE_OPTIONS: Record<string, string[]> = {
  mysql: [
    "tinyint",
    "tinyint unsigned",
    "smallint",
    "smallint unsigned",
    "mediumint",
    "mediumint unsigned",
    "int",
    "int unsigned",
    "integer",
    "integer unsigned",
    "bigint",
    "bigint unsigned",
    "float",
    "double",
    "double precision",
    "real",
    "decimal",
    "numeric",
    "bit",
    "boolean",
    "bool",
    "serial",
    "char",
    "varchar",
    "tinytext",
    "text",
    "mediumtext",
    "longtext",
    "binary",
    "varbinary",
    "tinyblob",
    "blob",
    "mediumblob",
    "longblob",
    "enum",
    "set",
    "date",
    "datetime",
    "timestamp",
    "time",
    "year",
    "json",
    "geometry",
    "point",
    "linestring",
    "polygon",
    "multipoint",
    "multilinestring",
    "multipolygon",
    "geometrycollection",
  ],
};

const DATA_TYPE_OPTION_ALIASES: Partial<Record<DatabaseType, string>> = {};

export function getDataTypeOptions(dbType: DatabaseType | undefined): string[] {
  const key = dbType ? (DATA_TYPE_OPTION_ALIASES[dbType] ?? dbType) : "";
  return DATA_TYPE_OPTIONS[key] ?? [];
}

export function isMysqlEnumDataType(dbType: DatabaseType | undefined, dataType: string): boolean {
  return dbType === "mysql" && splitDataType(dataType).baseType.trim().toLowerCase() === "enum";
}

export function mysqlEnumDataType(values: readonly string[]): string {
  // Match MySQL's canonical ENUM literal escaping, including values returned by SHOW CREATE TABLE.
  const literals = values.map((value) => `'${value.replace(/\\/g, "\\\\").replace(/'/g, "''")}'`);
  return `enum(${literals.join(",")})`;
}

export interface ColumnEditorControls {
  length: boolean;
  nullable: boolean;
  primaryKey: boolean;
  defaultValue: boolean;
  comment: boolean;
}

const DEFAULT_COLUMN_EDITOR_CONTROLS: ColumnEditorControls = {
  length: true,
  nullable: true,
  primaryKey: true,
  defaultValue: true,
  comment: true,
};

export function getColumnEditorControls(_dbType: DatabaseType | undefined): ColumnEditorControls {
  {}
  return DEFAULT_COLUMN_EDITOR_CONTROLS;
}

export function isProtectedManticoreIdColumn(_dbType: DatabaseType | undefined, _columnName: string): boolean {
  return false;
}

export function canEditManticoreColumnProperties(_dbType: DatabaseType | undefined, _hasOriginalColumn: boolean): boolean {
  return false;
}

export const DEFAULT_TYPE_LENGTHS: Record<string, string> = {
  tinyint: "4",
  "tinyint unsigned": "4",
  smallint: "6",
  "smallint unsigned": "6",
  mediumint: "9",
  "mediumint unsigned": "9",
  int: "11",
  "int unsigned": "11",
  integer: "11",
  "integer unsigned": "11",
  int4: "11",
  bigint: "20",
  "bigint unsigned": "20",
  int8: "20",
  float: "10,2",
  real: "10,2",
  "double precision": "10,2",
  double: "10,2",
  decimal: "10,0",
  numeric: "10,0",
  number: "10,0",
  char: "1",
  character: "1",
  varchar: "255",
  "character varying": "255",
  varchar2: "255",
  nvarchar2: "255",
  nvarchar: "255",
  nchar: "1",
  varbinary: "255",
  binary: "1",
  bit: "1",
  year: "4",
};

export const QUESTDB_TYPE_LENGTHS: Record<string, string> = {
  geohash: "8c",
  decimal: "10,2",
};

export const SQLSERVER_TYPE_LENGTHS: Record<string, string> = {
  decimal: "10,0",
  numeric: "10,0",
  float: "53",
  char: "1",
  nchar: "1",
  varchar: "255",
  nvarchar: "255",
  binary: "1",
  varbinary: "255",
};

export const DEFAULT_TYPE_LENGTH_DISABLES: string[] = [];

export const POSTGRES_TYPE_LENGTH_DISABLES: string[] = [
  "bigint",
  "int8",
  "bigserial",
  "serial8",
  "boolean",
  "bool",
  "box",
  "bytea",
  "cidr",
  "circle",
  "date",
  "double precision",
  "float",
  "float8",
  "inet",
  "integer",
  "int",
  "int4",
  "json",
  "jsonb",
  "line",
  "lseg",
  "macaddr",
  "macaddr8",
  "money",
  "path",
  "pg_lsn",
  "pg_snapshot",
  "point",
  "polygon",
  "real",
  "float4",
  "smallint",
  "int2",
  "smallserial",
  "serial2",
  "serial",
  "serial4",
  "text",
  "tsquery",
  "tsvector",
  "txid_snapshot",
  "uuid",
  "xml",
];

export const ORACLE_LIKE_TYPE_LENGTH_DISABLES: string[] = ["binary_double", "binary_float", "bigint", "boolean", "bool", "byte", "date", "double", "double precision", "float", "integer", "int", "long", "long raw", "nclob", "real", "smallint", "text", "tinyint"];

export const SQLSERVER_TYPE_LENGTH_DISABLES: string[] = ["bigint", "bit", "date", "datetime", "image", "int", "integer", "money", "ntext", "real", "smalldatetime", "smallint", "smallmoney", "sql_variant", "text", "timestamp", "tinyint", "uniqueidentifier", "xml"];

export function supportsTableStructureExtendedProperties(databaseType?: DatabaseType): boolean {
  return databaseType === "mysql";
}

export function parseExtraToColumnExtra(extra: string | null | undefined, databaseType?: DatabaseType): ColumnExtra {
  const result: ColumnExtra = {};
  if (!extra) return result;
  const lower = extra.toLowerCase().trim();
  if (!lower) return result;

  if (databaseType === "mysql") {
    if (lower.includes("auto_increment") || lower.includes("autoincrement")) {
      result.autoIncrement = true;
    }
    if (databaseType === "mysql" && lower.includes("on update current_timestamp")) {
      result.onUpdateCurrentTimestamp = true;
    }
  } else {
    {
      {
        {
        }
      }
    }
  }

  return result;
}

const MANTICORE_COLUMN_PROPERTY_TOKENS = new Set(["indexed", "stored", "attribute"]);

function splitManticoreDdlColumnLine(line: string): { name: string; dataType: string; extra: string } | null {
  const trimmed = line.trim().replace(/,$/, "").trim();
  if (!trimmed || trimmed.startsWith(")") || trimmed.startsWith("(")) return null;

  let name = "";
  let rest = "";
  const quoted = trimmed.match(/^`((?:``|[^`])+)`\s+(.+)$/);
  if (quoted) {
    name = quoted[1]!.replace(/``/g, "`");
    rest = quoted[2]!.trim();
  } else {
    const plain = trimmed.match(/^([A-Za-z_][\w$]*)\s+(.+)$/);
    if (!plain) return null;
    name = plain[1]!;
    rest = plain[2]!.trim();
  }

  const parts = rest.split(/\s+/).filter(Boolean);
  const dataType = parts.shift() ?? "";
  const properties = parts.filter((part) => {
    const normalized = part.toLowerCase();
    return MANTICORE_COLUMN_PROPERTY_TOKENS.has(normalized) || /^secondary_index\s*=/.test(normalized);
  });
  if (!name || !dataType || properties.length === 0) return null;

  return { name, dataType, extra: properties.join(" ") };
}

export function applyManticoreDdlColumnExtras(columns: ColumnInfo[], ddl: string): ColumnInfo[] {
  if (!ddl.trim()) return columns;
  const extrasByColumn = new Map<string, { dataType: string; extra: string }>();
  for (const line of ddl.split(/\r?\n/)) {
    const parsed = splitManticoreDdlColumnLine(line);
    if (parsed) extrasByColumn.set(parsed.name.toLowerCase(), { dataType: parsed.dataType, extra: parsed.extra });
  }
  if (extrasByColumn.size === 0) return columns;

  return columns.map((column) => {
    const ddlColumn = extrasByColumn.get(column.name.toLowerCase());
    if (!ddlColumn) return column;
    const existingExtra = column.extra?.trim();
    return {
      ...column,
      data_type: ddlColumn.dataType || column.data_type,
      extra: existingExtra ? `${existingExtra} ${ddlColumn.extra}` : ddlColumn.extra,
    };
  });
}

function columnDefaultForEditor(column: Pick<ColumnInfo, "column_default" | "data_type">, databaseType?: DatabaseType): string {
  if (column.column_default === null) return "";
  const defaultValue = column.column_default;
  if (databaseType === "mysql" && defaultValue === "" && isMysqlCharacterDataType(column.data_type)) {
    // MySQL metadata uses an empty string for DEFAULT '', so keep it distinct from no default.
    return "''";
  }
  {}
  {}
  return defaultValue;
}

const CHARACTER_LENGTH_METADATA_TYPES = new Set(["binary", "bpchar", "char", "character", "character varying", "nchar", "nvarchar", "nvarchar2", "varbinary", "varchar", "varchar2"]);
const NUMERIC_PRECISION_METADATA_TYPES = new Set(["decimal", "number", "numeric"]);

function columnDataTypeForEditor(column: ColumnInfo, databaseType?: DatabaseType): string {
  {}
  const parsed = splitDataTypeForDatabase(databaseType, column.data_type);
  if (parsed.params) return column.data_type;

  const baseType = parsed.baseType.trim().replace(/\s+/g, " ");
  const normalized = baseType.toLowerCase();
  if (CHARACTER_LENGTH_METADATA_TYPES.has(normalized) && Number.isInteger(column.character_maximum_length) && Number(column.character_maximum_length) > 0) {
    return combineDataTypeForDatabase(databaseType, baseType, String(column.character_maximum_length));
  }
  {}
  if (NUMERIC_PRECISION_METADATA_TYPES.has(normalized) && Number.isInteger(column.numeric_precision) && Number(column.numeric_precision) > 0) {
    const scale = Number.isInteger(column.numeric_scale) && Number(column.numeric_scale) >= 0 ? `,${column.numeric_scale}` : "";
    return combineDataTypeForDatabase(databaseType, baseType, `${column.numeric_precision}${scale}`);
  }
  return column.data_type;
}

export function createColumnDrafts(columns: ColumnInfo[], databaseType?: DatabaseType): EditableStructureColumn[] {
  return columns.map((column, index) => {
    const defaultValue = columnDefaultForEditor(column, databaseType);
    const enumValues = isMysqlEnumDataType(databaseType, column.data_type) ? [...(column.enum_values ?? [])] : undefined;
    const dataType = enumValues?.length ? mysqlEnumDataType(enumValues) : columnDataTypeForEditor(column, databaseType);
    return {
      id: `existing:${column.name}`,
      name: column.name,
      dataType,
      enumValues,
      isNullable: column.is_nullable,
      defaultValue,
      comment: column.comment ?? "",
      isPrimaryKey: column.is_primary_key,
      characterSet: column.character_set ?? "",
      collation: column.collation ?? "",
      extra: parseExtraToColumnExtra(column.extra, databaseType),
      original: { ...column, data_type: dataType, column_default: column.column_default === null ? null : defaultValue },
      originalPosition: index,
      markedForDrop: false,
    };
  });
}

/**
 * Turns another table's metadata into columns that will be added to the table
 * currently being edited. Unlike createColumnDrafts(), these must not retain
 * original metadata: the SQL builder uses that metadata to identify existing
 * columns that should be altered.
 */
export function createCopiedColumnDrafts(columns: ColumnInfo[], databaseType: DatabaseType | undefined, createId: () => string): EditableStructureColumn[] {
  return createColumnDrafts(columns, databaseType).map(({ original: _original, originalPosition: _originalPosition, isPrimaryKey: _isPrimaryKey, extra, ...column }) => ({
    ...column,
    id: `new:${createId()}`,
    isPrimaryKey: false,
    extra: copyableColumnExtra(extra),
  }));
}

/** Copy only field-local extras; keys and generated-value state are table-level concerns. */
function copyableColumnExtra(extra: ColumnExtra): ColumnExtra {
  const { autoIncrement: _autoIncrement, identity: _identity, ...copyableExtra } = extra;
  return copyableExtra;
}

/** Clone an editable field as a new column, without linking it to persisted metadata or key state. */
export function cloneColumnDraftAsNew(column: EditableStructureColumn, createId: () => string): EditableStructureColumn {
  return {
    id: `new:${createId()}`,
    name: column.name,
    dataType: column.dataType,
    enumValues: column.enumValues ? [...column.enumValues] : undefined,
    isNullable: column.isNullable,
    defaultValue: column.defaultValue,
    comment: column.comment,
    isPrimaryKey: false,
    characterSet: column.characterSet,
    collation: column.collation,
    extra: copyableColumnExtra(column.extra),
    markedForDrop: false,
  };
}

function existingColumnIdName(id: string): string | undefined {
  const prefix = "existing:";
  return id.startsWith(prefix) ? id.slice(prefix.length) : undefined;
}

function isNewColumnDraftId(id: string): boolean {
  return id.startsWith("new:");
}

function uniqueNames(names: Array<string | undefined>): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const name of names) {
    if (!name) continue;
    if (seen.has(name)) continue;
    seen.add(name);
    result.push(name);
  }
  return result;
}

function findColumnDraftByName(columns: EditableStructureColumn[], names: string[], usedIndexes: Set<number>): number | undefined {
  for (const name of names) {
    const exactIndex = columns.findIndex((column, index) => !usedIndexes.has(index) && column.name === name);
    if (exactIndex >= 0) return exactIndex;
  }

  for (const name of names) {
    const lowerName = name.toLowerCase();
    const matches = columns.map((column, index) => ({ column, index })).filter(({ column, index }) => !usedIndexes.has(index) && column.name.toLowerCase() === lowerName);
    if (matches.length === 1) return matches[0]!.index;
  }

  return undefined;
}

export function rehydrateColumnDraftsFromMetadata(draftColumns: EditableStructureColumn[], columns: ColumnInfo[], databaseType?: DatabaseType): EditableStructureColumn[] {
  const metadataDrafts = createColumnDrafts(columns, databaseType);
  if (!metadataDrafts.length) return draftColumns;
  if (!draftColumns.length) return metadataDrafts;

  const usedMetadataIndexes = new Set<number>();
  const nextColumns = draftColumns.map((column) => {
    if (!column.original && isNewColumnDraftId(column.id)) return column;

    const needsHydration = !column.original || column.originalPosition === undefined;
    const candidates = uniqueNames([column.original?.name, existingColumnIdName(column.id), column.name]);
    const metadataIndex = findColumnDraftByName(metadataDrafts, candidates, usedMetadataIndexes);
    if (metadataIndex === undefined) return column;
    usedMetadataIndexes.add(metadataIndex);
    const metadataDraft = metadataDrafts[metadataIndex]!;
    const shouldHydrateEnum = isMysqlEnumDataType(databaseType, column.dataType) && column.enumValues === undefined && metadataDraft.enumValues !== undefined;
    if (!needsHydration && !shouldHydrateEnum) return column;

    return {
      ...column,
      dataType: shouldHydrateEnum ? metadataDraft.dataType : column.dataType,
      enumValues: column.enumValues ?? metadataDraft.enumValues,
      original: shouldHydrateEnum ? metadataDraft.original : (column.original ?? metadataDraft.original),
      originalPosition: column.originalPosition ?? metadataDraft.originalPosition,
    };
  });

  if (usedMetadataIndexes.size === 0) {
    return [...metadataDrafts, ...draftColumns];
  }

  const missingMetadataDrafts = metadataDrafts.filter((_, index) => !usedMetadataIndexes.has(index));
  return [...nextColumns, ...missingMetadataDrafts];
}

/** Canonicalize index method for structure editor options (e.g. Postgres `btree` → `BTREE`). */
export function normalizeStructureIndexType(indexType: string | null | undefined): string {
  return (indexType ?? "").trim().toUpperCase();
}

/** Case-insensitive index-type equality (draft is uppercased; API may still return lowercase amname). */
export function sameStructureIndexType(left: string | null | undefined, right: string | null | undefined): boolean {
  return normalizeStructureIndexType(left) === normalizeStructureIndexType(right);
}

/** Keep selected fields removable even after they are no longer available on the table. */
export function filterStructureIndexColumnOptions(availableColumns: readonly string[], selectedColumns: readonly string[], search = ""): string[] {
  const availableSet = new Set(availableColumns);
  const unavailableSelected = selectedColumns.filter((column) => column.trim() && !availableSet.has(column));
  const options = [...new Set([...unavailableSelected, ...availableColumns])];
  const query = search.trim().toLowerCase();
  return query ? options.filter((column) => column.toLowerCase().includes(query)) : options;
}

export function createIndexDrafts(indexes: IndexInfo[]): EditableStructureIndex[] {
  return indexes.map((index) => ({
    id: `existing:${index.name}`,
    name: index.name,
    columns: [...index.columns],
    nameEdited: true,
    isUnique: index.is_unique,
    isPrimary: index.is_primary,
    filter: index.filter ?? "",
    // Match Select options (BTREE/GIN/…); Postgres pg_am.amname is lowercase.
    indexType: normalizeStructureIndexType(index.index_type),
    includedColumns: index.included_columns ? [...index.included_columns] : [],
    comment: index.comment ?? "",
    columnOpclasses: index.column_opclasses ? [...index.column_opclasses] : [],
    original: index,
    markedForDrop: false,
  }));
}

export function createForeignKeyDrafts(foreignKeys: ForeignKeyInfo[]): EditableStructureForeignKey[] {
  const groups = new Map<string, ForeignKeyInfo[]>();
  for (const foreignKey of foreignKeys) {
    const key = [foreignKey.name, foreignKey.ref_schema ?? "", foreignKey.ref_table, foreignKey.on_update ?? "", foreignKey.on_delete ?? ""].join("\u0000");
    groups.set(key, [...(groups.get(key) ?? []), foreignKey]);
  }

  return [...groups.values()].map((group, index) => {
    const first = group[0]!;
    const original = {
      ...first,
      column: group.map((foreignKey) => foreignKey.column).join(", "),
      ref_column: group.map((foreignKey) => foreignKey.ref_column).join(", "),
    };
    return {
      id: `existing:${first.name}:${index}`,
      name: first.name,
      column: original.column,
      refSchema: first.ref_schema ?? "",
      refTable: first.ref_table,
      refColumn: original.ref_column,
      onUpdate: first.on_update ?? "",
      onDelete: first.on_delete ?? "",
      original,
      markedForDrop: false,
    };
  });
}

export function createTriggerDrafts(triggers: TriggerInfo[]): EditableStructureTrigger[] {
  return triggers.map((trigger) => ({
    id: `existing:${trigger.name}`,
    name: trigger.name,
    timing: trigger.timing,
    event: trigger.event,
    statement: trigger.statement ?? "",
    original: trigger,
    markedForDrop: false,
  }));
}

export function canEditStructuredTriggerDraft(databaseType: DatabaseType | undefined, trigger: EditableStructureTrigger): boolean {
  return !trigger.original || databaseType !== undefined;
}

export function toColumnNames(columns: string[]): string {
  return columns.join(", ");
}

const AUTO_INDEX_NAME_MAX_LENGTH = 63;

export type StructureIndexKind = "primary" | "unique" | "index" | "fulltext" | "spatial";
type IndexNamingOptions = Partial<Pick<EditableStructureIndex, "isPrimary" | "isUnique" | "indexType">> & { maxLength?: number };

export function structureIndexKind(index: IndexNamingOptions): StructureIndexKind {
  if (index.isPrimary) return "primary";
  if (index.indexType?.trim().toUpperCase() === "FULLTEXT") return "fulltext";
  if (index.indexType?.trim().toUpperCase() === "SPATIAL") return "spatial";
  return index.isUnique ? "unique" : "index";
}

export type SpecialIndexColumnIssue = "specialIndexUnique" | "fulltextIndexColumns" | "spatialIndexColumn" | "spatialIndexNullable";

export function specialIndexColumnIssue(dialect: string, indexType: string, columns: readonly Pick<EditableStructureColumn, "dataType" | "isNullable">[], isUnique = false): SpecialIndexColumnIssue | null {
  const type = indexType.trim().toUpperCase();
  const fulltext = dialect === "mysql" && type === "FULLTEXT";
  const spatial = dialect === "mysql" && type === "SPATIAL";
  if (!fulltext && !spatial) return null;
  if (isUnique) return "specialIndexUnique";
  if (!columns.length) return null;
  const types = columns.map((column) => splitDataType(column.dataType).baseType.toLowerCase());
  if (fulltext && types.some((value) => !/^(?:char|varchar|tinytext|text|mediumtext|longtext)$/.test(value))) return "fulltextIndexColumns";
  if (spatial) {
    const validType = dialect === "mysql" ? /^(?:geometry|point|linestring|polygon|multipoint|multilinestring|multipolygon|geometrycollection|geomcollection)$/ : /^(?:geometry|geography)$/;
    if (columns.length !== 1 || !validType.test(types[0])) return "spatialIndexColumn";
    if (dialect === "mysql" && columns[0].isNullable) return "spatialIndexNullable";
  }
  return null;
}

function normalizeIndexNamePart(value: string): string {
  const trimmed = value.trim();
  const unquoted = (trimmed.startsWith("[") && trimmed.endsWith("]")) || (trimmed.startsWith("`") && trimmed.endsWith("`")) || (trimmed.startsWith('"') && trimmed.endsWith('"')) ? trimmed.slice(1, -1) : trimmed;
  return unquoted
    .trim()
    .replace(/[^a-zA-Z0-9]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toUpperCase();
}

function truncateIndexName(value: string, maxLength = AUTO_INDEX_NAME_MAX_LENGTH): string {
  if (value.length <= maxLength) return value;
  const suffix = "_IDX";
  if (!value.endsWith(suffix)) return value.slice(0, maxLength).replace(/_+$/g, "");
  return `${value.slice(0, maxLength - suffix.length).replace(/_+$/g, "")}${suffix}`;
}

// Keep the original API and table-qualified naming for existing callers and dialects.
export function generateIndexName(tableName: string, columns: string[], maxLength = AUTO_INDEX_NAME_MAX_LENGTH): string {
  const parts = [tableName, ...columns].map(normalizeIndexNamePart).filter(Boolean);
  if (parts.length === 0) return "";
  return truncateIndexName(`${parts.join("_")}_IDX`, maxLength);
}

export function generateUniqueIndexName(tableName: string, columns: string[], existingNames: Iterable<string>, maxLength = AUTO_INDEX_NAME_MAX_LENGTH): string {
  const base = generateIndexName(tableName, columns, maxLength);
  if (!base) return "";

  const taken = new Set([...existingNames].map((name) => name.trim().toLowerCase()).filter(Boolean));
  if (!taken.has(base.toLowerCase())) return base;

  for (let counter = 2; counter < 10_000; counter++) {
    const suffix = `_${counter}`;
    const stem = base.length + suffix.length <= maxLength ? base : base.slice(0, maxLength - suffix.length).replace(/_+$/g, "");
    const candidate = `${stem}${suffix}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
  return base;
}

function indexNameHash(value: string): string {
  // Keep long names deterministic and distinguish columns beyond the cutoff.
  let hash = 2166136261;
  for (let i = 0; i < value.length; i++) hash = Math.imul(hash ^ value.charCodeAt(i), 16777619);
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function truncateShortIndexName(value: string, maxLength = AUTO_INDEX_NAME_MAX_LENGTH): string {
  if (value.length <= maxLength) return value;
  const suffix = `_${indexNameHash(value)}`;
  if (maxLength <= suffix.length) return value.slice(0, maxLength);
  return `${value.slice(0, maxLength - suffix.length).replace(/_+$/g, "")}${suffix}`;
}

export function generateShortIndexName(columnName: string, options: IndexNamingOptions = {}): string {
  if (options.isPrimary) return "PRIMARY";
  if (!columnName.trim()) return "";
  // MySQL permits BMP Unicode identifiers, but not supplementary characters.
  const normalized = columnName
    .trim()
    .replace(/[^\p{L}\p{N}]+/gu, "_")
    .replace(/[\uD800-\uDFFF]/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
  const name = normalized || `column_${indexNameHash(columnName)}`;
  const prefix = { primary: "pk", unique: "uk", index: "idx", fulltext: "ft", spatial: "sp" }[structureIndexKind(options)];
  return truncateShortIndexName(`${prefix}_${name}`, options.maxLength);
}

export function generateUniqueShortIndexName(columnName: string, existingNames: Iterable<string>, options: IndexNamingOptions = {}): string {
  const maxLength = options.maxLength ?? AUTO_INDEX_NAME_MAX_LENGTH;
  const base = generateShortIndexName(columnName, options);
  if (!base) return "";
  if (options.isPrimary) return base;

  const taken = new Set([...existingNames].map((name) => name.trim().toLowerCase()).filter(Boolean));
  if (!taken.has(base.toLowerCase())) return base;

  for (let counter = 2; ; counter++) {
    const suffix = `_${counter}`;
    const stem = generateShortIndexName(columnName, { ...options, maxLength: Math.max(1, maxLength - suffix.length) });
    const candidate = `${stem}${suffix}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

export function splitDataType(raw: string): { baseType: string; params: string } {
  const trimmed = raw.trim();
  const parenIdx = trimmed.indexOf("(");
  if (parenIdx === -1) return { baseType: trimmed, params: "" };
  const closeIdx = trimmed.lastIndexOf(")");
  const baseTypePrefix = trimmed.slice(0, parenIdx).trim();
  const params = trimmed.slice(parenIdx + 1, closeIdx).trim();
  const suffix = trimmed
    .slice(closeIdx + 1)
    .trim()
    .replace(/\s+/g, " ");
  const baseType = /^(?:signed|unsigned|zerofill)(?:\s+(?:signed|unsigned|zerofill))*$/i.test(suffix) ? `${baseTypePrefix} ${suffix}`.trim() : baseTypePrefix;
  return { baseType, params };
}

function splitDataTypeForDatabase(_dbType: DatabaseType | undefined, raw: string): { baseType: string; params: string } {
  {}
  {}
  {}
  return splitDataType(raw);
}

export function dataTypeBaseInputValue(dbType: DatabaseType | undefined, rawDataType: string): string {
  return splitDataTypeForDatabase(dbType, rawDataType).baseType;
}

export type DataTypeLengthUnit = "BYTE" | "CHAR";

function normalizedDataTypeName(rawDataType: string): string {
  return splitDataType(rawDataType).baseType.trim().replace(/\s+/g, " ").toLowerCase();
}

export function getDataTypeLengthUnitOptions(_dbType: DatabaseType | undefined, _rawDataType: string): readonly DataTypeLengthUnit[] {
  {
    return [];
  }
}

function splitDataTypeLengthParams(dbType: DatabaseType | undefined, rawDataType: string): { length: string; unit: DataTypeLengthUnit | "" } {
  const { params } = splitDataTypeForDatabase(dbType, rawDataType);
  if (!params || getDataTypeLengthUnitOptions(dbType, rawDataType).length === 0) {
    return { length: params, unit: "" };
  }

  const match = params.match(/^(.*\S)\s+(BYTE|CHAR)$/i);
  if (!match) return { length: params, unit: "" };
  return {
    length: match[1]!.trim(),
    unit: match[2]!.toUpperCase() as DataTypeLengthUnit,
  };
}

export function dataTypeLengthUnitValue(dbType: DatabaseType | undefined, rawDataType: string): DataTypeLengthUnit | "" {
  return splitDataTypeLengthParams(dbType, rawDataType).unit;
}

export function combineDataTypeForDatabaseWithLengthUnit(dbType: DatabaseType | undefined, baseType: string, length: string, unit: string | undefined): string {
  const normalizedLength = length.trim();
  if (!normalizedLength) return combineDataTypeForDatabase(dbType, baseType, "");

  const normalizedUnit = unit?.trim().toUpperCase();
  const validUnit = normalizedUnit === "BYTE" || normalizedUnit === "CHAR" ? normalizedUnit : "";
  const allowedUnits = getDataTypeLengthUnitOptions(dbType, baseType);
  const params = validUnit && allowedUnits.includes(validUnit) ? `${normalizedLength} ${validUnit}` : normalizedLength;
  return combineDataTypeForDatabase(dbType, baseType, params);
}

export function restoreCharacterLengthUnitsAfterSave(dbType: DatabaseType | undefined, columns: EditableStructureColumn[], savedDataTypesByColumn: ReadonlyMap<string, string>): EditableStructureColumn[] {
  if (savedDataTypesByColumn.size === 0) return columns;

  return columns.map((column) => {
    if (dataTypeLengthUnitValue(dbType, column.dataType)) return column;
    const savedDataType = savedDataTypesByColumn.get(column.name.trim().toLowerCase());
    if (!savedDataType || !dataTypeLengthUnitValue(dbType, savedDataType)) return column;
    if (normalizedDataTypeName(savedDataType) !== normalizedDataTypeName(column.dataType)) return column;

    return {
      ...column,
      dataType: savedDataType,
      original: column.original ? { ...column.original, data_type: savedDataType } : column.original,
    };
  });
}

/** MySQL character/text types that accept `CHARACTER SET` and `COLLATE`. */
const MYSQL_CHARACTER_DATA_TYPES = new Set(["char", "varchar", "tinytext", "text", "mediumtext", "longtext", "enum", "set"]);

export function isMysqlCharacterDataType(dataType: string): boolean {
  const { baseType } = splitDataType(dataType);
  return MYSQL_CHARACTER_DATA_TYPES.has(baseType.trim().replace(/\s+/g, " ").toLowerCase());
}

export function isSqlServerIdentityCompatibleDataType(rawDataType: string): boolean {
  const { baseType, params } = splitDataType(rawDataType);
  const normalized = baseType.trim().replace(/\s+/g, " ").toLowerCase();
  if (["tinyint", "smallint", "int", "integer", "bigint"].includes(normalized)) return true;
  if (normalized !== "decimal" && normalized !== "numeric") return false;
  const normalizedParams = params.replace(/\s+/g, "");
  if (!normalizedParams) return true;
  const parts = normalizedParams.split(",");
  if (parts.length === 1) return /^\d+$/.test(parts[0] ?? "");
  if (parts.length !== 2) return false;
  const [precision, scale] = parts;
  return /^\d+$/.test(precision ?? "") && scale === "0";
}

export function isDamengIdentityCompatibleDataType(rawDataType: string): boolean {
  const { baseType, params } = splitDataType(rawDataType);
  const normalized = baseType.trim().replace(/\s+/g, " ").toLowerCase();
  if (["tinyint", "smallint", "int", "integer", "bigint"].includes(normalized)) return true;
  if (!["number", "numeric", "decimal", "dec"].includes(normalized)) return false;
  const normalizedParams = params.replace(/\s+/g, "");
  if (!normalizedParams) return true;
  const parts = normalizedParams.split(",");
  if (parts.length === 1) return /^\d+$/.test(parts[0] ?? "");
  if (parts.length !== 2) return false;
  const [precision, scale] = parts;
  return /^\d+$/.test(precision ?? "") && scale === "0";
}

export function combineDataType(baseType: string, params: string): string {
  const type = baseType.trim();
  const p = params.trim();
  if (!type) return "";
  if (!p) return type;
  return `${type}(${p})`;
}

export function combineDataTypeForDatabase(dbType: DatabaseType | undefined, baseType: string, params: string): string {
  {}
  if (isDataTypeLengthDisabled(dbType, baseType)) {
    return baseType;
  }
  const normalizedParams = normalizeDataTypeParams(dbType, baseType, params);
  const mysqlType = combineMysqlNumericAttributeType(dbType, baseType, normalizedParams);
  if (mysqlType) return mysqlType;
  const qualifiedTemporalType = combineQualifiedTemporalType(baseType, normalizedParams, dbType);
  if (qualifiedTemporalType) return qualifiedTemporalType;
  return combineDataType(baseType, normalizedParams);
}

export function dataTypeLengthInputValue(dbType: DatabaseType | undefined, rawDataType: string): string {
  const parsed = splitDataTypeForDatabase(dbType, rawDataType);
  {}
  return isDataTypeLengthDisabled(dbType, parsed.baseType) ? "" : splitDataTypeLengthParams(dbType, rawDataType).length;
}

function combineQualifiedTemporalType(_baseType: string, _params: string, _dbType: DatabaseType | undefined): string | null {
  {}
  {}
  {
    return null;
  }
}

export function normalizeDataTypeParams(dbType: DatabaseType | undefined, baseType: string, params: string): string {
  const p = params.trim();
  if (!p) return "";
  {}
  if (!isTemporalPrecisionType(dbType, baseType)) return p;
  return isValidTemporalPrecision(dbType, baseType, p) ? p : "";
}

function isTemporalPrecisionType(dbType: DatabaseType | undefined, baseType: string): boolean {
  const normalized = baseType.trim().replace(/\s+/g, " ").toLowerCase();
  switch (dbType) {
    case "mysql":
      return ["time", "datetime", "timestamp"].includes(normalized);

    default:
      return false;
  }
}

function combineMysqlNumericAttributeType(dbType: DatabaseType | undefined, baseType: string, params: string): string | null {
  if (!params || !isMysqlLikeStructureType(dbType)) return null;
  const parts = baseType.trim().replace(/\s+/g, " ").split(" ").filter(Boolean);
  const typeName = parts[0]?.toLowerCase();
  if (!typeName || !["tinyint", "smallint", "mediumint", "int", "integer", "bigint", "real", "double", "float", "decimal", "numeric"].includes(typeName)) return null;
  const attrIndex = parts.findIndex((part) => ["signed", "unsigned", "zerofill"].includes(part.toLowerCase()));
  if (attrIndex === -1) return null;
  if (!parts.slice(attrIndex).every((part) => ["signed", "unsigned", "zerofill"].includes(part.toLowerCase()))) return null;
  return `${parts.slice(0, attrIndex).join(" ")}(${params}) ${parts.slice(attrIndex).join(" ")}`;
}

function isMysqlLikeStructureType(dbType: DatabaseType | undefined): boolean {
  return dbType === "mysql";
}

function isValidTemporalPrecision(_dbType: DatabaseType | undefined, baseType: string, params: string): boolean {
  if (!/^\d+$/.test(params)) return false;
  const value = Number(params);
  baseType.trim().replace(/\s+/g, " ").toLowerCase();
  const max = 6;
  return Number.isInteger(value) && value >= 0 && value <= max && String(value) === params;
}

export interface DataTypeDefaultOptions {
  /**
   * Native MySQL profiles use MySQL 8-safe defaults. Compatibility profiles
   * retain their existing DDL defaults because their server/version is unknown.
   */
  omitMysqlDeprecatedDefaults?: boolean;
}

export function getDefaultLengthForType(_dbType: DatabaseType | undefined, baseType: string, options: DataTypeDefaultOptions = {}): string {
  const key = baseType.trim().toLowerCase();
  {}
  if (_dbType === "mysql" && options.omitMysqlDeprecatedDefaults && isMysqlDeprecatedDefaultParameterType(key)) return "";
  {
    {
      {
        return DEFAULT_TYPE_LENGTHS[key] ?? "";
      }
    }
  }
}

/** Default data type for a newly added structure-editor column. */
export function defaultNewColumnDataType(dbType: DatabaseType | undefined, dataTypeOptions: readonly string[] = []): string {
  {}

  const options = dataTypeOptions.length > 0 ? dataTypeOptions : getDataTypeOptions(dbType);

  {}

  if (options.length > 0) {
    const preferred = options.find((type) => /^(varchar|character varying|nvarchar)$/i.test(type.trim())) ?? options.find((type) => /^(string|clob|lvarchar|text)$/i.test(type.trim())) ?? options.find((type) => /^varchar/i.test(type.trim()));
    if (preferred) {
      return combineDataTypeForDatabase(dbType, preferred, getDefaultLengthForType(dbType, preferred));
    }
  }

  return "varchar(255)";
}

/** Index at which to insert a new column (after the selected row, or append when none). */
export function resolveInsertColumnIndex(columns: readonly { id: string; markedForDrop?: boolean }[], selectedColumnId: string | null | undefined): number {
  if (!selectedColumnId) return columns.length;
  // Dropped rows are not valid insertion anchors.
  const index = columns.findIndex((column) => column.id === selectedColumnId && !column.markedForDrop);
  return index >= 0 ? index + 1 : columns.length;
}

/**
 * Compute the contiguous column-id range for a shift-click in the structure
 * editor, mirroring the object browser's range-select behavior
 * (objectBrowserSelection.ts): the range spans every row between the anchor
 * and the clicked row in visible order, but rows marked for drop are not
 * selectable and are dropped from the result.
 */
export function structureColumnSelectionRange(columns: readonly { id: string; markedForDrop?: boolean }[], anchorId: string, currentId: string): string[] {
  const anchorIndex = columns.findIndex((column) => column.id === anchorId);
  const currentIndex = columns.findIndex((column) => column.id === currentId);
  if (anchorIndex < 0 || currentIndex < 0) {
    return columns.some((column) => column.id === currentId && !column.markedForDrop) ? [currentId] : [];
  }
  const start = Math.min(anchorIndex, currentIndex);
  const end = Math.max(anchorIndex, currentIndex);
  return columns
    .slice(start, end + 1)
    .filter((column) => !column.markedForDrop)
    .map((column) => column.id);
}

function isMysqlDeprecatedDefaultParameterType(baseType: string): boolean {
  const typeName = baseType.split(/\s+/)[0];
  return ["tinyint", "smallint", "mediumint", "int", "integer", "bigint", "float", "double", "real"].includes(typeName ?? "");
}

export function isDataTypeLengthDisabled(_dbType: DatabaseType | undefined, baseType: string): boolean {
  const key = baseType.trim().toLowerCase();
  {
    {
      {
        {
          {
            {
              {
                if (isMysqlLikeStructureType(_dbType)) {
                  return key === "enum" || key === "set";
                } else {
                  return false;
                }
              }
            }
          }
        }
      }
    }
  }
}

export function buildStructureTargetLabel(connectionName: string | undefined, database: string | undefined, schema: string | undefined, tableName: string | undefined): string {
  const parts = [connectionName, database];
  if (schema && schema !== database) parts.push(schema);
  if (tableName) parts.push(tableName);
  return parts.filter(Boolean).join(" / ");
}

/** PostGIS `geometry(...)`/`geography(...)` typmod accepts these geometry sub-type
 * names (case-insensitive). An empty value means "no sub-type constraint". */
export const POSTGRES_GEOMETRY_TYPES: readonly string[] = ["Point", "LineString", "Polygon", "MultiPoint", "MultiLineString", "MultiPolygon", "GeometryCollection", "CircularString", "CompoundCurve", "CurvePolygon", "MultiCurve", "MultiSurface", "PolyhedralSurface", "TIN", "Triangle"];

/** Whether a column is a PostGIS `geometry`/`geography` on a PostgreSQL-family
 * database — the case where the structure editor shows dedicated geometry-type
 * and SRID controls instead of the generic length input. */
export function isPostgresGeometryDataType(_dbType: DatabaseType | undefined, _rawDataType: string): boolean {
  {
    return false;
  }
}

/** Geometry sub-type parsed from `geometry(Point,4326)` → `"Point"`. Returns empty
 * for bare `geometry` (no typmod) and for `geometry(GEOMETRY,srid)` — the latter is
 * PostGIS's storage form of "any sub-type" (what a blank sub-type + SRID compiles to),
 * normalized back to empty to match the user-facing "leave blank" intent. This keeps
 * the clear (X) button visually emptying the field instead of showing "GEOMETRY".
 * `geometry(,4326)` is invalid and never produced. */
export function postgresGeometryTypeValue(rawDataType: string): string {
  const { params } = splitDataType(rawDataType);
  const geomType = splitPostgresGeometryParams(params).geomType;
  return geomType.toLowerCase() === "geometry" ? "" : geomType;
}

/** SRID parsed from `geometry(Point,4326)` → `"4326"`. Empty when unspecified. */
export function postgresGeometrySridValue(rawDataType: string): string {
  const { params } = splitDataType(rawDataType);
  return splitPostgresGeometryParams(params).srid;
}

function splitPostgresGeometryParams(params: string): { geomType: string; srid: string } {
  const trimmed = params.trim();
  if (!trimmed) return { geomType: "", srid: "" };
  const commaIndex = trimmed.indexOf(",");
  if (commaIndex === -1) return { geomType: trimmed, srid: "" };
  return { geomType: trimmed.slice(0, commaIndex).trim(), srid: trimmed.slice(commaIndex + 1).trim() };
}

/** Reassemble a PostGIS spatial type string from its parts.
 *
 * - both empty → bare `geometry` (no typmod)
 * - sub-type only → `geometry(Point)`
 * - SRID only   → `geometry(GEOMETRY,4326)` (PostGIS rejects `geometry(,4326)` (syntax error at `,`) and `geometry(4326)` ("Invalid geometry type modifier"); `GEOMETRY` is a valid sub-type token meaning any geometry, so it is the canonical way to constrain SRID only)
 * - both        → `geometry(Point,4326)`
 */
export function combinePostgresGeometryType(baseType: string, geomType: string, srid: string): string {
  const type = baseType.trim();
  const geom = geomType.trim();
  const sridValue = srid.trim();
  if (!type) return "";
  if (!geom && !sridValue) return type;
  if (!geom) return `${type}(GEOMETRY,${sridValue})`; // PostGIS rejects geometry(,4326) and geometry(4326); GEOMETRY = any sub-type, constrains SRID only
  if (!sridValue) return `${type}(${geom})`;
  return `${type}(${geom},${sridValue})`;
}
