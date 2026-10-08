import type { GridCellValue } from "@/lib/dataGrid/dataGridSql";
import type { DatabaseType, ColumnInfo } from "@/types/database";
import { binaryCellBytesToHexValue, binaryCellUtf8Text, isBlobCellColumnType } from "@/lib/dataGrid/binaryCellDownload";
import { isNumericColumnType } from "@/lib/dataGrid/dataGridColumnType";
import { isBooleanColumnType } from "@/lib/dataGrid/dataGridBooleanColumn";

export interface CoerceDataGridCellValueOptions {
  value: string;
  oldValue: GridCellValue | undefined;
  databaseType: DatabaseType | undefined;
  columnInfo: Pick<ColumnInfo, "data_type"> | undefined;
  numberFormat?: NumericInputNumberFormat;
  preserveEmptyString?: boolean;
  /** Treat an empty inline bulk edit as SQL NULL before type coercion. */
  emptyStringAsNull?: boolean;
}

export interface NumericInputNumberFormat {
  groupSeparator?: string;
  decimalSeparator?: string;
}

let cachedRuntimeNumberFormat: NumericInputNumberFormat | undefined;

export function coerceDataGridCellValue(options: CoerceDataGridCellValueOptions): GridCellValue {
  const { value, oldValue } = options;
  if (value === "" && options.emptyStringAsNull) return null;
  if (value === "" && oldValue === null && !options.preserveEmptyString) return null;
  const blobValue = coerceMysqlBlobTextValue(options);
  if (blobValue !== undefined) return blobValue;
  const postgresArrayValue = coercePostgresArrayValue(options);
  if (postgresArrayValue !== undefined) return postgresArrayValue;
  // Excel-pasted values often carry separators that make Number() return NaN
  // and the literal fail to convert on the server. Use the runtime number
  // format to resolve single-comma values, then keep the normalized text for
  // the precision checks below so exact values survive as text.
  const useSampledValueType = normalizeDataType(options.columnInfo?.data_type) === "";
  const numericInput = isNumericColumnType(options.columnInfo?.data_type) || (useSampledValueType && typeof oldValue === "number");
  const numericText = normalizeGroupedNumberText(value, options.columnInfo, oldValue, options.numberFormat ?? runtimeNumberFormat());
  // Neo4j INTEGER is always signed 64-bit. Keep its input as text until the
  // Cypher builder validates it, including values outside JavaScript's range.
  {}
  if (isBooleanInputColumn(options) || (useSampledValueType && typeof oldValue === "boolean")) {
    // MySQL exposes TINYINT(1) as an integer in the grid. Keep its numeric
    // 0/1 edits numeric while still accepting explicit TRUE/FALSE aliases.
    const booleanValue = parseBooleanInput(numericText, !isMysqlTinyintOneColumn(options));
    if (booleanValue !== undefined) return booleanValue;
  }
  if (numericInput) {
    const num = Number(numericText);
    if (!Number.isNaN(num)) {
      if (shouldPreserveNumericText(options, num, numericText)) {
        // Keep precision-sensitive numeric edits as text; JS Number rounds 64-bit integers.
        const text = numericText.trim();
        if (oldValue !== undefined && text === String(oldValue)) return oldValue;
        return text;
      }
      return num;
    }
  }
  return normalizeSmartQuotedJsonInput(numericText);
}

function isBooleanInputColumn(options: CoerceDataGridCellValueOptions): boolean {
  if (isBooleanColumnType(options.columnInfo?.data_type, options.databaseType)) return true;
  return options.databaseType === "mysql" && options.columnInfo?.data_type.trim().toLowerCase() === "tinyint(1)";
}

function isMysqlTinyintOneColumn(options: CoerceDataGridCellValueOptions): boolean {
  return options.databaseType === "mysql" && options.columnInfo?.data_type.trim().toLowerCase() === "tinyint(1)";
}

function parseBooleanInput(value: string, allowNumericAliases: boolean): boolean | undefined {
  const normalized = value.trim().toLowerCase();
  if (normalized === "true") return true;
  if (normalized === "false") return false;
  if (allowNumericAliases && normalized === "1") return true;
  if (allowNumericAliases && normalized === "0") return false;
  return undefined;
}

