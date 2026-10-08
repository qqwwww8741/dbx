import type { DatabaseType, QueryResult } from "@/types/database";
import * as api from "@/lib/backend/api";
import { supportsDatabaseFeature } from "@/lib/database/databaseDriverManifest";
import { isQueryExecutionErrorResult } from "@/lib/query/queryResultError";

export interface ExplainPlanNode {
  id: string;
  title: string;
  nodeType: string;
  /** Optional dialect hint for engine-specific operator presentation. */
  dialect?: ExplainPlanDatabaseType;
  relation?: string;
  index?: string;
  cost?: string;
  /** Suppress derived cost shares when the engine's cost accumulation is undocumented. */
  costModel?: "unknown";
  rows?: string;
  width?: string;
  estimatedTimeUs?: string;
  details: string[];
  children: ExplainPlanNode[];
}

export type ExplainPlanDatabaseType = "mysql";

export interface ParsedExplainPlan {
  databaseType: ExplainPlanDatabaseType;
  raw: unknown;
  nodes: ExplainPlanNode[];
}

export type BuildExplainSqlResult = { ok: true; sql: string } | { ok: false; reason: "unsupported" | "empty" | "unsafe" };

export function formatExplainPlanDetails(node: ExplainPlanNode | undefined, estimatedTimeLabel: string): string[] {
  if (!node) return [];
  return node.estimatedTimeUs === undefined ? node.details : [`${estimatedTimeLabel}: ${node.estimatedTimeUs} µs`, ...node.details];
}

const SUPPORTED_EXPLAIN_TYPES = new Set<DatabaseType>(["mysql"]);
export function supportsExplainPlan(databaseType?: DatabaseType): databaseType is ExplainPlanDatabaseType {
  return !!databaseType && supportsDatabaseFeature(databaseType, "sqlExplain") && SUPPORTED_EXPLAIN_TYPES.has(databaseType);
}

/** `analyze` is honored by PostgreSQL only; every other engine ignores it server-side. */
export function buildExplainSql(databaseType: DatabaseType | undefined, sql: string, format: "json" | "standard" = "json", analyze?: boolean): Promise<BuildExplainSqlResult> {
  return api.buildExplainSql({ databaseType, sql, format, analyze }) as Promise<BuildExplainSqlResult>;
}

export function parseExplainResult(databaseType: ExplainPlanDatabaseType, result: QueryResult): ParsedExplainPlan {
  {
    {
      {
        {
          {
            {
            }
          }
        }
      }
    }
  }
  const raw = parseExplainCell(result.rows[0]?.[0]);
  const nodes = parseMysqlExplain(raw);
  return { databaseType, raw, nodes };
}

/**
 * Parse DM's getExplainInfo() text output.
 * Format (flat list with indentation):
 *   1   #NSET2: [cost, rows, width]
 *   2     #PIPE2: [cost, rows, width]
 *   3       #PRJT2: [cost, rows, width]; props...
 *   ...
 *   Statistics
 *       logical reads
 *       exec time(ms)
 *
 * For autotrace, rows include ->actual: [cost, estRows->actualRows, width]
 */

/**
 * Parse a single DM operator line:
 *   #NSET2: [cost, rows, width]; props
 *   #HASH2 INNER JOIN: [cost, rows->actual, width]; KEY_NUM(1), MEM_USED(20352KB)
 *   #CSCN2: [cost, rows, width]; INDEX_NAME; btr_scan(1)
 */

export function flattenExplainPlanNodes(nodes: ExplainPlanNode[]): ExplainPlanNode[] {
  const rows: ExplainPlanNode[] = [];
  function visit(node: ExplainPlanNode) {
    rows.push(node);
    node.children.forEach((child) => visit(child));
  }
  nodes.forEach((node) => visit(node));
  return rows;
}

export function sqlServerExplainResult(results: QueryResult[]): { result?: QueryResult; error?: string } {
  const errorResult = results.find((result) => isQueryExecutionErrorResult(result) && result.rows.length > 0);
  if (errorResult) return { error: String(errorResult.rows[0]?.[0] ?? "") };

  const result = results.find((candidate) => firstSqlServerShowplanXml(candidate) !== undefined);
  return result ? { result } : { error: "SQL Server did not return ShowPlan XML" };
}

function firstSqlServerShowplanXml(result: QueryResult): string | undefined {
  for (const row of result.rows) {
    for (const cell of row) {
      if (typeof cell === "string" && cell.includes("<ShowPlanXML")) return cell;
    }
  }
  return undefined;
}