export function dataGridCellEditorText(options: { value: GridCellValue | undefined; databaseType: DatabaseType | undefined; columnInfo: Pick<ColumnInfo, "data_type"> | undefined }): string {
  const value = options.value ?? null;
  if (value === null) return "";
  if (options.databaseType === "mysql" && isBlobCellColumnType(options.columnInfo?.data_type)) {
    const text = binaryCellUtf8Text(value, options.columnInfo?.data_type, options.databaseType);
    if (text !== null) return text;
  }
  {}
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function coerceMysqlBlobTextValue(options: CoerceDataGridCellValueOptions): GridCellValue | undefined {
  if (options.databaseType !== "mysql" || !isBlobCellColumnType(options.columnInfo?.data_type)) return undefined;
  const originalText = binaryCellUtf8Text(options.oldValue, options.columnInfo?.data_type, options.databaseType);
  if (originalText === null) return undefined;
  if (options.value === originalText) return options.oldValue;
  return binaryCellBytesToHexValue(new TextEncoder().encode(options.value));
}

export function dataGridCellDisplayText(options: { value: GridCellValue; databaseType: DatabaseType | undefined; columnInfo: Pick<ColumnInfo, "data_type"> | undefined }): string | undefined {
  {}
  if (typeof options.value === "string") {
    const timestampDisplay = normalizeTimestampFractionDisplayText(options.value, options.columnInfo?.data_type);
    if (timestampDisplay !== options.value) return timestampDisplay;
  }
  {}
  return undefined;
}

function normalizeTimestampFractionDisplayText(value: string, dataType: string | undefined): string {
  if (!/^timestamp(?:\s*\([^)]*\))?(?:\s+(?:with|without)\s+time\s+zone)?$/i.test(dataType?.trim() ?? "")) return value;
  const match = value.match(/^(\d{4}[-/]\d{1,2}[-/]\d{1,2}[ T]\d{1,2}:\d{1,2}:\d{1,2})\.(\d{1,2})(Z|[+-]\d{2}:?\d{2})?$/);
  return match ? `${match[1]}.${match[2].padEnd(3, "0")}${match[3] ?? ""}` : value;
}

function coercePostgresArrayValue(_options: CoerceDataGridCellValueOptions): unknown[] | undefined {
  {
    return undefined;
  }
}

function shouldPreserveNumericText(options: CoerceDataGridCellValueOptions, parsedNumber: number, text: string): boolean {
  if (!isNumericLiteralText(text)) return false;
  return shouldPreserveNumericTextForType(options.columnInfo?.data_type, text, parsedNumber);
}

function normalizeGroupedNumberText(value: string, columnInfo: Pick<ColumnInfo, "data_type"> | undefined, oldValue: GridCellValue | undefined, numberFormat: NumericInputNumberFormat): string {
  const useSampledNumberType = normalizeDataType(columnInfo?.data_type) === "" && typeof oldValue === "number";
  if (!isNumericColumnType(columnInfo?.data_type) && !useSampledNumberType) return value;
  return normalizeCommaSeparatedNumberText(value, numberFormat);
}

function normalizeCommaSeparatedNumberText(value: string, numberFormat: NumericInputNumberFormat): string {
  const trimmed = value.trim();
  const match = trimmed.match(/^([+-]?\d{1,3}(?:,\d{3})+(?:\.\d+)?)([eE][+-]?\d+)?$/);
  if (!match) return value;
  const mantissa = match[1];
  const exponent = match[2] ?? "";
  if (/^[+-]?\d{1,3},\d{3}$/.test(mantissa)) {
    if (numberFormat.groupSeparator === "," && numberFormat.decimalSeparator !== ",") {
      return `${mantissa.replace(",", "")}${exponent}`;
    }
    if (numberFormat.decimalSeparator === ",") {
      return `${mantissa.replace(",", ".")}${exponent}`;
    }
    return value;
  }
  return `${mantissa.replace(/,/g, "")}${exponent}`;
}