/**
 * Aggregates the `SET STATISTICS XML` runtime counters of one operator across its
 * threads: rows and executions are summed, elapsed time is wall clock per thread so
 * the slowest thread is the operator duration, and CPU time is additive work so it
 * is summed. Estimated plans carry no RunTimeInformation and yield undefined.
 */

function parseExplainCell(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

// ── DM (达梦) tabular explain parser ──────────────────────────────────

/**
 * DM's EXPLAIN returns a tabular result set.
 * Columns: EXPLAIN_ID, ID, OPERATION, OPTIONS, OBJECT_NAME, OBJECT_TYPE,
 *          COST, CARDINALITY, CPU_COST, IO_COST, etc.
 * ID is hierarchical dot-notation (1, 2, 2.1, 2.2, 3).
 */

/**
 * Build a tree from flat DM explain rows using dot-notation ID hierarchy.
 * Root nodes have IDs like "1", "2", "3".
 * Children have IDs like "1.1", "1.2", "2.1.1".
 */

// ── PostgreSQL JSON explain parser ────────────────────────────────────

// ── MySQL JSON explain parser ─────────────────────────────────────────

function parseMysqlExplain(raw: unknown): ExplainPlanNode[] {
  const root = objectValue(raw);
  if (!root) return [];
  const block = objectValue(root.query_block) || root;
  return [parseMysqlBlock(block, "0", "query_block")];
}

function parseMysqlBlock(block: Record<string, unknown>, id: string, nodeType: string): ExplainPlanNode {
  const costInfo = objectValue(block.cost_info);
  const children: ExplainPlanNode[] = [];

  const table = objectValue(block.table);
  if (table) children.push(parseMysqlTable(table, `${id}.0`));

  const nestedLoop = arrayValue(block.nested_loop);
  if (nestedLoop) {
    nestedLoop.forEach((item) => {
      const itemObject = objectValue(item);
      if (!itemObject) return;
      const nestedTable = objectValue(itemObject.table);
      if (nestedTable) {
        children.push(parseMysqlTable(nestedTable, `${id}.${children.length}`));
        return;
      }
      children.push(parseMysqlBlock(itemObject, `${id}.${children.length}`, "operation"));
    });
  }

  ["ordering_operation", "grouping_operation", "duplicates_removal", "union_result", "materialized_from_subquery"].forEach((key) => {
    const child = objectValue(block[key]);
    if (child) children.push(parseMysqlBlock(child, `${id}.${children.length}`, key));
  });

  return {
    id,
    title: nodeType,
    nodeType,
    cost: stringValue(costInfo?.query_cost),
    rows: numberLike(block.select_id),
    details: [stringValue(block.message)].filter(nonEmptyString),
    children,
  };
}

function parseMysqlTable(table: Record<string, unknown>, id: string): ExplainPlanNode {
  const relation = stringValue(table.table_name);
  const accessType = stringValue(table.access_type) || "table";
  const costInfo = objectValue(table.cost_info);
  const rows = numberLike(table.rows_examined_per_scan) || numberLike(table.rows_produced_per_join);
  const cost = stringValue(costInfo?.query_cost) || stringValue(costInfo?.read_cost) || stringValue(costInfo?.eval_cost);
  const details = [stringValue(table.attached_condition) ? `Condition: ${stringValue(table.attached_condition)}` : "", arrayValue(table.used_columns)?.length ? `Columns: ${arrayValue(table.used_columns)!.map(String).join(", ")}` : "", table.using_index === true ? "Using index" : ""].filter(Boolean);

  return {
    id,
    title: relation ? `${accessType} on ${relation}` : accessType,
    nodeType: accessType,
    relation,
    index: stringValue(table.key),
    cost,
    rows,
    details,
    children: [],
  };
}

// ── QuestDB explain parser ─────────────────────────────────────────

// ── Doris explain parser ────────────────────────────────────────────

// ── Helpers ───────────────────────────────────────────────────────────

function objectValue(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function arrayValue(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null;
}

function stringValue(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return undefined;
}

function nonEmptyString(value: string | undefined): value is string {
  return !!value;
}

function numberLike(value: unknown): string | undefined {
  if (typeof value === "number") return Number.isInteger(value) ? String(value) : String(value);
  if (typeof value === "string" && value.trim()) return value;
  return undefined;
}