function runtimeNumberFormat(): NumericInputNumberFormat {
  if (cachedRuntimeNumberFormat) return cachedRuntimeNumberFormat;
  const parts = new Intl.NumberFormat().formatToParts(12345.6);
  cachedRuntimeNumberFormat = {
    groupSeparator: parts.find((part) => part.type === "group")?.value,
    decimalSeparator: parts.find((part) => part.type === "decimal")?.value,
  };
  return cachedRuntimeNumberFormat;
}

function shouldPreserveNumericTextForType(dataType: string | undefined, text: string, parsedNumber: number): boolean {
  const normalized = normalizeDataType(dataType);
  if (isExactDecimalDataType(normalized)) return true;
  if (isLargeIntegerDataType(normalized)) return !Number.isSafeInteger(parsedNumber);
  return numericTextWouldLosePrecision(text, parsedNumber);
}

function normalizeDataType(dataType: string | undefined): string {
  return (dataType ?? "").trim().toLowerCase();
}

function isExactDecimalDataType(dataType: string): boolean {
  return /\b(?:decimal|numeric|number|dec|money|smallmoney|bigdecimal|bignumeric|big_numeric|fixed)\b/.test(dataType);
}

function isLargeIntegerDataType(dataType: string): boolean {
  return /\b(?:bigint|int8|int64|uint64|u64|bigserial|serial8|int128|uint128|int256|uint256)\b/.test(dataType);
}

function numericTextWouldLosePrecision(text: string, parsedNumber: number): boolean {
  if (isIntegerLiteralText(text)) return !Number.isSafeInteger(parsedNumber);
  return significantDigitCount(text) > 15;
}

function isNumericLiteralText(text: string): boolean {
  return /^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(text);
}

function isIntegerLiteralText(text: string): boolean {
  return /^[+-]?\d+$/.test(text);
}

function significantDigitCount(text: string): number {
  const mantissa = text.replace(/^[+-]/, "").split(/[eE]/)[0].replace(".", "");
  const withoutLeadingZeros = mantissa.replace(/^0+/, "");
  return withoutLeadingZeros.length;
}

function normalizeSmartQuotedJsonInput(value: string): string {
  // Check for smart double quotes that input methods might insert.
  // U+201C, U+201D, U+201E, U+201F, U+FF02
  if (!hasSmartDoubleQuotes(value)) return value;
  const trimmed = value.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return value;

  try {
    JSON.parse(value);
    return value;
  } catch {
    // Input methods can turn JSON delimiters into smart quotes.
  }

  // Input methods (especially on macOS and with Chinese IME) can turn JSON delimiters
  // into smart quotes. Normalize them to standard ASCII quotes.
  const normalized = normalizeSmartQuotes(value);
  try {
    JSON.parse(normalized);
    return normalized;
  } catch {
    return value;
  }
}

function hasSmartDoubleQuotes(value: string): boolean {
  // Check for smart double quotes: U+201C, U+201D, U+201E, U+201F, U+FF02
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (code === 0x201c || code === 0x201d || code === 0x201e || code === 0x201f || code === 0xff02) {
      return true;
    }
  }
  return false;
}

function normalizeSmartQuotes(value: string): string {
  let result = "";
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    if (code === 0x201c || code === 0x201d || code === 0x201e || code === 0x201f || code === 0xff02) {
      // Convert to standard double quote
      result += '"';
    } else {
      result += value[i];
    }
  }
  return result;
}

function formatPostgresArrayText(value: unknown[]): string {
  return `{${value.map(formatPostgresArrayElement).join(",")}}`;
}

function formatPostgresArrayElement(value: unknown): string {
  if (Array.isArray(value)) return formatPostgresArrayText(value);
  if (value === null) return "NULL";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  const text = typeof value === "string" ? value : JSON.stringify(value);
  if (text === undefined) return "";
  if (!needsQuotedPostgresArrayElement(text)) return text;
  return `"${text.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function needsQuotedPostgresArrayElement(value: string): boolean {
  return value === "" || /[\s,"{}\\]/.test(value) || value.toUpperCase() === "NULL";
}
