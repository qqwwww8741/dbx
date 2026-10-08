import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import type { MongoDumpFormat, MongoDumpSourceInput, MongoDumpCatalog, MongoRestoreSourcePreview, MongoDatabaseDumpRequest, MongoDatabaseRestoreRequest, MongoDatabaseDumpProgress } from "./mongodbDumpTypes";
import type { MongoRestoreUpload, MongoSourceReadOptions } from "./mongodbDumpTypes";
import type { UserSkillRootSettings, UserSkillsListResult, UserSkillsReadResult } from "@/types/userSkills";
import type { DatabaseBackupCommand, DatabaseBackupBackgroundStatus } from "@/lib/backup/backgroundDatabaseBackup";

export function databaseBackupCommand<T = unknown>(command: DatabaseBackupCommand): Promise<T> {
  return invoke("database_backup_command", { command });
}

export function databaseBackupBackground(enabled?: boolean): Promise<DatabaseBackupBackgroundStatus> {
  return invoke("database_backup_background", { enabled });
}

export async function downloadDatabaseBackupFile(_runId: string, _index: number): Promise<void> {
  throw new Error("Use the file manager to access desktop backup files");
}

export function prepareDatabaseBackupRestore(id: string, index: number): Promise<string | SqlFilePreview> {
  return invoke("database_backup_command", { command: { action: "file", id, index } });
}
import { assertUpdateAllowsCommand } from "@/lib/app/updatePreparation";
import { collectBrowserSupportInfo } from "@/lib/app/supportInfo";
// Re-exported below so the HTTP transport shares one definition; imported here
// for this module's own signatures (a re-export does not bind local names).
import type { PluginPlanCapabilities, PluginPlanRequest, PluginPlanResult } from "@/types/pluginPlan";
import type { PluginTableMetadata, PluginTableMetadataRequest } from "@/types/pluginSchemaMetadata";
import type { PluginDataGrant, PluginDataQueryRequest, PluginDataQueryResult } from "@/types/pluginData";
import type { AiToolApprovalOutcome, PluginToolPreview } from "@/types/pluginAiTools";

function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  assertUpdateAllowsCommand(command);
  return tauriInvoke<T>(command, args);
}

import type { MigrationPreflight, MigrationReport } from "./migration";
export type { MigrationPreflight, MigrationReport } from "./migration";
export const migrationStatus = (retry = false): Promise<MigrationPreflight> => invoke("migration_status", { retry });
export const migrationStart = (): Promise<MigrationReport> => invoke("migration_start");
export const migrationRetry = (): Promise<MigrationReport> => invoke("migration_retry");
export const migrationCleanupBackups = (): Promise<void> => invoke("migration_cleanup_backups");
import type { DetachedTabHandoff } from "@/lib/app/detachedTabHandoff";
import { BackendErrorException, type BackendError } from "@/lib/backend/errorUtils";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ExternalSqlFileTooLargeError } from "@/lib/sql/sqlFileOpen";
import { appendDebugLog, isDebugLoggingEnabled } from "@/lib/backend/debugLog";

import type { XuguTablespaceInfo } from "@/types/database";

import type { CsvQuoteMode } from "@/lib/export/csvQuoteMode";
import type { SqlExportColumnSelection, SqlInsertDialect, SqlInsertMode } from "@/lib/export/sqlInsertMode";

/** Normalize Tauri rejections once at the public backend boundary. */
async function invokeBackend<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw error instanceof BackendErrorException ? error : new BackendErrorException(error);
  }
}
import type {
  ConnectionConfig,
  ConnectionTestResult,
  DatabaseConnectionInfo,
  DatabaseInfo,
  DatabaseStorageInfo,
  SqlServerCompletionContext,
  SchemaInfo,
  LinkedServerInfo,
  CatalogInfo,
  TableInfo,
  ObjectInfo,
  CompletionAssistantRequest,
  CompletionAssistantResponse,
  ObjectStatistics,
  CustomTypeDetails,
  ObjectSource,
  ObjectSourceKind,
  MysqlEventInfo,
  ColumnInfo,
  SqlServerColumnMetadata,
  IndexInfo,
  ReferenceKeyInfo,
  ForeignKeyInfo,
  TriggerInfo,
  ConstraintInfo,
  PartitionInfo,
  SubpartitionInfo,
  FunctionInfo,
  SequenceInfo,
  RuleInfo,
  OwnerInfo,
  ExtensionInfo,
  EventTriggerInfo,
  QueryResult,
  SqlReferenceAnalysis,
  DatabaseType,
  InstalledPlugin,
  JdbcDriverInfo,
  JdbcLocalBundleInfo,
  JdbcMavenBundleInfo,
  JdbcPluginStatus,
  SavedSqlFile,
  SavedSqlFolder,
  SavedSqlLibrary,
  SshConfigHostEntry,
  LocalSshKey,
  TunnelProfile,
  TransactionLog,
  ExternalSqlFileVersion,
} from "@/types/database";
import { isTauriCommandUnavailable, normalizeConnectionTestResult } from "@/lib/connection/connectionDatabaseInfo";
import type { AnnotationFile, SchemaSnapshot } from "@/docs/types";
import type { CollectionInfo } from "@/types/database";
import type { SidebarObjectKind } from "@/lib/database/databaseObjectCapabilities";
import type { AiChatSelectionState, AiConfig, AiConfigItem, AiEffortCapability, AiEffortLevel, AiTestConnectionResult } from "@/types/ai";

import type { QueryEditability } from "@/lib/sql/sqlAnalysis";
import { isTerminalTransferProgress } from "@/lib/backend/transferProgress";
import type {
  ActivePluginSession,
  ConnectionLivenessMessage,
  PluginBinaryEvent,
  PluginConnectionActionResult,
  PluginEvent,
  PluginFilesystemListResult,
  PluginFilesystemMutationResult,
  PluginFilesystemReadResult,
  PluginInstallResult,
  PluginMarketplaceInstallRequest,
  PluginRepository,
  PluginRepositoryCatalogResult,
  PluginRollbackResult,
  PluginTrustedKey,
  PluginUiAssetPayload,
  TableVGroupLayout,
} from "@/types/database";
import type {
  DataGridColumnDistinctValuesSqlOptions,
  DataGridColumnValueFilterConditionOptions,
  DataGridColumnValuesFilterConditionOptions,
  DataGridContextFilterConditionOptions,
  DataGridConditionalUpdateSqlOptions,
  DataGridCountSqlOptions,
  DataGridCopyInsertStatementOptions,
  DataGridCopyUpdateStatementOptions,
  DataGridSaveStatementOptions,
  HiveTablePropertiesSqlOptions,
} from "@/lib/dataGrid/dataGridSql";
import type { DmlChangePreviewSqlOptions, DmlChangePreviewSqlResult } from "@/lib/sql/dmlChangePreview";
import type { DataGridExtractRequest, DataGridExtractResult } from "@/lib/dataGrid/dataGridCopyExtractor";
import type { DataCompareFromTablesOptions, DataCompareFromTablesPreparation, DataCompareSyncPlan, DataCompareSyncPlanOptions, DataComparePreparation, DataComparePreparationOptions } from "@/lib/dataGrid/dataCompare";
import type { SchemaDiffPreparation, SchemaDiffPreparationOptions, SchemaSyncSqlPlan, SelectedSchemaDiffInput, GenerateSchemaSyncPlanOptions, TableDiff, FunctionDiff, SequenceDiff, RuleDiff, OwnerDiff } from "@/lib/schema/schemaDiff";
import type { BuildCreatePartitionedTableSqlOptions, BuildTableOwnerChangeSqlOptions, BuildTableStructureChangeSqlOptions, BuildSingleColumnAlterSqlOptions, SqliteTableStructureChangePreview, TablePartitionSqlOptions, TableStructureChangeSql } from "@/lib/table/tableStructureEditorSql";
import type { BuildTableSelectSqlOptions } from "@/lib/table/tableSelectSql";
import type { DatabaseSearchSql, DatabaseSearchSqlOptions, SearchResultWhereOptions } from "@/lib/database/databaseSearch";
import type { BuildEditableObjectSourceSqlInput, BuildRoutineRenameObjectSourceInput } from "@/lib/table/objectSourceEditor";
import type { BuildViewDdlInput } from "@/lib/table/viewDdl";
import type { BuildRenameObjectSqlOptions } from "@/lib/table/objectRenameSql";
import type { CreateDatabaseSqlOptions } from "@/lib/database/createDatabaseSql";
import type {
  DatabaseNameSqlOptions,
  DatabasePropertyEditSqlOptions,
  DropTableChildObjectSqlOptions,
  DropObjectSqlOptions,
  DuplicateTableStructureSqlOptions,
  CopyTableDataSqlOptions,
  MysqlAutoIncrementSqlOptions,
  SchemaNameSqlOptions,
  TableAdminSqlOptions,
  VacuumTableSqlOptions,
} from "@/lib/database/dbAdminSql";
import type { BuildDatabaseSqlExportOptions, BuildExportInsertStatementsOptions, BuildExportSqlInsertOptions } from "@/lib/export/databaseExport";

export interface SshPromptResolution {
  id: string;
  action: "accept" | "reject" | "secret";
  remember?: boolean;
  secret?: string;
}

export interface AgentDriverInfo {
  db_type: string;
  label: string;
  version: string;
  size: number;
  installed: boolean;
  installed_version: string | null;
  update_available: boolean;
  requires_java_runtime?: boolean;
  jre: string;
  jre_installed: boolean;
}

export interface AgentDriverUpdateIssue {
  db_type: string;
  error: string;
}

export interface UpgradeAllAgentDriversResult {
  upgraded: number;
  /** Drivers whose install was aborted by a user cancel (single-driver or batch). */
  cancelled: number;
  failed: AgentDriverUpdateIssue[];
}

export interface AgentUpdateBlocker {
  db_type: string;
  label: string;
  connections: string[];
}

export type AgentOfflineArtifactKind = "jar" | "native";

export type AgentOfflineExportUnavailableReason = "unmanagedInstall" | "localInstall" | "launchConfig" | "missingArtifact" | "invalidArtifact" | "unsafeSource" | "externalDriverRequired" | "missingManagedJre" | "invalidManagedJre";

export interface AgentOfflineExportCandidate {
  dbType: string;
  label: string;
  version: string;
  size: number;
  artifactKind: AgentOfflineArtifactKind | null;
  requiredJre: string | null;
  eligible: boolean;
  unavailableReason: AgentOfflineExportUnavailableReason | null;
}

export interface AgentOfflineExportPreview {
  platform: string;
  candidates: AgentOfflineExportCandidate[];
}

export interface AgentOfflineExportResult {
  platform: string;
  driverCount: number;
  jreCount: number;
  bytes: number;
}

export interface AgentOfflineImportResult {
  count: number;
  jreCount: number;
  /** Items the package could not install; the rest of the import still ran. */
  failures: AgentOfflineImportFailure[];
}

export interface AgentOfflineImportFailure {
  /** Managed JRE key (e.g. "21") or driver key (e.g. "oracle"). */
  key: string;
  /** True when the failed item is a managed JRE runtime rather than a driver. */
  is_jre: boolean;
  /** Failure text, including the underlying OS error when there is one. */
  error: string;
}

export type JavaRuntimeMode = "managed" | "system" | "custom";

export interface JavaRuntimeConfig {
  mode: JavaRuntimeMode;
  custom_java_path: string | null;
}

export interface DriverStoreUsageItem {
  id: string;
  bytes: number;
}

export interface DriverStoreUsage {
  total_bytes: number;
  jre_bytes: number;
  agent_driver_bytes: number;
  download_cache_bytes?: number;
  jdbc_plugin_bytes: number;
  jdbc_driver_bytes: number;
  jres: DriverStoreUsageItem[];
  agent_drivers: DriverStoreUsageItem[];
}

export type DriverRuntimeHealth = "healthy" | "warning" | "error";
export type DriverRuntimeStatus = "running" | "stopped" | "error" | "unknown";

export interface DriverRuntimeInfo {
  id: string;
  driver_key: string;
  label: string;
  kind: string;
  source: string;
  status: DriverRuntimeStatus;
  pid: number | null;
  memory_bytes: number | null;
  cpu_percent: number | null;
  uptime_seconds: number | null;
  version: string | null;
  last_error: string | null;
  can_stop: boolean;
  can_restart: boolean;
  control_unavailable_reason: string | null;
  protocol_mode: "multi_session" | "legacy" | null;
  active_sessions: number | null;
}

export interface DriverRuntimeSummary {
  running_count: number;
  total_memory_bytes: number;
  last_error: string | null;
  health: DriverRuntimeHealth;
  runtimes: DriverRuntimeInfo[];
}

export interface DesktopSettings {
  show_tray_icon: boolean;
  icon_theme: "default" | "black";
  quit_on_close: boolean;
  close_action_prompted: boolean;
  debug_logging_enabled: boolean;
  metadata_cache_max_memory_mb: number;
  duckdb_worker_process_isolation: boolean;
  duckdb_worker_max_processes: number;
  saved_sql_sync_dir?: string | null;
  driver_store_dir?: string | null;
  plugin_store_dir?: string | null;
  agent_store_dir?: string | null;
  custom_ai_skill_root_enabled?: boolean | null;
  custom_ai_skill_root?: string | null;
  sidebar_table_page_size?: number | null;
}

export interface McpGlobalPolicy {
  readOnly: boolean;
  allowDangerousSql: boolean;
  allowedConnectionIds: string[] | null;
  allowedGroupIds: string[];
  allowedToolNames: string[] | null;
  connectionPolicies: McpConnectionPolicy[];
  groupPolicies: McpGroupPolicy[];
  configured: boolean;
  queryTimeoutSecs: number | null;
}

export interface McpGroupPolicy {
  groupId: string;
  readOnly: boolean;
  allowDangerousSql: boolean;
}

export interface McpConnectionPolicy {
  connectionId: string;
  readOnly: boolean;
  allowDangerousSql: boolean;
  executionModeConfigured: boolean;
  executionModePolicyVersion: number | null;
  databaseScope: "all" | "selected" | "none";
  allowedDatabases: string[];
  databasePolicies: McpDatabasePolicy[];
  /** Opt-in for AI-agent DML against a Salesforce org; forced off by `readOnly`. */
  allowSalesforceDml: boolean;
}

export interface McpDatabasePolicy {
  databaseName: string;
  readOnly: boolean;
  allowDangerousSql: boolean;
}

export interface SavedSqlSyncEntry {
  folderName?: string;
  fileName: string;
  sql: string;
}

export interface SavedSqlSyncRequest {
  targetDir: string;
  entries: SavedSqlSyncEntry[];
}

export interface WebDavConfig {
  endpoint: string;
  username?: string;
  password?: string;
  remotePath?: string;
}

export interface WebDavSyncSummary {
  remotePath: string;
  bytes: number;
  exportedAt?: string;
  appVersion?: string;
}

export interface SyncCatalogItem {
  id: string;
  label: string;
}

export interface PluginUiStorageItemRef {
  pluginId: string;
  key: string;
  pluginName?: string;
}

export interface SyncSelection {
  connections?: string[];
  connectionSecrets?: string[];
  tunnelProfiles?: string[];
  tunnelSecrets?: string[];
  savedSqlFolders?: string[];
  savedSqlFiles?: string[];
  desktopSettings?: string[];
  editorSettings?: string[];
  aiConfigs?: string[];
  pluginUiStorage?: PluginUiStorageItemRef[];
  sidebarLayout?: boolean;
  pinnedTreeNodeIds?: boolean;
  includeSecrets: boolean;
  syncCredentials: boolean;
}

export interface SyncSnapshotCatalog {
  exportedAt: string;
  appVersion: string;
  hasEncryptedSecrets: boolean;
  connections: SyncCatalogItem[];
  connectionSecrets: string[];
  tunnelProfiles: SyncCatalogItem[];
  tunnelSecrets: string[];
  savedSqlFolders: SyncCatalogItem[];
  savedSqlFiles: SyncCatalogItem[];
  desktopSettings: SyncCatalogItem[];
  editorSettings: SyncCatalogItem[];
  aiConfigs: SyncCatalogItem[];
  aiConfigsLocked: boolean;
  pluginUiStorage: PluginUiStorageItemRef[];
  pluginUiStorageLocked: boolean;
  hasSidebarLayout: boolean;
  hasPinnedTreeNodeIds: boolean;
  selection?: SyncSelection;
}

export interface WebDavDownloadResult {
  summary: WebDavSyncSummary;
  editorSettings?: unknown;
  desktopSettings: DesktopSettings;
  applySummary: {
    encryptedSecretsPresent: boolean;
    secretsApplied: boolean;
  };
}

export interface LocalBackupImportResult {
  editorSettings?: unknown;
  desktopSettings: DesktopSettings;
  applySummary: WebDavDownloadResult["applySummary"];
}

export interface LocalBackupExportSummary {
  bytes: number;
}

export interface WebDavPasswordStatus {
  hasSavedPassword: boolean;
}

export interface WebDavSyncSecretsStatus {
  enabled: boolean;
  hasSavedPassphrase: boolean;
}

export type SnippetProvider = "github" | "gitee" | "gitlab";

export interface SnippetSyncConfig {
  provider: SnippetProvider;
  instanceUrl?: string;
  token?: string;
  snippetId?: string;
  replaceLegacySnippet?: boolean;
}

export interface SnippetSyncSettings {
  snippetId?: string;
  legacyCleanupRequiredId?: string;
}

export interface SnippetSyncSummary {
  provider: SnippetProvider;
  snippetId: string;
  bytes: number;
  exportedAt?: string;
  appVersion?: string;
  legacyCleanupRequiredId?: string;
}

export interface SnippetDownloadResult {
  summary: SnippetSyncSummary;
  editorSettings?: unknown;
  desktopSettings: DesktopSettings;
  applySummary: WebDavDownloadResult["applySummary"];
}

export interface SnippetTokenStatus {
  hasSavedToken: boolean;
}

export interface AppSupportInfo {
  appVersion: string;
  runtime: "desktop" | "web";
  osName: string;
  osVersion?: string | null;
  arch: string;
  userAgent?: string;
  databaseTypes?: string[];
  localDriverVersions?: Array<{ dbType: string; version: string }>;
  aiProviders?: string[];
}

export interface QueryPagination {
  limit: number;
  offset: number;
  sessionId?: string;
}

export interface QueryPaginationExecutionPlanOptions {
  sql: string;
  queryBaseSql: string;
  databaseType?: DatabaseType;
  pagination: QueryPagination;
  useAgentCursor: boolean;
  firstPageUsesActualSql?: boolean;
}

export interface QueryPaginationExecutionPlan {
  sqlToExecute: string;
  pageSql?: string;
  pageLimit?: number;
  pageOffset?: number;
  countSql?: string;
  exactQueryRowBound?: number;
  useAgentResultSession: boolean;
  paginationRowNumberColumn?: string;
  paginationError?: string;
}

export type QuerySortDirection = "asc" | "desc";

export interface SortedQuerySqlOptions {
  originalSql: string;
  databaseType?: DatabaseType;
  resultColumns: string[];
  columnIndex: number;
  column: string;
  direction: QuerySortDirection;
}

export interface QuerySqlBuildResult {
  ok: boolean;
  sql?: string;
  reason?: "empty" | "multi" | "not_select" | "unsupported" | "with";
}

export interface BuildExplainSqlOptions {
  databaseType?: DatabaseType;
  sql: string;
  /** MySQL can return either the existing JSON plan or its native tabular plan. */
  format?: "json" | "standard";
  /** PostgreSQL only: run the statement so the plan carries measured rows and timings. */
  analyze?: boolean;
}

export interface ExplainSqlBuildResult {
  ok: boolean;
  sql?: string;
  reason?: "unsupported" | "empty" | "unsafe";
}

export interface DroppedFilePreviewSqlOptions {
  path: string;
  limit?: number;
}

export type XlsxCellValue = string | number | boolean | null;

export interface DriverInstallProgress {
  operation_id?: string;
  step: string;
  downloaded?: number;
  total?: number;
  db_type?: string;
  current?: number;
  total_drivers?: number;
}

export interface AiMessage {
  role: "user" | "assistant" | "system";
  content: string;
  /** Transient images for this message. Persisted conversation history intentionally omits them. */
  images?: Array<{
    mediaType: string;
    data: string;
  }>;
}

export interface AiTaskContract {
  action?: string;
  mode?: string;
  userRequest?: string;
}

export interface AiCompletionRequest {
  config: AiConfig;
  systemPrompt: string;
  messages: AiMessage[];
  taskContract?: AiTaskContract;
  maxTokens?: number;
  /** Stable per-conversation key used by the Responses API prompt cache. */
  promptCacheKey?: string;
}

export interface AiModelInfo {
  id: string;
  displayName?: string;
  supportedEffortLevels?: AiEffortLevel[];
  effortCapability?: AiEffortCapability;
}

export async function aiComplete(request: AiCompletionRequest): Promise<string> {
  return invoke("ai_complete", { request });
}

export interface AiStreamChunk {
  session_id: string;
  delta: string;
  reasoning_delta?: string;
  done: boolean;
  /** Web-only explicit terminal error; Tauri reports invoke failures directly. */
  error?: string;
}

export async function aiStream(sessionId: string, request: AiCompletionRequest, onChunk: (chunk: AiStreamChunk) => void): Promise<void> {
  const unlisten: UnlistenFn = await listen<AiStreamChunk>("ai-stream-chunk", (event) => {
    if (event.payload.session_id === sessionId) {
      onChunk(event.payload);
      if (event.payload.done) unlisten();
    }
  });
  try {
    await invoke("ai_stream", { sessionId, request });
  } catch (e) {
    unlisten();
    throw e;
  }
}

export type AgentEvent =
  | { type: "turn_start"; turn: number }
  | { type: "text_delta"; delta: string }
  | { type: "write_sql_confirmation_required"; sql: string }
  | { type: "production_write_blocked"; sql: string }
  | { type: "reasoning_delta"; delta: string }
  | {
      type: "tool_call_start";
      tool_call_id: string;
      tool_name: string;
      args: Record<string, unknown>;
    }
  | {
      /** A plugin tool call that may change state waits for the user's approval; the run is paused. */
      type: "tool_approval_required";
      approval_id: string;
      tool_call_id: string;
      tool_name: string;
      plugin_id: string;
      plugin_name: string;
      plugin_tool: string;
      connection_id: string;
      connection_name: string;
      /** Exactly the arguments DBX forwards if the user approves. */
      args: Record<string, unknown>;
      timeout_secs: number;
    }
  | { type: "tool_approval_resolved"; approval_id: string; tool_call_id: string; outcome: AiToolApprovalOutcome }
  | {
      type: "tool_call_end";
      tool_call_id: string;
      tool_name: string;
      result: unknown;
      is_error: boolean;
    }
  | { type: "turn_end"; turn: number }
  | {
      /**
       * The reply stream is fully consumed but the run is NOT yet confirmed
       * successful — the CLI process may still exit non-zero or hang after
       * closing stdout. Non-terminal: the UI may stop the reply animation on
       * it, but must keep listening for the real `agent_end` (success) /
       * `error` (failure).
       */
      type: "response_complete";
    }
  | { type: "agent_end"; input_tokens?: number; output_tokens?: number }
  | {
      type: "context_compacted";
      summary: string;
      summary_tokens: number;
      compacted_messages: number;
      estimated_before: number;
      estimated_after: number;
    }
  | { type: "error"; message: string };

type TauriAgentEvent = AgentEvent & {
  session_id?: string;
};

export async function aiAgentStream(
  sessionId: string,
  request: AiCompletionRequest,
  connectionId: string,
  database: string,
  schema: string | undefined,
  dbType: string,
  onEvent: (event: AgentEvent) => void,
  mode?: string,
  allowWriteSql = false,
  confirmedWriteSql?: string,
  confirmedConnectionId?: string,
  confirmedDatabase?: string,
  confirmedSchema?: string,
  _signal?: AbortSignal,
  selectedDatabases?: string[],
): Promise<string> {
  const unlisten: UnlistenFn = await listen<TauriAgentEvent>("ai-agent-event", (event) => {
    const payload = event.payload;
    if (payload.session_id && payload.session_id !== sessionId) return;
    onEvent(payload);
    if (payload.type === "agent_end" || payload.type === "error") {
      unlisten();
    }
  });
  try {
    return await invoke("ai_agent_stream", {
      sessionId,
      request,
      connectionId,
      database,
      schema,
      dbType,
      mode,
      allowWriteSql,
      confirmedWriteSql,
      confirmedConnectionId,
      confirmedDatabase,
      confirmedSchema,
      selectedDatabases,
    });
  } catch (e) {
    unlisten();
    throw e;
  }
}

export async function saveAiConfig(config: AiConfig): Promise<void> {
  return invoke("save_ai_config", { config });
}

export async function saveAiProviderConfig(provider: string, config: AiConfig): Promise<void> {
  return invoke("save_ai_provider_config", { provider, config });
}

export async function loadAiProviderConfigs(): Promise<Record<string, AiConfig>> {
  return invoke("load_ai_provider_configs");
}

export async function aiTestConnection(config: AiConfig): Promise<AiTestConnectionResult> {
  return invoke("ai_test_connection", { config });
}

export async function aiListModels(config: AiConfig): Promise<AiModelInfo[]> {
  return invoke("ai_list_models", { config });
}

export async function aiResolveModelEffort(config: AiConfig, modelId: string): Promise<AiEffortCapability> {
  return invoke("ai_resolve_model_effort", { config, modelId });
}

export async function saveAiChatSelection(selection: AiChatSelectionState): Promise<void> {
  return invoke("save_ai_chat_selection", { selection });
}

export async function loadAiChatSelection(): Promise<AiChatSelectionState | null> {
  return invoke("load_ai_chat_selection");
}

export async function aiCancelStream(sessionId: string): Promise<boolean> {
  return invoke("ai_cancel_stream", { sessionId });
}

/** Answers a pending plugin tool approval of the agent run `sessionId`; false when nothing was waiting. */
export async function resolveAiToolApproval(sessionId: string, approvalId: string, approved: boolean): Promise<boolean> {
  return invoke("ai_resolve_tool_approval", { sessionId, approvalId, approved });
}

/** Plugin ids whose MCP tools the built-in AI agent may call. */
export async function getAiPluginToolPlugins(): Promise<string[]> {
  return invoke("get_ai_plugin_tool_plugins");
}

export async function setAiPluginToolPluginEnabled(pluginId: string, enabled: boolean): Promise<string[]> {
  return invoke("set_ai_plugin_tool_plugin_enabled", { pluginId, enabled });
}

/** Tools the built-in AI would get from a plugin (may start its sidecar). */
export async function previewPluginAiTools(pluginId: string): Promise<PluginToolPreview> {
  return invoke("preview_plugin_ai_tools", { pluginId });
}

export async function saveAiConfigs(configs: AiConfigItem[]): Promise<void> {
  return invoke("save_ai_configs", { configs });
}

export async function loadAiConfigs(): Promise<AiConfigItem[]> {
  return invoke("load_ai_configs");
}

export async function setDefaultAiConfig(configId: string): Promise<void> {
  return invoke("set_default_ai_config", { configId });
}

export async function saveAiConfigItem(config: AiConfigItem): Promise<void> {
  return invoke("save_ai_config_item", { config });
}

export async function deleteAiConfig(configId: string): Promise<void> {
  return invoke("delete_ai_config", { configId });
}

export async function loadAiConfig(): Promise<AiConfig | null> {
  return invoke("load_ai_config");
}

export async function loadDesktopSettings(): Promise<DesktopSettings> {
  return invoke("load_desktop_settings");
}

export async function saveDesktopSettings(settings: DesktopSettings): Promise<void> {
  return invoke("save_desktop_settings", { settings });
}

export async function loadMcpGlobalPolicy(): Promise<McpGlobalPolicy> {
  return invoke("load_mcp_global_policy");
}

export async function saveMcpGlobalPolicy(policy: Omit<McpGlobalPolicy, "configured">): Promise<void> {
  return invoke("save_mcp_global_policy", { policy });
}

export interface McpHttpServerSettings {
  enabled: boolean;
  host: string;
  port: number;
  path: string;
  allowRemote: boolean;
  allowedHosts: string[];
  allowedOrigins: string[];
}

export interface McpHttpServerStatus {
  enabled: boolean;
  running: boolean;
  endpoint: string | null;
  accessToken: string | null;
  lastError: string | null;
  recentLogs: string[];
}

export interface WebMcpHttpStatus {
  enabled: boolean;
  endpointPath: string;
  tokenSource: "environment" | "file" | "managed" | null;
  allowedHosts: string[];
  allowedOrigins: string[];
  deploymentManaged: boolean;
  managementAvailable: boolean;
  accessToken: string | null;
}

export interface WebMcpHttpSettings {
  enabled: boolean;
  allowedHosts: string[];
  allowedOrigins: string[];
}

export async function loadMcpHttpServerSettings(): Promise<McpHttpServerSettings> {
  return invoke("load_mcp_http_server_settings");
}

export async function saveMcpHttpServerSettings(settings: McpHttpServerSettings): Promise<McpHttpServerStatus> {
  return invoke("save_mcp_http_server_settings", { settings });
}

export async function mcpHttpServerStatus(): Promise<McpHttpServerStatus> {
  return invoke("mcp_http_server_status");
}

export async function rotateMcpHttpServerToken(): Promise<McpHttpServerStatus> {
  return invoke("rotate_mcp_http_server_token");
}

export async function loadWebMcpHttpStatus(): Promise<WebMcpHttpStatus> {
  return { enabled: false, endpointPath: "/mcp", tokenSource: null, allowedHosts: [], allowedOrigins: [], deploymentManaged: false, managementAvailable: false, accessToken: null };
}

export async function saveWebMcpHttpSettings(_settings: WebMcpHttpSettings): Promise<WebMcpHttpStatus> {
  throw new Error("Web MCP settings are available only in DBX Web");
}

export async function rotateWebMcpToken(): Promise<WebMcpHttpStatus> {
  throw new Error("Web MCP settings are available only in DBX Web");
}

export async function loadMaxAgentTurns(): Promise<number> {
  return invoke("load_max_agent_turns");
}

export async function loadSqlFileUploadMaxBytes(): Promise<number> {
  return 200 * 1024 * 1024;
}

export async function saveSqlFileUploadMaxMb(_sqlFileUploadMaxMb: number): Promise<void> {
  // No-op on desktop: SQL files are streamed directly from disk, no server upload cap applies.
}

export async function saveMaxAgentTurns(maxAgentTurns: number): Promise<void> {
  return invoke("save_max_agent_turns", { maxAgentTurns });
}

export async function loadHistoryRetentionLimit(): Promise<number> {
  return invoke("load_history_retention_limit");
}

export async function saveHistoryRetentionLimit(limit: number): Promise<void> {
  return invoke("save_history_retention_limit", { limit });
}

export async function loadMcpHistoryRetentionLimit(): Promise<number> {
  return invoke("load_mcp_history_retention_limit");
}

export async function saveMcpHistoryRetentionLimit(limit: number): Promise<void> {
  return invoke("save_mcp_history_retention_limit", { limit });
}

export async function loadMaxRetries(): Promise<number> {
  return invoke("load_max_retries");
}

export async function saveMaxRetries(maxRetries: number): Promise<void> {
  return invoke("save_max_retries", { maxRetries });
}

export type { OpenTabsStatePayload, PersistedEditorGroup } from "@/lib/app/openTabsPersistence";
/** Shared with `@/lib/plugins/pluginHostBridge`; re-exported so the HTTP transport reuses one definition. */
export type { PluginPlanCapabilities, PluginPlanRequest, PluginPlanResult } from "@/types/pluginPlan";
export type { PluginColumnMetadata, PluginMetadataFieldAvailability, PluginMetadataFieldCapabilities, PluginTableMetadata, PluginTableMetadataRequest } from "@/types/pluginSchemaMetadata";
import type { OpenTabsStatePayload } from "@/lib/app/openTabsPersistence";
import { uuid } from "@/lib/common/utils";

export async function loadEditorSettings(): Promise<unknown | null> {
  return invoke("load_editor_settings");
}

export async function saveEditorSettings(settings: unknown): Promise<void> {
  return invoke("save_editor_settings", { settings });
}

export interface GlobalSearchSettings {
  roots: string[];
  extensions: string[];
}

export function loadGlobalSearchSettings(): Promise<GlobalSearchSettings | null> {
  return invoke("load_global_search_settings");
}

export function saveGlobalSearchSettings(settings: GlobalSearchSettings): Promise<void> {
  return invoke("save_global_search_settings", { settings });
}

export interface BackgroundImageInfo {
  storedPath: string;
  fileName: string;
}

export async function saveBackgroundImage(sourcePath: string): Promise<BackgroundImageInfo> {
  return invoke("save_background_image", { sourcePath });
}

export async function clearBackgroundImage(storedPath: string): Promise<void> {
  return invoke("clear_background_image", { storedPath });
}

export async function readBackgroundImage(storedPath: string): Promise<string> {
  return invoke("read_background_image", { storedPath });
}

export async function checkBackgroundImage(storedPath: string): Promise<boolean> {
  return invoke("check_background_image", { storedPath });
}

export async function loadOpenTabsState(): Promise<OpenTabsStatePayload | null> {
  return invoke("load_open_tabs_state");
}

export async function saveOpenTabsState(payload: OpenTabsStatePayload): Promise<void> {
  return invoke("save_open_tabs_state", { payload });
}

export async function saveDetachedTabHandoff(tabId: string, handoff: DetachedTabHandoff): Promise<void> {
  return invoke("save_detached_tab_handoff", { tabId, handoff });
}

export async function loadDetachedTabHandoff(tabId: string): Promise<DetachedTabHandoff | null> {
  return invoke("load_detached_tab_handoff", { tabId });
}

export async function listDetachedTabHandoffs(): Promise<DetachedTabHandoff[]> {
  return invoke("list_detached_tab_handoffs");
}

export async function deleteDetachedTabHandoff(tabId: string): Promise<void> {
  return invoke("delete_detached_tab_handoff", { tabId });
}

export async function approveDetachedWindowClose(): Promise<void> {
  return invoke("approve_detached_window_close");
}

export async function loadSavedSqlEditorPositions(): Promise<unknown[] | null> {
  return invoke("load_saved_sql_editor_positions");
}

export async function saveSavedSqlEditorPositions(positions: unknown[]): Promise<void> {
  return invoke("save_saved_sql_editor_positions", { positions });
}

export async function loadTransferTaskLibrary(): Promise<unknown | null> {
  return invoke("load_transfer_task_library");
}

export async function saveTransferTaskLibrary(library: unknown): Promise<void> {
  return invoke("save_transfer_task_library", { library });
}

export async function completeAppClose(action: "quit" | "hide"): Promise<void> {
  return invoke("complete_app_close", { action });
}

export async function requestAppClose(): Promise<void> {
  return invoke("request_app_close_from_window_controls");
}

export interface DriverStoreMigrationResult {
  driver_store_dir: string | null;
  plugin_store_dir: string | null;
  agent_store_dir: string | null;
  plugins_dir: string;
  agents_dir: string;
  migrated_plugins: boolean;
  migrated_agents: boolean;
}

export async function setDriverStoreDir(newDir: string | null): Promise<DriverStoreMigrationResult> {
  return invoke("set_driver_store_dir", { newDir });
}

export async function setPluginStoreDir(newDir: string | null): Promise<DriverStoreMigrationResult> {
  return invoke("set_plugin_store_dir", { newDir });
}

export async function setAgentStoreDir(newDir: string | null): Promise<DriverStoreMigrationResult> {
  return invoke("set_agent_store_dir", { newDir });
}

export interface DriverStorePathInfo {
  driver_store_dir: string | null;
  plugin_store_dir: string | null;
  agent_store_dir: string | null;
  plugins_dir: string;
  agents_dir: string;
}

export async function getDriverStorePath(): Promise<DriverStorePathInfo> {
  return invoke("get_driver_store_path");
}

export async function webdavSyncTest(config: WebDavConfig): Promise<void> {
  return invoke("webdav_sync_test", { config });
}

export async function webdavPasswordStatus(config: WebDavConfig): Promise<WebDavPasswordStatus> {
  return invoke("webdav_password_status", { config });
}

export async function saveWebdavSavedPassword(config: WebDavConfig, password: string): Promise<void> {
  return invoke("save_webdav_saved_password", { config, password });
}

export async function forgetWebdavSavedPassword(config: WebDavConfig): Promise<void> {
  return invoke("forget_webdav_saved_password", { config });
}

export async function webdavSyncSecretsStatus(): Promise<WebDavSyncSecretsStatus> {
  return invoke("webdav_sync_secrets_status");
}

export async function saveWebdavSyncSecretsPreference(enabled: boolean, passphrase?: string): Promise<void> {
  return invoke("save_webdav_sync_secrets_preference", { enabled, passphrase });
}

export async function forgetWebdavSyncSecretsPassphrase(): Promise<void> {
  return invoke("forget_webdav_sync_secrets_passphrase");
}

export async function cloudSyncLocalCatalog(editorSettings?: unknown): Promise<SyncSnapshotCatalog> {
  return invoke("cloud_sync_local_catalog", { editorSettings });
}

export async function localBackupExport(path: string, editorSettings: unknown, secretsPassphrase: string | undefined, selection: SyncSelection): Promise<LocalBackupExportSummary> {
  return invoke("local_backup_export", { path, editorSettings, secretsPassphrase, selection });
}

export async function localBackupInspect(path: string, secretsPassphrase?: string): Promise<SyncSnapshotCatalog> {
  return invoke("local_backup_inspect", { path, secretsPassphrase });
}

export async function localBackupImport(path: string, secretsPassphrase: string | undefined, restoreSecrets: boolean, selection: SyncSelection): Promise<LocalBackupImportResult> {
  return invoke("local_backup_import", { path, secretsPassphrase, restoreSecrets, selection });
}

export async function webdavSyncInspect(config: WebDavConfig, secretsPassphrase?: string): Promise<SyncSnapshotCatalog> {
  return invoke("webdav_sync_inspect", { config, secretsPassphrase });
}

export async function webdavSyncUpload(config: WebDavConfig, editorSettings?: unknown, secretsPassphrase?: string, includeSecrets = false, selection?: SyncSelection): Promise<WebDavSyncSummary> {
  return invoke("webdav_sync_upload", {
    config,
    editorSettings,
    secretsPassphrase,
    includeSecrets,
    selection,
  });
}

export async function webdavSyncDownload(config: WebDavConfig, secretsPassphrase?: string, restoreSecrets = true, selection?: SyncSelection): Promise<WebDavDownloadResult> {
  return invoke("webdav_sync_download", { config, secretsPassphrase, restoreSecrets, selection });
}

export async function snippetSyncTest(config: SnippetSyncConfig): Promise<void> {
  return invoke("snippet_sync_test", { config });
}

export async function snippetTokenStatus(config: SnippetSyncConfig): Promise<SnippetTokenStatus> {
  return invoke("snippet_token_status", { config });
}

export async function saveSnippetSavedToken(config: SnippetSyncConfig, token: string): Promise<void> {
  return invoke("save_snippet_saved_token", { config, token });
}

export async function forgetSnippetSavedToken(config: SnippetSyncConfig): Promise<void> {
  return invoke("forget_snippet_saved_token", { config });
}

export async function snippetSyncSettings(provider: SnippetProvider, instanceUrl?: string): Promise<SnippetSyncSettings> {
  return invoke("snippet_sync_settings", { provider, instanceUrl });
}

export async function saveSnippetSyncId(provider: SnippetProvider, snippetId?: string, instanceUrl?: string): Promise<void> {
  return invoke("save_snippet_sync_id", { provider, snippetId, instanceUrl });
}

export async function retrySnippetLegacyCleanup(config: SnippetSyncConfig): Promise<SnippetSyncSettings> {
  return invoke("retry_snippet_legacy_cleanup", { config });
}

export async function snippetSyncInspect(config: SnippetSyncConfig, snippetPassphrase?: string, secretsPassphrase?: string): Promise<SyncSnapshotCatalog> {
  return invoke("snippet_sync_inspect", { config, snippetPassphrase, secretsPassphrase });
}

export async function snippetSyncUpload(config: SnippetSyncConfig, editorSettings?: unknown, snippetPassphrase?: string, includeSecrets = false, secretsPassphrase?: string, selection?: SyncSelection): Promise<SnippetSyncSummary> {
  return invoke("snippet_sync_upload", {
    config,
    editorSettings,
    snippetPassphrase,
    includeSecrets,
    secretsPassphrase,
    selection,
  });
}

export async function snippetSyncDownload(config: SnippetSyncConfig, snippetPassphrase?: string, restoreSecrets = false, secretsPassphrase?: string, selection?: SyncSelection): Promise<SnippetDownloadResult> {
  return invoke("snippet_sync_download", { config, snippetPassphrase, restoreSecrets, secretsPassphrase, selection });
}

export async function loadPinnedTreeNodeIds(): Promise<string[]> {
  return invoke("load_pinned_tree_node_ids");
}

export async function savePinnedTreeNodeIds(ids: string[]): Promise<void> {
  return invoke("save_pinned_tree_node_ids", { ids });
}

export async function listSystemFonts(): Promise<string[]> {
  return invoke("list_system_fonts");
}

export async function listSshConfigHosts(): Promise<SshConfigHostEntry[]> {
  return invoke("list_ssh_config_hosts");
}

export async function listLocalSshKeys(): Promise<LocalSshKey[]> {
  return invoke("list_local_ssh_keys");
}

export async function pendingOpenSqlFiles(): Promise<string[]> {
  return invoke("pending_open_sql_files");
}

export async function pendingOpenDbFiles(): Promise<string[]> {
  return invoke("pending_open_db_files");
}

export async function pendingOpenConnectionLinks(): Promise<string[]> {
  return invoke("pending_open_connection_links");
}

export async function pendingOpenAiConfigLinks(): Promise<string[]> {
  return invoke("pending_open_ai_config_links");
}

export async function pendingOpenPluginInstallLinks(): Promise<string[]> {
  return invoke("pending_open_plugin_install_links");
}

export interface ExternalSqlFileSnapshot {
  content: string;
  version: ExternalSqlFileVersion;
  encoding?: import("@/types/database").QueryTab["externalSqlEncoding"];
}

export type ExternalSqlFileStatus = { kind: "present"; sizeBytes: number; modifiedNs: string } | { kind: "missing" };

export type ExternalSqlFileWriteResult = { kind: "written"; version: ExternalSqlFileVersion } | { kind: "conflict"; currentVersion: ExternalSqlFileVersion } | { kind: "missing" };

export async function readExternalSqlFileSnapshot(path: string, maxSizeBytes?: number, encoding?: string): Promise<ExternalSqlFileSnapshot> {
  const result = await invoke<{ kind: "content"; content: string; version: ExternalSqlFileVersion; encoding?: ExternalSqlFileSnapshot["encoding"] } | { kind: "tooLarge"; sizeBytes: number; maxSizeBytes: number }>("read_external_sql_file", { path, maxSizeBytes, encoding: encoding ?? null });
  if (result.kind === "tooLarge") {
    throw new ExternalSqlFileTooLargeError(result.sizeBytes, result.maxSizeBytes);
  }
  return { content: result.content, version: result.version, ...(result.encoding ? { encoding: result.encoding } : {}) };
}

export async function readExternalSqlFile(path: string, maxSizeBytes?: number, encoding?: string): Promise<string> {
  return (await readExternalSqlFileSnapshot(path, maxSizeBytes, encoding)).content;
}

export async function inspectExternalSqlFile(path: string): Promise<ExternalSqlFileStatus> {
  return invoke("inspect_external_sql_file", { path });
}

export async function writeExternalSqlFile(path: string, content: string, options: { expectedContentHash?: string; expectedMissing?: boolean; encoding?: string } = {}): Promise<ExternalSqlFileWriteResult> {
  return invoke("write_external_sql_file", {
    path,
    content,
    expectedContentHash: options.expectedContentHash ?? null,
    expectedMissing: options.expectedMissing ?? false,
    encoding: options.encoding ?? "utf8",
  });
}

export async function saveExternalSqlFile(defaultFileName: string, content: string, filterExtension?: string, encoding?: string): Promise<{ path: string; version: ExternalSqlFileVersion } | null> {
  return invoke("save_external_sql_file", { defaultFileName, content, filterExtension, encoding: encoding ?? "utf8" });
}

export interface SqlFileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  children: SqlFileEntry[];
}

export async function listSqlFilesInFolder(folderPath: string, fileFilter?: string): Promise<SqlFileEntry[]> {
  return invoke("list_sql_files_in_folder", { folderPath, fileFilter });
}

export async function createSqlFileInFolder(rootPath: string, directoryPath: string, fileName: string): Promise<string> {
  return invoke("create_sql_file_in_folder", { rootPath, directoryPath, fileName });
}

export async function renameSqlFileInFolder(rootPath: string, filePath: string, fileName: string): Promise<string> {
  return invoke("rename_sql_file_in_folder", { rootPath, filePath, fileName });
}

export async function deleteSqlFileInFolder(rootPath: string, filePath: string): Promise<void> {
  return invoke("delete_sql_file_in_folder", { rootPath, filePath });
}

export interface GlobalSearchRequest {
  roots: string[];
  query: string;
  extensions?: string[];
  caseSensitive?: boolean;
  useRegex?: boolean;
  wholeWord?: boolean;
  limit?: number;
}

export interface GlobalSearchMatch {
  path: string;
  fileName: string;
  /** 1-based line number. */
  line: number;
  /** 1-based char column within the line (for CodeMirror). */
  column: number;
  matchText: string;
  lineText: string;
}

export async function globalSearch(request: GlobalSearchRequest): Promise<GlobalSearchMatch[]> {
  return invoke("global_search", { request });
}

// --- AI Conversations ---

export interface AiChatMessage {
  role: string;
  content: string;
  mentions?: unknown[];
  reasoning?: string;
  kind?: "contextSummary" | "writeSqlConfirmation" | "productionWriteBlocked";
  /** Set on the assistant message whose generation failed; persisted (mirrors dbx-core `AiChatMessage.failed`). */
  failed?: boolean;
  /** Target frozen when this assistant turn started, retained for confirmation. */
  sourceBinding?: import("@/lib/ai/aiConversationBinding").AiConversationBinding;
  /**
   * Footprint of a turn that carried a context selection (#10058). The selection
   * text is never persisted (it can be 12 000 chars and records are
   * cloud-synced), so this boolean is all a reloaded transcript has left to say
   * the turn was not empty. Absent on records written before the field existed.
   */
  selectionsOmitted?: boolean;
}

export interface AiConversation {
  pluginContext?: import("@/lib/ai/aiPluginConversation").AiPluginContext;
  id: string;
  title: string;
  connectionName: string;
  /** Connection this conversation is bound to (#9902). The binding belongs to
   *  the conversation, not to whichever editor tab is active.
   *  Empty means "unbound": either persisted before session-scoped binding
   *  existed and its `connectionName` matched zero or several saved connections
   *  (names are not unique), or the bound connection was deleted. Consumers must
   *  ask the user rather than fall back to the active tab. */
  connectionId: string;
  database: string;
  /** Schema for schema-scoped engines (Postgres, Dameng); absent otherwise. */
  schema?: string;
  messages: AiChatMessage[];
  /** One editable "send later" input saved while an active run occupies the
   *  conversation (parent PRD §5). Persisted with the conversation. */
  queuedInput?: string;
  createdAt: string;
  updatedAt: string;
}

export type AiRunStatus = "preparing" | "queued" | "running" | "awaiting_write_confirmation" | "completed" | "failed" | "cancelled" | "interrupted" | "pending_recoverable";

export type AiRunFifoCategory = "normal_send" | "write_confirmation_resume";

export interface AiRun {
  runId: string;
  conversationId: string;
  sessionIds: string[];
  status: AiRunStatus;
  connectionId: string;
  database: string;
  schema?: string;
  pendingConfirmation?: unknown;
  fifoCategory?: AiRunFifoCategory;
  pendingInput?: string;
  /** Highest event seq assigned to this run across all its sessions (parent
   *  PRD §8). Drives the unread baseline and the "updates while you were away"
   *  separator anchor. */
  maxSeq?: number;
  createdAt: string;
  updatedAt: string;
}

export async function saveAiConversation(conversation: AiConversation): Promise<void> {
  return invoke("save_ai_conversation", { conversation });
}

export async function loadAiConversations(): Promise<AiConversation[]> {
  return invoke("load_ai_conversations");
}

export async function deleteAiConversation(id: string): Promise<void> {
  return invoke("delete_ai_conversation", { id });
}

export async function saveAiRun(run: AiRun): Promise<void> {
  return invoke("save_ai_run", { run });
}

export async function saveAiRunState(conversation: AiConversation, run: AiRun): Promise<void> {
  return invoke("save_ai_run_state", { conversation, run });
}

export async function loadAiRuns(): Promise<AiRun[]> {
  return invoke("load_ai_runs");
}

// --- Prompt Templates ---

export interface PromptTemplate {
  id: string;
  name: string;
  content: string;
  createdAt: string;
  updatedAt: string;
}

export async function loadPromptTemplates(): Promise<PromptTemplate[]> {
  return invoke("load_prompt_templates");
}

export async function savePromptTemplate(id: string, name: string, content: string): Promise<PromptTemplate> {
  return invoke("save_prompt_template", { id, name, content });
}

export async function deletePromptTemplate(id: string): Promise<void> {
  return invoke("delete_prompt_template", { id });
}

export async function getAiGlobalCustomInstructions(): Promise<string> {
  return invoke("get_ai_global_custom_instructions");
}

export async function setAiGlobalCustomInstructions(content: string): Promise<void> {
  return invoke("set_ai_global_custom_instructions", { content });
}

export async function listUserSkills(settings: UserSkillRootSettings): Promise<UserSkillsListResult> {
  return invoke("list_user_skills", { customRootEnabled: settings.customRootEnabled, customRoot: settings.customRoot });
}

export async function readUserSkills(ids: string[], settings: UserSkillRootSettings): Promise<UserSkillsReadResult> {
  return invoke("read_user_skills", { ids, customRootEnabled: settings.customRootEnabled, customRoot: settings.customRoot });
}

export async function testConnection(config: ConnectionConfig): Promise<string> {
  return invokeBackend("test_connection", { config });
}

export async function testSshTunnel(config: ConnectionConfig): Promise<string> {
  return invokeBackend("test_ssh_tunnel", { config });
}

export async function testConnectionWithInfo(config: ConnectionConfig): Promise<ConnectionTestResult> {
  try {
    const result = await invoke<unknown>("test_connection_with_info", {
      config,
    });
    return normalizeConnectionTestResult(result, config);
  } catch (error) {
    if (!isTauriCommandUnavailable(error, "test_connection_with_info")) throw error;
    return normalizeConnectionTestResult(await testConnection(config), config);
  }
}

// ---------------------------------------------------------------------------
// Salesforce OAuth (browser redirect + device-code flows)
// ---------------------------------------------------------------------------

/** Identity of the user an established Salesforce connection is authenticated as (cached backend-side). */

export async function connectDb(config: ConnectionConfig, clientAttempt?: number): Promise<string> {
  return invokeBackend("connect_db", { config, clientAttempt });
}

export async function connectionDatabaseInfo(connectionId: string, database?: string): Promise<DatabaseConnectionInfo | undefined> {
  const info = await invokeBackend<DatabaseConnectionInfo | null>("connection_database_info", { connectionId, database });
  return info ?? undefined;
}

export async function saveConnectionDatabaseInfo(connectionId: string, databaseInfo: DatabaseConnectionInfo): Promise<void> {
  return invokeBackend("save_connection_database_info", {
    connectionId,
    databaseInfo,
  });
}

export interface WriteUnlockState {
  remainingMs: number;
}

export async function unlockConnectionWrites(connectionId: string, durationSecs: number): Promise<number> {
  const state = await invokeBackend<WriteUnlockState>("unlock_connection_writes", { connectionId, durationSecs });
  return state.remainingMs;
}

export async function lockConnectionWrites(connectionId: string): Promise<void> {
  return invokeBackend("lock_connection_writes", { connectionId });
}

export async function connectionWriteUnlockState(connectionId: string): Promise<number> {
  const state = await invokeBackend<WriteUnlockState>("connection_write_unlock_state", { connectionId });
  return state.remainingMs;
}

export async function connectionFinalProxyPort(config: ConnectionConfig): Promise<number> {
  return invokeBackend("connection_final_proxy_port", { config });
}

export async function disconnectDb(connectionId: string, clientAttempt?: number): Promise<void> {
  return invokeBackend("disconnect_db", { connectionId, clientAttempt });
}

export async function sessionCredentialStatus(connectionId: string): Promise<boolean> {
  return invokeBackend("session_credential_status", { connectionId });
}

export async function forgetSessionCredential(connectionId: string): Promise<void> {
  return invokeBackend("forget_session_credential", { connectionId });
}

export async function replaceNacosSessionCredential(connectionId: string, username: string, password: string): Promise<void> {
  return invokeBackend("replace_nacos_session_credential", { connectionId, username, password });
}

export async function checkConnectionHealth(connectionId: string): Promise<void> {
  return invokeBackend("check_connection_health", { connectionId });
}

/**
 * Read-only counterpart of `checkConnectionHealth`: reports whether the connection still has a
 * pool, without probing or mutating anything (#4339).
 *
 * Liveness events must be confirmed through this, never through `checkConnectionHealth`: the
 * latter removes unhealthy pools and is the path `ensureConnected` uses to trigger a reconnect.
 */
export async function connectionIsOpen(connectionId: string): Promise<boolean> {
  return invokeBackend("connection_is_open", { connectionId });
}

export async function prewarmConnection(connectionId: string, database?: string, catalog?: string, clientSessionId?: string): Promise<void> {
  return invokeBackend("prewarm_connection", { connectionId, database, catalog, clientSessionId });
}

export async function connectionIdentifierQuote(connectionId: string, database?: string): Promise<string | undefined> {
  const quote = await invoke<string | null>("connection_identifier_quote", {
    connectionId,
    database,
  });
  return quote ?? undefined;
}

export async function closeDatabaseConnection(connectionId: string, database: string): Promise<boolean> {
  return invoke("close_database_connection", { connectionId, database });
}

export async function listDatabases(connectionId: string): Promise<DatabaseInfo[]> {
  return invoke("list_databases", { connectionId });
}

export async function listDatabaseMetadata(connectionId: string): Promise<DatabaseInfo[]> {
  return invoke("list_database_metadata", { connectionId });
}

export async function listDatabaseStorage(connectionId: string, databases: string[]): Promise<DatabaseStorageInfo[]> {
  return invoke("list_database_storage", { connectionId, databases });
}

export async function listXuguTablespaces(connectionId: string, database?: string): Promise<XuguTablespaceInfo[]> {
  return invoke("list_xugu_tablespaces", { connectionId, database });
}

export async function getSqlServerCompletionContext(connectionId: string, database: string): Promise<SqlServerCompletionContext> {
  return invoke("get_sqlserver_completion_context", { connectionId, database });
}

export async function listDorisCatalogs(connectionId: string): Promise<CatalogInfo[]> {
  return invoke("list_doris_catalogs", { connectionId });
}

export async function listDorisCatalogDatabases(connectionId: string, catalog: string): Promise<DatabaseInfo[]> {
  return invoke("list_doris_catalog_databases", { connectionId, catalog });
}

export async function listSqlServerLinkedServers(connectionId: string): Promise<LinkedServerInfo[]> {
  return invoke("list_sqlserver_linked_servers", { connectionId });
}

export async function listSqlServerLinkedServerCatalogs(connectionId: string, server: string): Promise<DatabaseInfo[]> {
  return invoke("list_sqlserver_linked_server_catalogs", {
    connectionId,
    server,
  });
}

export async function listSqlServerLinkedServerSchemas(connectionId: string, server: string, catalog: string): Promise<string[]> {
  return invoke("list_sqlserver_linked_server_schemas", {
    connectionId,
    server,
    catalog,
  });
}

export async function listSqlServerLinkedServerTables(connectionId: string, server: string, catalog: string, schema: string, filter?: string, limit?: number, offset?: number): Promise<TableInfo[]> {
  return invoke("list_sqlserver_linked_server_tables", {
    connectionId,
    server,
    catalog,
    schema,
    filter,
    limit,
    offset,
  });
}

export async function saveSchemaCache(cacheKey: string, payload: unknown): Promise<void> {
  return invoke("save_schema_cache", { cacheKey, payload });
}

export async function loadSchemaCache<T = unknown>(cacheKey: string): Promise<T | null> {
  return invoke("load_schema_cache", { cacheKey });
}

export async function deleteSchemaCachePrefix(prefix: string): Promise<void> {
  return invoke("delete_schema_cache_prefix", { prefix });
}

export async function listTables(connectionId: string, database: string, schema: string, filter?: string, limit?: number, offset?: number, objectTypes?: SidebarObjectKind[], catalog?: string, tableNameFilter?: import("@/types/database").TableNameFilter): Promise<TableInfo[]> {
  return invoke("list_tables", {
    connectionId,
    database,
    schema,
    filter,
    limit,
    offset,
    objectTypes,
    catalog,
    tableNameFilter,
  });
}

export async function getTableComment(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<string | null> {
  return invoke("get_table_comment", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function getMysqlTableAutoIncrement(connectionId: string, database: string, table: string): Promise<string | null> {
  return invoke("get_mysql_table_auto_increment", { connectionId, database, table });
}

export async function listObjects(
  connectionId: string,
  database: string,
  schema: string,
  objectTypes?: (SidebarObjectKind | "EVENT")[],
  filter?: string,
  limit?: number,
  offset?: number,
  catalog?: string,
  tableNameFilter?: import("@/types/database").TableNameFilter,
  executionId?: string,
): Promise<ObjectInfo[]> {
  return invoke("list_objects", {
    connectionId,
    database,
    schema,
    objectTypes,
    filter,
    limit,
    offset,
    catalog,
    tableNameFilter,
    executionId,
  });
}

export async function listObjectStatistics(connectionId: string, database: string, schema: string): Promise<ObjectStatistics[]> {
  return invoke("list_object_statistics", { connectionId, database, schema });
}

export async function listCompletionObjects(connectionId: string, database: string, schema: string): Promise<ObjectInfo[]> {
  return invoke("list_completion_objects", { connectionId, database, schema });
}

export async function completionAssistantSearch(request: CompletionAssistantRequest): Promise<CompletionAssistantResponse> {
  return invoke("completion_assistant_search", { request });
}

export async function getObjectSource(connectionId: string, database: string, schema: string, name: string, objectType: ObjectSourceKind, signature?: string, relationName?: string): Promise<ObjectSource> {
  return invoke("get_object_source", {
    connectionId,
    database,
    schema,
    name,
    objectType,
    signature,
    relationName,
  });
}

export async function getEventInfo(connectionId: string, database: string, schema: string, name: string): Promise<MysqlEventInfo> {
  return invoke("get_event_info", { connectionId, database, schema, name });
}

export async function listSchemas(connectionId: string, database: string, applyVisibleFilter = false): Promise<string[]> {
  return invoke("list_schemas", { connectionId, database, applyVisibleFilter });
}

export async function listSchemaInfos(connectionId: string, database: string): Promise<SchemaInfo[]> {
  return invoke("list_schema_infos", { connectionId, database });
}

export async function getCustomTypeDetails(connectionId: string, database: string, schema: string, name: string): Promise<CustomTypeDetails> {
  return invoke("get_custom_type_details", { connectionId, database, schema, name });
}
export async function getColumns(connectionId: string, database: string, schema: string, table: string, catalog?: string, clientSessionId?: string): Promise<ColumnInfo[]> {
  return invoke("get_columns", {
    connectionId,
    database,
    schema,
    table,
    catalog,
    clientSessionId,
  });
}

export async function getPluginTableMetadata(request: PluginTableMetadataRequest): Promise<PluginTableMetadata> {
  return invoke("get_plugin_table_metadata", { request });
}

export async function getSqlServerColumnMetadata(connectionId: string, database: string, schema: string, table: string): Promise<SqlServerColumnMetadata[]> {
  return invoke("get_sqlserver_column_metadata", {
    connectionId,
    database,
    schema,
    table,
  });
}

export interface TableColumnsResult {
  table_name: string;
  columns: ColumnInfo[];
  error?: string;
}

export async function getAllColumns(connectionId: string, database: string, schema: string): Promise<TableColumnsResult[]> {
  return invoke("get_all_columns", { connectionId, database, schema });
}

export async function listDataTypes(connectionId: string, database: string): Promise<string[]> {
  return invoke("list_data_types", { connectionId, database });
}

export async function executeQuery(
  connectionId: string,
  database: string,
  sql: string,
  schema?: string,
  executionId?: string,
  options?: {
    maxRows?: number;
    catalog?: string;
    fetchSize?: number;
    pageSize?: number;
    rowOffset?: number;
    resultSessionId?: string;
    clientSessionId?: string;
    timeoutSecs?: number;
    executionMode?: "simple";
  },
): Promise<QueryResult> {
  try {
    return await invoke("execute_query", {
      connectionId,
      database,
      sql,
      schema,
      executionId,
      ...options,
    });
  } catch (error) {
    throw new BackendErrorException(error);
  }
}

export async function executeConditionalUpdate(
  connectionId: string,
  database: string,
  sql: string,
  schema?: string,
  executionId?: string,
  options?: {
    maxRows?: number;
    catalog?: string;
    fetchSize?: number;
    pageSize?: number;
    rowOffset?: number;
    resultSessionId?: string;
    clientSessionId?: string;
    timeoutSecs?: number;
    executionMode?: "simple";
  },
): Promise<QueryResult> {
  try {
    return await invoke("execute_conditional_update", {
      connectionId,
      database,
      sql,
      schema,
      executionId,
      ...options,
    });
  } catch (error) {
    throw new BackendErrorException(error);
  }
}

export async function executeMulti(
  connectionId: string,
  database: string,
  sql: string,
  schema?: string,
  executionId?: string,
  options?: {
    maxRows?: number;
    catalog?: string;
    fetchSize?: number;
    pageSize?: number;
    rowOffset?: number;
    maxResultBytes?: number;
    resultKeyColumns?: string[];
    tableDataPreview?: boolean;
    resultSessionId?: string;
    clientSessionId?: string;
    timeoutSecs?: number;
    useTransaction?: boolean;
    continueOnError?: boolean;
    executionMode?: "simple";
    /** MySQL auto-commit tabs: keep a transaction the user opened explicitly
     *  (`BEGIN` / `START TRANSACTION`) open across executions until COMMIT /
     *  ROLLBACK instead of rolling it back when the batch ends. */
    preserveExplicitTransaction?: boolean;
  },
): Promise<QueryResult[]> {
  const diagnosticsEnabled = isDebugLoggingEnabled();
  const startedAt = diagnosticsEnabled ? performance.now() : 0;
  try {
    const results = await invoke<QueryResult[]>("execute_multi", {
      connectionId,
      database,
      sql,
      schema,
      executionId,
      ...options,
    });
    if (diagnosticsEnabled) {
      appendDebugLog("info", "[DBX][query-transport:tauri]", {
        traceId: executionId?.slice(0, 8),
        totalMs: Math.round(performance.now() - startedAt),
        resultCount: results.length,
        rowCounts: results.map((result) => result.rows.length),
        columnCounts: results.map((result) => result.columns.length),
      });
    }
    return results;
  } catch (error) {
    if (diagnosticsEnabled) {
      appendDebugLog("warn", "[DBX][query-transport:tauri:error]", {
        traceId: executionId?.slice(0, 8),
        totalMs: Math.round(performance.now() - startedAt),
      });
    }
    throw new BackendErrorException(error);
  }
}

export interface ExecuteMultiProgress {
  executionId: string;
  statementIndex: number;
  completed: number;
  total: number;
  success: boolean;
  executionTimeMs: number;
  affectedRows: number;
  error?: BackendError;
}

export async function executeMultiWithProgress(
  connectionId: string,
  database: string,
  sql: string,
  onProgress: (progress: ExecuteMultiProgress) => void,
  schema?: string,
  options?: {
    maxRows?: number;
    catalog?: string;
    fetchSize?: number;
    pageSize?: number;
    rowOffset?: number;
    maxResultBytes?: number;
    resultKeyColumns?: string[];
    tableDataPreview?: boolean;
    resultSessionId?: string;
    clientSessionId?: string;
    timeoutSecs?: number;
    useTransaction?: boolean;
    continueOnError?: boolean;
    executionMode?: "simple";
    preserveExplicitTransaction?: boolean;
    executionId?: string;
  },
): Promise<QueryResult[]> {
  const executionId = options?.executionId ?? uuid();
  const { executionId: _executionId, ...invokeOptions } = options ?? {};
  const unlisten = await listen<ExecuteMultiProgress>("query-batch-progress", (event) => {
    if (event.payload.executionId === executionId) onProgress(event.payload);
  });
  try {
    return await invoke("execute_multi", { connectionId, database, sql, schema, executionId, ...invokeOptions });
  } catch (error) {
    throw new BackendErrorException(error);
  } finally {
    unlisten();
  }
}

export async function refreshConnections(): Promise<void> {
  return invoke("refresh_connections");
}

export async function cancelQueryAndWait(executionId: string): Promise<{ requested: boolean; terminal: boolean }> {
  return invokeBackend("cancel_conditional_update", { executionId });
}

export async function cancelQuery(executionId: string): Promise<boolean> {
  return invoke("cancel_query", { executionId });
}

export interface ConditionalUpdateCancellationResult {
  requested: boolean;
  terminal: boolean;
}

export async function cancelConditionalUpdate(executionId: string): Promise<ConditionalUpdateCancellationResult> {
  return invoke("cancel_conditional_update", { executionId });
}

export async function closeQuerySession(connectionId: string, database: string, sessionId: string, clientSessionId?: string, catalog?: string): Promise<boolean> {
  return invoke("close_query_session", {
    connectionId,
    database,
    sessionId,
    clientSessionId,
    catalog,
  });
}

export async function closeClientConnectionSession(connectionId: string, database: string, clientSessionId: string, catalog?: string): Promise<boolean> {
  return invoke("close_client_connection_session", {
    connectionId,
    database,
    clientSessionId,
    catalog,
  });
}

export async function executeBatch(connectionId: string, database: string, statements: string[], schema?: string, timeoutSecs?: number, useTransaction?: boolean): Promise<QueryResult> {
  return invoke("execute_batch", {
    connectionId,
    database,
    statements,
    schema,
    timeoutSecs,
    useTransaction,
  });
}

export async function executeScript(connectionId: string, database: string, sql: string, schema?: string): Promise<QueryResult> {
  return invoke("execute_script", { connectionId, database, sql, schema });
}

export async function executeScriptWith2pc(connectionId: string, database: string, statements: string[], schema?: string, destructiveConfirmed = false): Promise<TransactionLog> {
  return invoke("execute_script_with_2pc", {
    connectionId,
    database,
    statements,
    schema,
    destructiveConfirmed,
  });
}

export async function executeInTransaction(connectionId: string, database: string, statements: string[], schema?: string, catalog?: string): Promise<QueryResult> {
  return invoke("execute_in_transaction", {
    connectionId,
    database,
    statements,
    schema,
    catalog,
  });
}

export async function beginManualTransaction(connectionId: string, database: string, schema?: string, catalog?: string): Promise<string> {
  return invokeBackend("begin_manual_transaction", { connectionId, database, schema, catalog });
}

export async function executeInManualTransaction(
  txnSessionId: string,
  sql: string,
  database: string,
  schema?: string,
  maxRows?: number,
  tableDataPreview?: boolean,
  pageSize?: number,
  resultSessionId?: string,
  classificationSql?: string,
  executionId?: string,
  timeoutSecs?: number,
): Promise<QueryResult[]> {
  return invokeBackend("execute_in_manual_transaction", {
    txnSessionId,
    sql,
    database,
    schema,
    maxRows,
    tableDataPreview,
    pageSize,
    resultSessionId,
    classificationSql,
    executionId,
    timeoutSecs,
  });
}

export async function commitManualTransaction(txnSessionId: string): Promise<QueryResult> {
  return invokeBackend("commit_manual_transaction", { txnSessionId });
}

export async function rollbackManualTransaction(txnSessionId: string): Promise<QueryResult> {
  return invokeBackend("rollback_manual_transaction", { txnSessionId });
}

export async function analyzeSqlReferences(sql: string, dialect?: string): Promise<SqlReferenceAnalysis> {
  return invoke("analyze_sql_references", { sql, dialect });
}

export async function findStatementAtCursor(sql: string, cursorPos: number, databaseType?: DatabaseType): Promise<string> {
  return invoke("find_statement_at_cursor", { sql, cursorPos, databaseType });
}

export async function prepareQueryPaginationExecutionPlan(options: QueryPaginationExecutionPlanOptions): Promise<QueryPaginationExecutionPlan> {
  return invoke("prepare_query_pagination_execution_plan", { options });
}

export async function buildSortedQuerySql(options: SortedQuerySqlOptions): Promise<QuerySqlBuildResult> {
  return invoke("build_sorted_query_sql", { options });
}

export async function buildExplainSql(options: BuildExplainSqlOptions): Promise<ExplainSqlBuildResult> {
  return invoke("build_explain_sql", { options });
}

export async function buildCreateUserSql(username: string, password: string, tablespace: string): Promise<string> {
  return invoke("build_create_user_sql", { username, password, tablespace });
}

export async function getExplainInfo(connectionId: string, database: string | undefined, schema: string | undefined, sql: string, mode: string): Promise<string | undefined> {
  // Preserve Agent/driver errors so the explain view can show the actionable cause.
  return invoke<string>("get_explain_info", {
    connectionId,
    database,
    schema,
    sql,
    mode,
  });
}

/** Plugin Host API: what the host and this connection can plan. Never connects. */
export async function getPluginPlanCapabilities(connectionId: string): Promise<PluginPlanCapabilities> {
  return invoke<PluginPlanCapabilities>("get_plugin_plan_capabilities", { connectionId });
}

/**
 * Plugin Host API: acquires the estimated plan for caller-supplied SQL. The
 * backend generates and owns the EXPLAIN statement; the request cannot carry one.
 */
export async function getPluginEstimatedPlan(request: PluginPlanRequest): Promise<PluginPlanResult> {
  return invoke<PluginPlanResult>("get_plugin_estimated_plan", { request });
}

/**
 * Plugin Host API (`host.data:read`): one read-only statement on a connection
 * the user granted to `pluginId`. The backend enforces permission, grant, and gate.
 */
export async function queryPluginData(pluginId: string, request: PluginDataQueryRequest): Promise<PluginDataQueryResult> {
  return invoke<PluginDataQueryResult>("query_plugin_data", { pluginId, request });
}

export async function getPluginDataGrants(pluginId: string): Promise<PluginDataGrant[]> {
  return invoke<PluginDataGrant[]>("get_plugin_data_grants", { pluginId });
}

export async function setPluginDataGrant(pluginId: string, connectionId: string, granted: boolean): Promise<PluginDataGrant[]> {
  return invoke<PluginDataGrant[]>("set_plugin_data_grant", { pluginId, connectionId, granted });
}

export async function buildDroppedFilePreviewSql(options: DroppedFilePreviewSqlOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_dropped_file_preview_sql", {
    options,
  });
  return result ?? undefined;
}

export async function buildTableSelectSql(options: BuildTableSelectSqlOptions): Promise<string> {
  return invoke("build_table_select_sql", { options, includeDatabaseName: options.includeDatabaseName === true });
}

export async function buildDatabaseSearchSql(options: DatabaseSearchSqlOptions): Promise<DatabaseSearchSql | null> {
  return invoke("build_database_search_sql", { options });
}

export async function buildSearchResultWhere(options: SearchResultWhereOptions): Promise<string> {
  return invoke("build_search_result_where", { options });
}

export async function buildRenameObjectSql(options: BuildRenameObjectSqlOptions): Promise<string> {
  return invoke("build_rename_object_sql", { options });
}

export async function buildRenameDatabaseSql(options: { databaseType?: DatabaseType; oldName: string; newName: string; terminateConnections: boolean }): Promise<string> {
  return invoke("build_rename_database_sql", {
    databaseType: options.databaseType,
    oldName: options.oldName,
    newName: options.newName,
    terminateConnections: options.terminateConnections,
  });
}

export async function buildRenameDatabasePreflightSql(options: { databaseType?: DatabaseType; databaseName: string }): Promise<string> {
  return invoke("build_rename_database_preflight_sql", {
    databaseType: options.databaseType,
    databaseName: options.databaseName,
  });
}

export async function buildCreateDatabaseSql(options: CreateDatabaseSqlOptions): Promise<string> {
  return invoke("build_create_database_sql", { options });
}

export async function buildDuckDbAttachDatabaseSql(path: string, name: string): Promise<string> {
  return invoke("build_duckdb_attach_database_sql", {
    options: { path, name },
  });
}

export async function buildSqliteAttachDatabaseSql(path: string, name: string): Promise<string> {
  return invoke("build_sqlite_attach_database_sql", {
    options: { path, name },
  });
}

export async function buildDropObjectSql(options: DropObjectSqlOptions): Promise<string> {
  return invoke("build_drop_object_sql", { options });
}

export async function buildDropTableSql(options: TableAdminSqlOptions): Promise<string> {
  return invoke("build_drop_table_sql", { options });
}

export async function buildDropTableChildObjectSql(options: DropTableChildObjectSqlOptions): Promise<string> {
  return invoke("build_drop_table_child_object_sql", { options });
}

export async function buildEmptyTableSql(options: TableAdminSqlOptions): Promise<string> {
  return invoke("build_empty_table_sql", { options });
}

export async function buildTruncateTableSql(options: TableAdminSqlOptions): Promise<string> {
  return invoke("build_truncate_table_sql", { options });
}

export async function buildVacuumTableSql(options: VacuumTableSqlOptions): Promise<string> {
  return invoke("build_vacuum_table_sql", { options });
}

export async function buildMysqlAutoIncrementSql(options: MysqlAutoIncrementSqlOptions): Promise<string> {
  return invoke("build_mysql_auto_increment_sql", { options });
}

export async function buildDropDatabaseSql(options: DatabaseNameSqlOptions): Promise<string> {
  return invoke("build_drop_database_sql", { options });
}

export async function buildCreateSchemaSql(options: SchemaNameSqlOptions): Promise<string> {
  return invoke("build_create_schema_sql", { options });
}

export async function buildUpdateDatabasePropertiesSql(options: DatabasePropertyEditSqlOptions): Promise<string> {
  return invoke("build_update_database_properties_sql", { options });
}

export async function buildDropSchemaSql(options: SchemaNameSqlOptions): Promise<string> {
  return invoke("build_drop_schema_sql", { options });
}

export async function buildDuplicateTableStructureSql(options: DuplicateTableStructureSqlOptions): Promise<string> {
  return invoke("build_duplicate_table_structure_sql", { options });
}

export async function buildCopyTableDataSql(options: CopyTableDataSqlOptions): Promise<string> {
  return invoke("build_copy_table_data_sql", { options });
}

export async function buildExecutableObjectSourceStatements(input: BuildEditableObjectSourceSqlInput): Promise<string[]> {
  return invoke("build_executable_object_source_statements", { input });
}

export async function buildExecutableObjectSourceSql(input: BuildEditableObjectSourceSqlInput): Promise<string> {
  return invoke("build_executable_object_source_sql", { input });
}

export async function buildEditableObjectSource(input: BuildEditableObjectSourceSqlInput): Promise<string> {
  return invoke("build_editable_object_source", { input });
}

export async function buildRoutineRenameObjectSourceStatements(input: BuildRoutineRenameObjectSourceInput): Promise<string[]> {
  return invoke("build_routine_rename_object_source_statements", { input });
}

export async function buildViewDdlSql(input: BuildViewDdlInput): Promise<string> {
  return invoke("build_view_ddl_sql", { input });
}

export async function buildTableStructureChangeSql(options: BuildTableStructureChangeSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_table_structure_change_sql", { options });
}

export async function buildTableOwnerChangeSql(options: BuildTableOwnerChangeSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_table_owner_change_sql", { options });
}

export async function buildTablePartitionOperationSql(options: TablePartitionSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_table_partition_operation_sql", { options });
}

export async function buildCreatePartitionedTableSql(options: BuildCreatePartitionedTableSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_create_partitioned_table_sql", { options: options.options, partitioning: options.partitioning });
}

export async function previewSqliteTableStructureChange(connectionId: string, database: string, options: BuildTableStructureChangeSqlOptions): Promise<SqliteTableStructureChangePreview> {
  return invoke("preview_sqlite_table_structure_change", {
    connectionId,
    database,
    options,
  });
}

export async function applySqliteTableStructureChange(connectionId: string, database: string, options: BuildTableStructureChangeSqlOptions, schemaRevision: string): Promise<QueryResult> {
  return invoke("apply_sqlite_table_structure_change", {
    connectionId,
    database,
    options,
    schemaRevision,
  });
}

export async function buildCreateTableSql(options: BuildTableStructureChangeSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_create_table_sql", { options });
}

export async function buildSingleColumnAlterSql(options: BuildSingleColumnAlterSqlOptions): Promise<TableStructureChangeSql> {
  return invoke("build_single_column_alter_sql", { options });
}

export async function analyzeEditableQueryEditability(sql: string): Promise<QueryEditability> {
  return invoke("analyze_editable_query_editability", { sql });
}

/// A server-side check that must pass before `statements` may run. Without a
/// primary key a row is addressed by matching every column value, so the same
/// predicate can match rows outside the loaded page; `sql` counts the matches
/// of a predicate the save actually sends, and the save must be refused with
/// `message` unless the returned count is at most `maxMatchedRows`.
export interface DataGridSaveGuard {
  sql: string;
  maxMatchedRows: number;
  message: string;
}

export interface DataGridSavePreparation {
  validationError?: string;
  statements: string[];
  rollbackStatements: string[];
  executionSchema?: string;
  keylessGuards?: DataGridSaveGuard[];
}

export async function prepareDataGridSave(options: DataGridSaveStatementOptions, driverProfile?: string): Promise<DataGridSavePreparation> {
  return invoke("prepare_data_grid_save", { options, driverProfile });
}

export async function extractDataGridSelection(request: DataGridExtractRequest): Promise<DataGridExtractResult> {
  return invoke("extract_data_grid_selection", { request });
}

export async function buildDataGridCopyUpdateStatements(options: DataGridCopyUpdateStatementOptions): Promise<string[]> {
  return invoke("build_data_grid_copy_update_statements", { options });
}

export async function buildDataGridCopyInsertStatement(options: DataGridCopyInsertStatementOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_data_grid_copy_insert_statement", { options });
  return result ?? undefined;
}

export async function buildDataGridContextFilterCondition(options: DataGridContextFilterConditionOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_data_grid_context_filter_condition", { options });
  return result ?? undefined;
}

export async function buildDmlChangePreviewSql(options: DmlChangePreviewSqlOptions): Promise<DmlChangePreviewSqlResult> {
  return invoke("build_dml_change_preview_sql", { options });
}

export async function buildDataGridColumnValueFilterCondition(options: DataGridColumnValueFilterConditionOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_data_grid_column_value_filter_condition", { options });
  return result ?? undefined;
}

export async function buildDataGridColumnValuesFilterCondition(options: DataGridColumnValuesFilterConditionOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_data_grid_column_values_filter_condition", { options });
  return result ?? undefined;
}

export async function buildDataGridColumnDistinctValuesSql(options: DataGridColumnDistinctValuesSqlOptions): Promise<string> {
  return invoke("build_data_grid_column_distinct_values_sql", { options });
}

export async function buildDataGridCountSql(options: DataGridCountSqlOptions): Promise<string> {
  return invoke("build_data_grid_count_sql", { options });
}

export async function buildDataGridConditionalUpdateSql(options: DataGridConditionalUpdateSqlOptions): Promise<string | undefined> {
  const result = await invoke<string | null>("build_data_grid_conditional_update_sql", { options });
  return result ?? undefined;
}

export async function buildHiveTablePropertiesSql(options: HiveTablePropertiesSqlOptions): Promise<string> {
  return invoke("build_hive_table_properties_sql", { options });
}

export async function buildExportInsertStatements(options: BuildExportInsertStatementsOptions): Promise<string[]> {
  return invoke("build_export_insert_statements", { options });
}

export async function buildExportSqlInsert(options: BuildExportSqlInsertOptions): Promise<string> {
  return invoke("build_export_sql_insert", { options });
}

export async function buildDatabaseSqlExport(options: BuildDatabaseSqlExportOptions): Promise<string> {
  return invoke("build_database_sql_export", { options });
}

export async function prepareDataCompare(options: DataComparePreparationOptions): Promise<DataComparePreparation> {
  return invoke("prepare_data_compare", { options });
}

export async function prepareDataCompareFromTables(options: DataCompareFromTablesOptions): Promise<DataCompareFromTablesPreparation> {
  return invoke("prepare_data_compare_from_tables", { options });
}

export async function prepareDataCompareMissingTarget(options: import("@/lib/dataGrid/dataCompare").DataCompareMissingTargetOptions): Promise<DataCompareFromTablesPreparation> {
  return invoke("prepare_data_compare_missing_target", { options });
}

export async function buildDataCompareSyncPlan(options: DataCompareSyncPlanOptions): Promise<DataCompareSyncPlan> {
  return invoke("build_data_compare_sync_plan", { options });
}

export async function listIndexes(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<IndexInfo[]> {
  return invoke("list_indexes", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listReferenceKeyColumns(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<string[]> {
  return invoke("list_reference_key_columns", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listReferenceKeys(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<ReferenceKeyInfo[]> {
  return invoke("list_reference_keys", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listForeignKeys(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<ForeignKeyInfo[]> {
  return invoke("list_foreign_keys", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listForeignKeysForDatabase(connectionId: string, database: string, schema: string, catalog?: string, executionId?: string): Promise<Record<string, ForeignKeyInfo[]>> {
  return invoke("list_foreign_keys_for_database", { connectionId, database, schema, catalog, executionId });
}

export async function listTriggers(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<TriggerInfo[]> {
  return invoke("list_triggers", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listConstraints(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<ConstraintInfo[]> {
  return invoke("list_constraints", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function listPartitions(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<PartitionInfo[]> {
  return invoke("list_partitions", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export interface TablePartitionStatus {
  isPartitionedParent: boolean;
  isPartition: boolean;
}

export async function getTablePartitionStatus(connectionId: string, database: string, schema: string, table: string): Promise<TablePartitionStatus> {
  return invoke("get_table_partition_status", {
    connectionId,
    database,
    schema,
    table,
  });
}

export async function getTablePartitioning(connectionId: string, database: string, schema: string, table: string): Promise<import("@/types/database").PgTablePartitioning> {
  return invoke("get_table_partitioning", {
    connectionId,
    database,
    schema,
    table,
  });
}

export async function listInvalidIndexes(connectionId: string, database: string, schema: string, table: string): Promise<string[]> {
  return invoke("list_invalid_indexes", {
    connectionId,
    database,
    schema,
    table,
  });
}

export async function listSubpartitions(connectionId: string, database: string, schema: string, table: string, catalog?: string): Promise<SubpartitionInfo[]> {
  return invoke("list_subpartitions", {
    connectionId,
    database,
    schema,
    table,
    catalog,
  });
}

export async function getTableDdl(connectionId: string, database: string, schema: string, table: string, objectType?: ObjectSourceKind, catalog?: string, portable = false): Promise<string> {
  return invoke("get_table_ddl", {
    connectionId,
    database,
    schema,
    table,
    objectType,
    catalog,
    portable,
  });
}

export async function getTableDisplayDdl(connectionId: string, database: string, schema: string, table: string, objectType?: ObjectSourceKind, catalog?: string): Promise<string> {
  return invoke("get_table_ddl", {
    connectionId,
    database,
    schema,
    table,
    objectType,
    catalog,
    includePostgresAccess: true,
    portable: false,
  });
}

export async function prepareSchemaDiff(options: SchemaDiffPreparationOptions): Promise<SchemaDiffPreparation> {
  return invoke("prepare_schema_diff", { options });
}

export async function listDialectDataTypes(dialectName: string): Promise<string[]> {
  return invoke("list_dialect_data_types", { dialectName });
}

export async function generateSchemaSyncSql(diffs: TableDiff[], databaseType: DatabaseType, targetSchema?: string, functionDiffs?: FunctionDiff[], sequenceDiffs?: SequenceDiff[], ruleDiffs?: RuleDiff[], ownerDiffs?: OwnerDiff[], cascadeDelete?: boolean): Promise<string> {
  return invoke("generate_schema_sync_sql", {
    diffs,
    databaseType,
    targetSchema,
    functionDiffs: functionDiffs ?? [],
    sequenceDiffs: sequenceDiffs ?? [],
    ruleDiffs: ruleDiffs ?? [],
    ownerDiffs: ownerDiffs ?? [],
    cascadeDelete: cascadeDelete ?? false,
  });
}

export async function generateSchemaSyncPlan(input: SelectedSchemaDiffInput, options: GenerateSchemaSyncPlanOptions): Promise<SchemaSyncSqlPlan> {
  return invoke("generate_schema_sync_plan", {
    ...input,
    ...options,
  });
}

export async function listFunctions(connectionId: string, database: string, schema: string): Promise<FunctionInfo[]> {
  return invoke("list_functions", { connectionId, database, schema });
}

export async function listSequences(connectionId: string, database: string, schema: string, withLastValues: boolean): Promise<SequenceInfo[]> {
  return invoke("list_sequences", {
    connectionId,
    database,
    schema,
    withLastValues,
  });
}

export async function listRules(connectionId: string, database: string, schema: string): Promise<RuleInfo[]> {
  return invoke("list_rules", { connectionId, database, schema });
}

export async function listOwners(connectionId: string, database: string, schema: string): Promise<OwnerInfo[]> {
  return invoke("list_owners", { connectionId, database, schema });
}

export async function getTableOwner(connectionId: string, database: string, schema: string, table: string): Promise<string | null> {
  return invoke("get_table_owner", { connectionId, database, schema, table });
}

export async function listExtensions(connectionId: string, database: string, schema?: string): Promise<ExtensionInfo[]> {
  return invoke("list_extensions", { connectionId, database, schema });
}

export async function listAvailableExtensions(connectionId: string, database: string): Promise<ExtensionInfo[]> {
  return invoke("list_available_extensions", { connectionId, database });
}

export async function listEventTriggers(connectionId: string, database: string): Promise<EventTriggerInfo[]> {
  return invoke("list_event_triggers", { connectionId, database });
}

// --- Docs ---

export async function collectDocsSnapshot(connectionId: string, database: string, schemas: string[], tables: string[], projectName?: string): Promise<SchemaSnapshot> {
  return invoke("docs_collect_snapshot", { connectionId, database, schemas, tables, projectName });
}

export interface DocsCollectProgress {
  completed: number;
  total: number;
  current: string;
}

export async function collectDocsSnapshotForExport(connectionId: string, database: string, schemas: string[], tables: string[], onProgress: (progress: DocsCollectProgress) => void): Promise<SchemaSnapshot> {
  const channel = new Channel<DocsCollectProgress>();
  channel.onmessage = onProgress;
  return invoke("docs_collect_snapshot_for_export", { connectionId, database, schemas, tables, projectName: database, onProgress: channel });
}

export async function loadDocsAnnotations(connectionId: string): Promise<AnnotationFile | null> {
  return invoke("docs_load_annotations", { connectionId });
}

export async function applyDocsAnnotations(connectionId: string, snapshot: SchemaSnapshot, annotations: AnnotationFile): Promise<SchemaSnapshot> {
  return invoke("docs_apply_annotations", { connectionId, snapshot, annotations });
}

export async function saveDocsAnnotations(connectionId: string, annotations: AnnotationFile): Promise<void> {
  return invoke("docs_save_annotations", { connectionId, annotations });
}

export async function exportDocsHtml(filePath: string, snapshot: SchemaSnapshot, annotations: AnnotationFile, lang: string): Promise<void> {
  return invoke("docs_export_html", { filePath, snapshot, annotations, lang });
}

export async function saveConnections(configs: ConnectionConfig[], removedIds: string[] = []): Promise<void> {
  return invoke("save_connections", { configs, removedIds });
}

export async function loadConnections(): Promise<ConnectionConfig[]> {
  return invoke("load_connections");
}

export async function loadTunnelProfiles(): Promise<TunnelProfile[]> {
  return invoke("load_tunnel_profiles");
}

export async function saveTunnelProfiles(profiles: TunnelProfile[]): Promise<void> {
  return invoke("save_tunnel_profiles", { profiles });
}

export async function testTunnelProfile(profile: TunnelProfile): Promise<string> {
  return invoke("test_tunnel_profile", { profile });
}

export async function resolveSshPrompt(resolution: SshPromptResolution): Promise<void> {
  await invoke("resolve_ssh_prompt", { resolution });
}

export async function readKeychainPassword(service: string): Promise<string> {
  return invoke("read_keychain_password", { service, account: null });
}

export async function readKeychainPasswords(services: string[]): Promise<[string, string][]> {
  return invoke("read_keychain_passwords", { services });
}

export async function decryptConfig(payload: unknown, passphrase: string): Promise<string> {
  const { decryptConfig: decryptConfigPayload } = await import("@/lib/backend/configCrypto");
  return decryptConfigPayload(payload as any, passphrase);
}

export async function listPlugins(): Promise<InstalledPlugin[]> {
  return invoke("list_plugins");
}

export async function listPluginTrustedKeys(): Promise<PluginTrustedKey[]> {
  return invoke("list_plugin_trusted_keys");
}

export async function savePluginTrustedKey(keyId: string, publicKey: string): Promise<PluginTrustedKey[]> {
  return invoke("save_plugin_trusted_key", { keyId, publicKey });
}

export async function removePluginTrustedKey(keyId: string): Promise<PluginTrustedKey[]> {
  return invoke("remove_plugin_trusted_key", { keyId });
}

export async function listPluginRepositories(): Promise<PluginRepository[]> {
  return invoke("list_plugin_repositories");
}

export async function savePluginRepository(repository: PluginRepository): Promise<PluginRepository[]> {
  return invoke("save_plugin_repository", { repository });
}

export async function removePluginRepository(repositoryId: string): Promise<PluginRepository[]> {
  return invoke("remove_plugin_repository", { repositoryId });
}

export async function fetchPluginMarketplaceCatalogs(): Promise<PluginRepositoryCatalogResult[]> {
  return invoke("fetch_plugin_marketplace_catalogs");
}

export async function installMarketplacePlugin(request: PluginMarketplaceInstallRequest): Promise<PluginInstallResult> {
  return invoke("install_marketplace_plugin", { request });
}

export async function installPluginPackage(pathOrFile: string | File, allowUnsigned = false): Promise<PluginInstallResult> {
  if (typeof pathOrFile !== "string") throw new Error("Desktop plugin installation requires a local .dbxp file path");
  return invoke("install_plugin_package", { path: pathOrFile, allowUnsigned });
}

export async function installPluginPackageFromUrl(url: string, allowUnsigned = false): Promise<PluginInstallResult> {
  return invoke("install_plugin_package_from_url", { url, allowUnsigned });
}

export async function rollbackPlugin(pluginId: string): Promise<PluginRollbackResult> {
  return invoke("rollback_plugin", { pluginId });
}

export async function uninstallPlugin(pluginId: string): Promise<InstalledPlugin[]> {
  return invoke("uninstall_plugin", { pluginId });
}

export async function activatePlugin(pluginId: string): Promise<ActivePluginSession[]> {
  return invoke("activate_plugin", { pluginId });
}

export async function listActivePlugins(): Promise<ActivePluginSession[]> {
  return invoke("list_active_plugins");
}

export async function stopPlugin(pluginId: string): Promise<void> {
  return invoke("stop_plugin", { pluginId });
}

export async function invokePlugin<T = unknown>(pluginId: string, method: string, params: unknown = null, timeoutMs?: number): Promise<T> {
  return invoke("invoke_plugin", { pluginId, method, params, timeoutMs });
}

export async function invokePluginConnectionAction(config: ConnectionConfig, actionId: string): Promise<PluginConnectionActionResult> {
  return invoke("invoke_plugin_connection_action", { config, actionId });
}

export async function notifyPlugin(pluginId: string, method: string, params: unknown = null): Promise<void> {
  return invoke("notify_plugin", { pluginId, method, params });
}

export async function sendPluginBinary(pluginId: string, channel: string, dataBase64: string): Promise<void> {
  return invoke("send_plugin_binary", { pluginId, channel, dataBase64 });
}

export interface PluginLocalFileHandle {
  /** Opaque uuid string from the Rust registry — never a number (JS doubles lose precision above 2^53). */
  handleId: string;
  name: string;
  size: number;
  contentType: string;
  write: boolean;
  /** Only for files expanded out of a dropped folder: '/'-separated path relative to the dropped folder root. */
  relativePath?: string;
}

export interface PluginLocalFileChunk {
  dataBase64: string;
  length: number;
  eof: boolean;
}

export interface PluginLocalFileWriteResult {
  written: number;
  nextOffset: number;
}

// The native open/save dialogs run on the Rust side: the host never passes
// paths into the plugin-file registry, it only receives handles for what the
// user picked. Only the OS drop flow goes through openDroppedPluginLocalFiles:
// the Rust command accepts exactly the paths its own drop pipeline granted to
// this webview (one open attempt per granted path); a granted folder expands
// to its contained files on the Rust side, and `truncated` flags any cap
// cutoff so partial delivery is visible to the plugin.
export interface PluginDroppedFilesResult {
  dropId: string;
  files: PluginLocalFileHandle[];
  truncated: boolean;
}

export async function openDroppedPluginLocalFiles(pluginId: string, paths: string[]): Promise<PluginDroppedFilesResult> {
  return invoke("plugin_file_open_dropped", { pluginId, paths });
}

export async function pickPluginLocalFiles(pluginId: string, multiple: boolean): Promise<PluginLocalFileHandle[]> {
  return invoke("plugin_file_pick_files", { pluginId, multiple });
}

export async function savePluginLocalFileAs(pluginId: string, defaultFileName: string): Promise<PluginLocalFileHandle | null> {
  return invoke("plugin_file_save_as", { pluginId, defaultFileName });
}

export async function readPluginLocalFileChunk(pluginId: string, handleId: string, offset: number, length?: number): Promise<PluginLocalFileChunk> {
  return invoke("plugin_file_read", { pluginId, handleId, offset, length });
}

export async function writePluginLocalFileChunk(pluginId: string, handleId: string, offset: number, dataBase64: string): Promise<PluginLocalFileWriteResult> {
  return invoke("plugin_file_write", { pluginId, handleId, offset, dataBase64 });
}

export async function closePluginLocalFile(pluginId: string, handleId: string): Promise<void> {
  return invoke("plugin_file_close", { pluginId, handleId });
}

export async function openPluginMedia(pluginId: string, method: string, params: Record<string, unknown>): Promise<string> {
  return invoke("plugin_media_open", { pluginId, method, params });
}

export async function closePluginMedia(pluginId: string, token: string): Promise<void> {
  return invoke("plugin_media_close", { pluginId, token });
}

export async function getPluginUiStorage(pluginId: string, key: string): Promise<unknown> {
  return invoke("plugin_ui_storage_get", { pluginId, key });
}

export async function setPluginUiStorage(pluginId: string, key: string, value: unknown): Promise<void> {
  return invoke("plugin_ui_storage_set", { pluginId, key, value });
}

export async function deletePluginUiStorage(pluginId: string, key: string): Promise<void> {
  return invoke("plugin_ui_storage_delete", { pluginId, key });
}

export async function listPluginFilesystemEntries(pluginId: string, providerId: string, options: { connectionId?: string; uri?: string; cursor?: string; limit?: number } = {}): Promise<PluginFilesystemListResult> {
  return invoke("list_plugin_filesystem_entries", { pluginId, providerId, ...options });
}

export async function readPluginFilesystemFile(pluginId: string, providerId: string, uri: string, options: { connectionId?: string; maxBytes?: number } = {}): Promise<PluginFilesystemReadResult> {
  return invoke("read_plugin_filesystem_file", { pluginId, providerId, uri, ...options });
}

export async function writePluginFilesystemFile(pluginId: string, providerId: string, uri: string, dataBase64: string, options: { connectionId?: string; create?: boolean; overwrite?: boolean; etag?: string } = {}): Promise<PluginFilesystemMutationResult> {
  return invoke("write_plugin_filesystem_file", { pluginId, providerId, uri, dataBase64, create: options.create === true, overwrite: options.overwrite === true, connectionId: options.connectionId, etag: options.etag });
}

export async function createPluginFilesystemDirectory(pluginId: string, providerId: string, uri: string, connectionId?: string): Promise<PluginFilesystemMutationResult> {
  return invoke("create_plugin_filesystem_directory", { pluginId, providerId, uri, connectionId });
}

export async function deletePluginFilesystemEntry(pluginId: string, providerId: string, uri: string, options: { connectionId?: string; recursive?: boolean } = {}): Promise<PluginFilesystemMutationResult> {
  return invoke("delete_plugin_filesystem_entry", { pluginId, providerId, uri, connectionId: options.connectionId, recursive: options.recursive === true });
}

export async function renamePluginFilesystemEntry(pluginId: string, providerId: string, sourceUri: string, targetUri: string, options: { connectionId?: string; overwrite?: boolean } = {}): Promise<PluginFilesystemMutationResult> {
  return invoke("rename_plugin_filesystem_entry", { pluginId, providerId, sourceUri, targetUri, connectionId: options.connectionId, overwrite: options.overwrite === true });
}

export async function readPluginAsset(pluginId: string, path: string): Promise<PluginUiAssetPayload> {
  return invoke("read_plugin_asset", { pluginId, path });
}

export async function readPluginUiEntry(pluginId: string): Promise<PluginUiAssetPayload> {
  return invoke("read_plugin_ui_entry", { pluginId });
}

export async function readPluginUiAsset(pluginId: string, path: string): Promise<PluginUiAssetPayload> {
  return invoke("read_plugin_ui_asset", { pluginId, path });
}

export async function subscribePluginEvents(onEvent: (event: PluginEvent) => void, onBinary?: (event: PluginBinaryEvent) => void): Promise<UnlistenFn> {
  const unlistenEvent = await listen<PluginEvent>("dbx-plugin-event", (event) => onEvent(event.payload));
  const unlistenBinary = await listen<PluginBinaryEvent>("dbx-plugin-binary", (event) => onBinary?.(event.payload));
  return () => {
    unlistenEvent();
    unlistenBinary();
  };
}

/**
 * Subscribe to backend connection-liveness messages (#4339).
 *
 * The listener receives the message unwrapped; the web transport's SSE handler parses the same
 * shape, so the store can stay transport-agnostic.
 */
export async function subscribeConnectionLiveness(onEvent: (event: ConnectionLivenessMessage) => void): Promise<UnlistenFn> {
  return listen<ConnectionLivenessMessage>("dbx-connection-liveness", (event) => onEvent(event.payload));
}

export async function listJdbcDrivers(): Promise<JdbcDriverInfo[]> {
  return invoke("list_jdbc_drivers");
}

export async function listJdbcMavenBundles(): Promise<JdbcMavenBundleInfo[]> {
  return invoke("list_jdbc_maven_bundles");
}

export async function listJdbcLocalBundles(): Promise<JdbcLocalBundleInfo[]> {
  return invoke("list_jdbc_local_bundles");
}

export async function importJdbcDrivers(paths: (string | File)[]): Promise<JdbcDriverInfo[]> {
  if (paths.some((path) => typeof path !== "string")) {
    throw new Error("Desktop JDBC driver import requires local file paths");
  }
  return invoke("import_jdbc_drivers", { paths });
}

export async function installJdbcDriverFromMaven(coordinate: string, repositories: string[] = []): Promise<JdbcDriverInfo[]> {
  return invoke("install_jdbc_driver_from_maven", {
    request: { coordinate, repositories },
  });
}

export async function installPrestoSqlJdbcDriver(): Promise<JdbcDriverInfo[]> {
  return invoke("install_prestosql_jdbc_driver");
}

export async function deleteJdbcDriver(path: string): Promise<JdbcDriverInfo[]> {
  return invoke("delete_jdbc_driver", { path });
}

export async function deleteJdbcMavenBundle(bundleId: string): Promise<JdbcDriverInfo[]> {
  return invoke("delete_jdbc_maven_bundle", { bundleId });
}

export async function deleteJdbcLocalBundle(bundleId: string): Promise<JdbcDriverInfo[]> {
  return invoke("delete_jdbc_local_bundle", { bundleId });
}

export async function jdbcPluginStatus(): Promise<JdbcPluginStatus> {
  return invoke("jdbc_plugin_status");
}

export async function installJdbcPlugin(): Promise<JdbcPluginStatus> {
  return invoke("install_jdbc_plugin");
}

export async function installJdbcPluginLocal(path: string | File): Promise<JdbcPluginStatus> {
  if (typeof path !== "string") {
    throw new Error("Desktop JDBC plugin install requires a local file path");
  }
  return invoke("install_jdbc_plugin_local", { path });
}

export async function uninstallJdbcPlugin(): Promise<JdbcPluginStatus> {
  return invoke("uninstall_jdbc_plugin");
}

export async function listInstalledAgentsLocal(): Promise<AgentDriverInfo[]> {
  return invoke("list_installed_agents_local");
}

export async function listInstalledAgents(source?: UpdateDownloadSource): Promise<AgentDriverInfo[]> {
  return invoke("list_installed_agents", { source });
}

export async function isAgentInstalled(dbType: string): Promise<boolean> {
  return invoke("is_agent_installed", { dbType });
}

export async function getDriverStoreUsage(): Promise<DriverStoreUsage> {
  return invoke("get_driver_store_usage");
}

export async function clearDriverDownloadCache(): Promise<void> {
  return invoke("clear_driver_download_cache");
}

export async function getDriverRuntimeSummary(): Promise<DriverRuntimeSummary> {
  return invoke("get_driver_runtime_summary");
}

export async function stopDriverRuntime(runtimeId: string): Promise<void> {
  return invoke("stop_driver_runtime", { runtimeId });
}

export async function restartDriverRuntime(runtimeId: string): Promise<void> {
  return invoke("restart_driver_runtime", { runtimeId });
}

export async function installAgent(dbType: string, source?: UpdateDownloadSource, operationId?: string): Promise<void> {
  return invoke("install_agent", { dbType, source, operationId });
}

export async function upgradeAllAgents(source?: UpdateDownloadSource, operationId?: string): Promise<UpgradeAllAgentDriversResult> {
  return invoke("upgrade_all_agents", { source, operationId });
}

export async function cancelAgentInstall(dbType: string, operationId?: string): Promise<void> {
  return invoke("cancel_agent_install", { dbType, operationId });
}

export async function cancelAgentUpgradeAll(operationId?: string): Promise<void> {
  return invoke("cancel_agent_upgrade_all", { operationId });
}

export async function checkAgentUpdateBlockers(dbTypes: string[]): Promise<AgentUpdateBlocker[]> {
  return invoke("check_agent_update_blockers", { dbTypes });
}

export async function uninstallAgent(dbType: string): Promise<void> {
  return invoke("uninstall_agent", { dbType });
}

export async function getAgentJavaRuntimeConfig(): Promise<JavaRuntimeConfig> {
  return invoke("get_agent_java_runtime_config");
}

export async function setAgentJavaRuntimeConfig(config: JavaRuntimeConfig): Promise<JavaRuntimeConfig> {
  return invoke("set_agent_java_runtime_config", { config });
}

export async function invalidateAgentRegistryCache(): Promise<void> {
  return invoke("invalidate_agent_registry_cache");
}

export async function importAgentsFromZip(path: string | File, operationId?: string): Promise<AgentOfflineImportResult> {
  if (typeof path !== "string") {
    throw new Error("Desktop offline package import requires a local file path");
  }
  return invoke("import_agents_from_zip", { path, operationId });
}

export async function previewAgentOfflineExport(): Promise<AgentOfflineExportPreview> {
  return invoke("preview_agent_offline_export");
}

export async function exportAgentsOffline(path: string, driverKeys: string[]): Promise<AgentOfflineExportResult> {
  return invoke("export_agents_offline", { path, driverKeys });
}

export async function importAgentDriver(dbType: string, path: string | File): Promise<void> {
  if (typeof path !== "string") {
    throw new Error("Desktop driver import requires a local file path");
  }
  return invoke("import_agent_driver_cmd", { dbType, path });
}

export const importAgentJar = importAgentDriver;

export async function reinstallJre(jreKey?: string, source?: UpdateDownloadSource, operationId?: string): Promise<void> {
  return invoke("reinstall_jre", { jreKey, source, operationId });
}

export async function uninstallJre(jreKey: string): Promise<void> {
  return invoke("uninstall_jre", { jreKey });
}

export async function listenAgentInstallProgress(handler: (progress: DriverInstallProgress) => void): Promise<UnlistenFn> {
  return listen<DriverInstallProgress>("agent-install-progress", (event) => handler(event.payload));
}

export async function loadSavedSqlLibrary(): Promise<SavedSqlLibrary> {
  return invoke("load_saved_sql_library");
}

export async function loadSavedSqlFilesForSync(): Promise<SavedSqlFile[]> {
  return invoke("load_saved_sql_files_for_sync");
}

export async function loadSavedSqlFile(id: string): Promise<SavedSqlFile | null> {
  return invoke("load_saved_sql_file", { id });
}

export async function saveSavedSqlFolder(folder: SavedSqlFolder): Promise<SavedSqlFolder> {
  return invoke("save_saved_sql_folder", { folder });
}

export async function deleteSavedSqlFolder(id: string): Promise<void> {
  return invoke("delete_saved_sql_folder", { id });
}

export async function saveSavedSqlFile(file: SavedSqlFile): Promise<SavedSqlFile> {
  return invoke("save_saved_sql_file", { file });
}

export async function deleteSavedSqlFile(id: string): Promise<void> {
  return invoke("delete_saved_sql_file", { id });
}

export async function savedSqlStorageDir(): Promise<string> {
  return invoke("saved_sql_storage_dir");
}

export async function openSavedSqlStorageDir(dir?: string | null): Promise<void> {
  return invoke("open_saved_sql_storage_dir", { dir });
}

export async function revealPathInFileManager(path: string): Promise<void> {
  return invoke("reveal_path_in_file_manager", { path });
}

export async function deleteDatabaseBackupFiles(paths: string[], allowedRoots: string[] = []): Promise<number> {
  return invoke("delete_database_backup_files", { paths, allowedRoots });
}

export async function isSqliteDatabaseFile(path: string): Promise<boolean> {
  return invoke("is_sqlite_database_file", { path });
}

export async function backupSqliteDatabase(connectionId: string, destinationPath: string): Promise<void> {
  return invoke("backup_sqlite_database", { connectionId, destinationPath });
}

export async function restoreSqliteDatabase(connectionId: string, sourcePath: string): Promise<void> {
  return invoke("restore_sqlite_database", { connectionId, sourcePath });
}

export async function syncSavedSqlDirectory(request: SavedSqlSyncRequest): Promise<void> {
  return invoke("sync_saved_sql_directory", { request });
}

export async function saveSidebarLayout(layout: import("@/types/database").SidebarLayout): Promise<void> {
  return invoke("save_sidebar_layout", { layout });
}

export async function loadSidebarLayout(): Promise<import("@/types/database").SidebarLayout | null> {
  return invoke("load_sidebar_layout");
}

export async function saveTableVGroups(scopeKey: string, layout: TableVGroupLayout): Promise<void> {
  return invoke("save_table_vgroups", { scopeKey, layout });
}

export async function loadTableVGroups(): Promise<Record<string, import("@/types/database").TableVGroupLayout>> {
  return invoke("load_table_vgroups");
}

export async function deleteTableVGroupsForConnection(connectionId: string): Promise<void> {
  return invoke("delete_table_vgroups_for_connection", { connectionId });
}

// --- Updates ---
export interface UpdateInfo {
  current_version: string;
  latest_version: string;
  update_available: boolean;
  portable_mode: boolean;
  manual_update_only: boolean;
  release_name: string;
  release_url: string;
  release_notes: string;
}

export type UpdateDownloadSource = "official" | "cnb";

export interface DownloadedUpdate {
  cache_id: string;
  version: string;
  portable_mode: boolean;
  release_url: string;
  release_notes: string;
  downloaded_at: number;
}

export interface UpdateDownloadProgress {
  attempt_id: string;
  version: string;
  downloaded: number;
  total: number | null;
}

export interface McpServerStatus {
  installed: boolean;
  installation_source: "native" | "homebrew" | "npm" | null;
  npm_available: boolean;
  npm_installed: boolean;
  node_path: string | null;
  node_version: string | null;
  current_version: string | null;
  latest_version: string | null;
  update_available: boolean;
  bin_path: string | null;
  native_bin_path: string | null;
  script_path: string | null;
  data_dir: string | null;
  install_command: string;
  update_command: string;
  uninstall_command: string;
  error: string | null;
}

export async function checkMcpServerStatus(): Promise<McpServerStatus> {
  return invoke("check_mcp_server_status");
}

export async function checkForUpdates(locale?: string, source?: UpdateDownloadSource): Promise<UpdateInfo> {
  return invoke("check_for_updates", { locale, source });
}

export async function fetchChangelog(lang?: string): Promise<import("@/lib/app/changelog").ChangelogData> {
  return invoke("fetch_changelog", { lang });
}

export async function getSystemProxyUrl(): Promise<string | null> {
  return invoke("get_system_proxy_url");
}

export async function downloadUpdate(source: UpdateDownloadSource, latestVersion: string, attemptId: string, releaseNotes?: string): Promise<DownloadedUpdate> {
  return invoke("download_update", { source, latestVersion, attemptId, releaseNotes });
}

export async function cancelUpdateDownload(): Promise<void> {
  return invoke("cancel_update_download");
}

export async function getDownloadedUpdate(): Promise<DownloadedUpdate | null> {
  return invoke("get_downloaded_update");
}

export async function discardDownloadedUpdate(cacheId: string): Promise<void> {
  return invoke("discard_downloaded_update", { cacheId });
}

export async function installDownloadedUpdate(cacheId: string, expectedVersion: string): Promise<void> {
  return invoke("install_downloaded_update", { cacheId, expectedVersion });
}

export async function getAppVersion(): Promise<string> {
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

export async function getAppSupportInfo(): Promise<AppSupportInfo> {
  const info = await invoke<AppSupportInfo>("get_app_support_info");
  return { ...info, userAgent: collectBrowserSupportInfo() };
}

// --- Redis ---
export interface RedisKeyInfo {
  key_display: string;
  key_raw: string;
  key_type?: string;
  ttl?: number;
  size?: number;
  value_preview?: string;
}

export interface RedisDatabaseInfo {
  db: number;
  /** 该库的键数量；服务端无法给出可信数量时（如 kvrocks 未执行过 DBSIZE SCAN）缺省。 */
  keys?: number;
}

export type RedisBlobEncoding = "utf8" | "binary";

export interface RedisBlob {
  raw_base64: string;
  encoding: RedisBlobEncoding;
}

export interface RedisListItem {
  index: number;
  value: RedisBlob;
}

export interface RedisSetItem {
  member: RedisBlob;
}

export interface RedisHashItem {
  field: RedisBlob;
  value: RedisBlob;
  field_ttl?: number;
}

export interface RedisZsetItem {
  score: string;
  member: RedisBlob;
}

export interface RedisKeysExpiryResult {
  applied: number;
  missing_key_raws: string[];
}

export interface RedisStreamField {
  field: string;
  value: string;
}

export interface RedisStreamEntry {
  id: string;
  fields: RedisStreamField[];
}

export interface RedisStreamPage {
  entries: RedisStreamEntry[];
  next_cursor?: string;
}

// Redis counters above Number.MAX_SAFE_INTEGER are transported as decimal strings.
export type RedisStreamMetric = number | string;

export interface RedisStreamGroup {
  name: RedisBlob;
  consumers: RedisStreamMetric;
  pending: RedisStreamMetric;
  last_delivered_id: string;
  entries_read?: RedisStreamMetric;
  lag?: RedisStreamMetric;
}

export interface RedisStreamConsumer {
  name: RedisBlob;
  pending: RedisStreamMetric;
  idle_ms: RedisStreamMetric;
  inactive_ms?: RedisStreamMetric;
}

export interface RedisStreamPendingEntry {
  id: string;
  consumer: RedisBlob;
  idle_ms: RedisStreamMetric;
  deliveries: RedisStreamMetric;
}

export interface RedisStreamPendingPage {
  entries: RedisStreamPendingEntry[];
  next_cursor?: string;
}

export type RedisValueData =
  | { kind: "string"; content: RedisBlob; total_bytes?: number; truncated?: boolean }
  | { kind: "json"; value: string }
  | {
      kind: "list";
      items: RedisListItem[];
      total: number;
      scan_cursor?: number;
    }
  | { kind: "set"; items: RedisSetItem[]; total: number; scan_cursor?: number }
  | {
      kind: "hash";
      items: RedisHashItem[];
      total: number;
      scan_cursor?: number;
    }
  | {
      kind: "zset";
      items: RedisZsetItem[];
      total: number;
      scan_cursor?: number;
    }
  | { kind: "stream"; entries: RedisStreamEntry[]; total?: number; next_cursor?: string }
  // kvrocks 把位图/HLL 实现为独立类型（TYPE 返回 bitmap / hyperloglog），
  // 这里单独建模，避免落到 unknown 后界面显示不出值。
  | { kind: "bitmap"; content: RedisBlob; total_bytes?: number; truncated?: boolean; set_bits?: number }
  | { kind: "hyperloglog"; count?: number }
  | { kind: "unknown"; redis_type: string };

export interface RedisValue {
  key_display: string;
  key_raw: string;
  ttl: number;
  redis_type: string;
  data: RedisValueData;
}

export type RedisCollectionPage = { kind: "list"; items: RedisListItem[]; scan_cursor?: number } | { kind: "set"; items: RedisSetItem[]; scan_cursor?: number } | { kind: "hash"; items: RedisHashItem[]; scan_cursor?: number } | { kind: "zset"; items: RedisZsetItem[]; scan_cursor?: number };

export interface RedisScanResult {
  cursor: number;
  keys: RedisKeyInfo[];
  total_keys: number;
}

export type RedisCommandSafety = "allowed" | "write" | "confirm" | "blocked";

export interface RedisCommandResult {
  command: string;
  safety: RedisCommandSafety;
  value: any;
}

export interface RedisSlowlogEntry {
  id: number;
  timestamp: number;
  duration_micros: number;
  command: string;
  client_addr: string | null;
  client_name: string | null;
}

export interface RedisNodeEndpoint {
  host: string;
  port: number;
}

// --- etcd ---
export type KvValueEncoding = "utf8" | "base64";
export type KvInt64 = string;

export interface KvValue {
  encoding: KvValueEncoding;
  data: string;
}

export interface KvKeyMetadata {
  createRevision?: KvInt64 | number | null;
  modRevision?: KvInt64 | number | null;
  version?: KvInt64 | number | null;
  lease?: KvInt64 | number | null;
  ttl?: number | null;
  valueSize?: number | null;
  czxid?: number | null;
  mzxid?: number | null;
  pzxid?: number | null;
  ctime?: number | null;
  mtime?: number | null;
  cversion?: number | null;
  aversion?: number | null;
  ephemeralOwner?: number | null;
  dataLength?: number | null;
  numChildren?: number | null;
  flags?: KvInt64 | number | null;
  lockIndex?: KvInt64 | number | null;
  session?: string | null;
}

export interface KvKeySummary extends KvKeyMetadata {
  key: string;
  keyIdentity?: string | null;
  keyBytes?: KvValue | null;
  value?: KvValue | null;
}

export interface KvListPrefixResponse {
  keys: KvKeySummary[];
  continuation?: string | null;
  revision?: KvInt64 | number | null;
  filteredByAcls?: boolean | null;
}

export interface KvListPrefixOptions {
  recursive?: boolean | null;
  revision?: KvInt64 | null;
  includeValues?: boolean | null;
}

export interface KvGetResponse {
  found: boolean;
  key?: string | null;
  keyIdentity?: string | null;
  keyBytes?: KvValue | null;
  value?: KvValue | null;
  metadata?: KvKeyMetadata | null;
}

export interface KvGetOptions {
  metadataOnly?: boolean | null;
  keyBytes?: KvValue | null;
  revision?: KvInt64 | null;
}

export interface KvPutResponse {
  revision?: number | null;
  version?: number | null;
  mtime?: number | null;
  key?: string | null;
  createdKey?: string | null;
}

export type KvWriteMode = "upsert" | "create" | "update";
export type KvCreateMode = "persistent" | "ephemeral" | "persistent_sequential" | "ephemeral_sequential";

export interface KvPutOptions {
  lease?: KvInt64 | number | null;
  ttl?: number | null;
  preserveLease?: boolean | null;
  writeMode?: KvWriteMode | null;
  createMode?: KvCreateMode | null;
  keyBytes?: KvValue | null;
  expectedModRevision?: KvInt64 | null;
  expectedCreateRevision?: KvInt64 | null;
  flags?: KvInt64 | null;
}

export interface KvDeleteOptions {
  keyBytes?: KvValue | null;
  expectedModRevision?: KvInt64 | null;
}

export interface KvDeleteResponse {
  deleted: number;
  revision?: KvInt64 | number | null;
}

export type KvHistoryEventType = "put" | "delete";
export interface KvHistoryEvent {
  eventType: KvHistoryEventType;
  revision: KvInt64;
  value?: KvValue | null;
  previousValue?: KvValue | null;
  metadata?: KvKeyMetadata | null;
}
export interface KvHistoryResponse {
  events: KvHistoryEvent[];
  observedRevision: KvInt64;
  truncated: boolean;
}
export interface KvStatusMember {
  endpoint: string;
  memberId?: KvInt64 | null;
  name?: string | null;
  version?: string | null;
  leaderId?: KvInt64 | null;
  revision?: KvInt64 | null;
  raftTerm?: KvInt64 | null;
  raftIndex?: KvInt64 | null;
  raftAppliedIndex?: KvInt64 | null;
  dbSize?: KvInt64 | null;
  dbSizeInUse?: KvInt64 | null;
  learner: boolean;
  reachable: boolean;
  latencyMs?: number | null;
  errors: string[];
}
export interface KvPrometheusMetrics {
  available: boolean;
  sourceUrl?: string | null;
  error?: string | null;
  collectedAtMs?: number | null;
  sampleCount?: number | null;
  serverVersion?: string | null;
  clusterVersion?: string | null;
  goVersion?: string | null;
  authRevision?: number | null;
  hasLeader?: number | null;
  isLeader?: number | null;
  leaderChangesTotal?: number | null;
  proposalsCommittedTotal?: number | null;
  proposalsAppliedTotal?: number | null;
  proposalsPending?: number | null;
  proposalsFailedTotal?: number | null;
  grpcRequestsTotal?: number | null;
  grpcFailuresTotal?: number | null;
  grpcMethodRequestsTotal: Record<string, number>;
  grpcMethodFailuresTotal: Record<string, number>;
  requestDurationSecondsSumByType: Record<string, number>;
  requestDurationSecondsCountByType: Record<string, number>;
  mvccPutTotal?: number | null;
  mvccDeleteTotal?: number | null;
  mvccRangeTotal?: number | null;
  mvccTxnTotal?: number | null;
  mvccCurrentRevision?: number | null;
  mvccCompactRevision?: number | null;
  mvccKeysTotal?: number | null;
  mvccEventsTotal?: number | null;
  mvccPendingEventsTotal?: number | null;
  mvccSlowWatcherTotal?: number | null;
  mvccWatchStreamTotal?: number | null;
  mvccWatcherTotal?: number | null;
  mvccTotalPutSizeBytes?: number | null;
  openReadTransactions?: number | null;
  leaseGrantedTotal?: number | null;
  leaseRenewedTotal?: number | null;
  leaseRevokedTotal?: number | null;
  leaseExpiredTotal?: number | null;
  leaseTtlSecondsSum?: number | null;
  leaseTtlSecondsCount?: number | null;
  clientReceivedBytesTotal?: number | null;
  clientSentBytesTotal?: number | null;
  peerReceivedBytesTotal?: number | null;
  peerSentBytesTotal?: number | null;
  peerReceivedFailuresTotal?: number | null;
  peerSentFailuresTotal?: number | null;
  walFsyncDurationSecondsSum?: number | null;
  walFsyncDurationSecondsCount?: number | null;
  walWriteBytesTotal?: number | null;
  walWriteDurationSecondsSum?: number | null;
  walWriteDurationSecondsCount?: number | null;
  backendCommitDurationSecondsSum?: number | null;
  backendCommitDurationSecondsCount?: number | null;
  backendSnapshotDurationSecondsSum?: number | null;
  backendSnapshotDurationSecondsCount?: number | null;
  backendDefragDurationSecondsSum?: number | null;
  backendDefragDurationSecondsCount?: number | null;
  diskDefragInflight?: number | null;
  snapshotApplyInProgress?: number | null;
  quotaBackendBytes?: number | null;
  knownPeers?: number | null;
  heartbeatSendFailuresTotal?: number | null;
  readIndexesFailedTotal?: number | null;
  slowApplyTotal?: number | null;
  slowReadIndexesTotal?: number | null;
  healthSuccessTotal?: number | null;
  healthFailuresTotal?: number | null;
  residentMemoryBytes?: number | null;
  virtualMemoryBytes?: number | null;
  cpuSecondsTotal?: number | null;
  processStartTimeSeconds?: number | null;
  processReceivedBytesTotal?: number | null;
  processTransmittedBytesTotal?: number | null;
  openFds?: number | null;
  maxFds?: number | null;
  goroutines?: number | null;
  goThreads?: number | null;
  goMaxProcs?: number | null;
  goHeapAllocBytes?: number | null;
  goHeapInuseBytes?: number | null;
  goHeapSysBytes?: number | null;
  goHeapObjects?: number | null;
  goNextGcBytes?: number | null;
  goGcDurationSecondsSum?: number | null;
  goGcDurationSecondsCount?: number | null;
  dbSizeMetricBytes?: number | null;
  dbSizeInUseMetricBytes?: number | null;
}
export interface KvStatusResponse {
  clusterId?: KvInt64 | null;
  revision?: KvInt64 | null;
  leaderId?: KvInt64 | null;
  keyCount?: KvInt64 | null;
  alarms: string[];
  members: KvStatusMember[];
  metrics?: KvPrometheusMetrics | null;
}

export interface EtcdDefragMemberResult {
  endpoint: string;
  status: "succeeded" | "failed" | "not_executed";
  durationMs?: number | null;
  error?: string | null;
}
export interface EtcdDefragResponse {
  members: EtcdDefragMemberResult[];
}
export interface EtcdWatchStartRequest {
  key: string;
  keyBytes?: KvValue | null;
  scope: "key" | "prefix";
  startRevision?: KvInt64 | null;
  includePrevKv: boolean;
}
export interface EtcdWatchStartResponse {
  watchId: string;
  startedRevision: KvInt64;
}
export interface EtcdWatchPollResponse {
  watchId: string;
  batches: Array<{ revision: KvInt64; events: Array<{ eventType: "put" | "delete"; revision: KvInt64; key: string; keyBytes?: KvValue | null; value?: KvValue | null; previousValue?: KvValue | null; metadata?: KvKeyMetadata | null }> }>;
  terminal?: { reason: string; message?: string; compactedRevision?: KvInt64 | null } | null;
}
export interface EtcdLeaseListResponse {
  leases: Array<{ id: KvInt64; ttl: number; grantedTtl?: number }>;
  partial: boolean;
  nextContinuation?: string | null;
}
export interface EtcdLeaseDetail {
  id: KvInt64;
  ttl: number;
  grantedTtl?: number;
  keys: KvValue[];
  truncated: boolean;
}
export interface EtcdAuthUserListResponse {
  users: string[];
}
export interface EtcdAuthUserDetail {
  user: string;
  roles: string[];
  authEnabled?: boolean;
}
export interface EtcdAuthPermission {
  access: "read" | "write" | "readwrite";
  key: KvValue;
  rangeEnd: KvValue;
  resource: "all" | "key" | "prefix";
}
export interface EtcdAuthRoleListResponse {
  roles: string[];
}
export interface EtcdAuthRoleDetail {
  role: string;
  permissions: EtcdAuthPermission[];
}
export interface EtcdPreflightResponse {
  token: string;
  action: string;
  confirmationText: string;
  expiresAtMs: number;
  clusterId?: KvInt64 | null;
}
export interface EtcdDangerousApproval {
  preflightToken: string;
  confirmationText: string;
}

// --- ZooKeeper ---

// --- Consul KV ---

// --- HBase ---

// --- Document stores ---
export interface DocumentQueryResult {
  documents: any[];
  raw_documents?: string[];
  extended_documents?: any[];
  total: number;
  total_is_exact?: boolean;
  next_cursor?: string;
}

export interface DynamoDbKeyInfo {
  name: string;
  attributeType: "S" | "N" | "B" | string;
}

export interface DynamoDbIndexInfo {
  name: string;
  kind: "global" | "local" | string;
  partitionKey: DynamoDbKeyInfo;
  sortKey?: DynamoDbKeyInfo;
  projectionType: "ALL" | "KEYS_ONLY" | "INCLUDE" | string;
  nonKeyAttributes: string[];
}

export interface DynamoDbTableDescription {
  name: string;
  status: string;
  itemCount: number;
  sizeBytes: number;
  partitionKey: DynamoDbKeyInfo;
  sortKey?: DynamoDbKeyInfo;
  indexes: DynamoDbIndexInfo[];
}

// Kept for callers that are specifically using MongoDB APIs.
export type MongoDocumentResult = DocumentQueryResult;

export interface MongoCollectionStatsResult {
  count: unknown;
  size: unknown;
  avgObjSize: unknown;
  storageSize: unknown;
  totalIndexSize: unknown;
  nindexes: unknown;
}

export interface MongoDropIndexFailure {
  name: string;
  message: string;
}

export interface MongoDropIndexesResult {
  dropped_names: string[];
  affected_rows: number;
  failures?: MongoDropIndexFailure[];
}

export interface MongoIndexKey {
  field: string;
  /** `1`, `-1`, or a MongoDB key type such as `text` / `2dsphere` / `hashed`. */
  direction: string;
}

/** Full MongoDB index specification, carrying the options `IndexInfo` cannot hold. */
export interface MongoIndexSpec {
  name: string;
  keys: MongoIndexKey[];
  is_unique: boolean;
  is_primary: boolean;
  is_sparse: boolean;
  /** TTL in seconds; null when the index does not expire. */
  expire_after_seconds: number | null;
  partial_filter_expression: string | null;
  /** Ignored by MongoDB 4.2+, still reported by older servers. */
  background: boolean;
  /** Only meaningful for geoHaystack indexes, removed in MongoDB 4.4+. */
  bucket_size: number | null;
  hidden: boolean;
  /** False when the driver could not report the properties above (Legacy Agent). */
  properties_complete: boolean;
  extra_options: string | null;
}

export interface MongoCloneCollectionResult {
  documents_copied: number;
  indexes_copied: number;
}

export interface MongoGridFsFileInfo {
  id: string;
  filename?: string;
  length: number;
  chunkSize: number;
  uploadDate?: string;
  metadata?: any;
  md5?: string;
  contentType?: string;
  aliases?: string[];
}

export interface MongoGridFsBucketInfo {
  name: string;
  fileCount: number;
  totalBytes: number;
}

export async function documentListDatabases(connectionId: string): Promise<string[]> {
  return invoke("document_list_databases", { connectionId });
}

export async function documentListCollections(connectionId: string, database: string): Promise<CollectionInfo[]> {
  return invoke("document_list_collections", { connectionId, database });
}

export async function vectorGetCollectionDetail(connectionId: string, database: string, collection: string): Promise<CollectionInfo> {
  return invoke("vector_collection_detail", {
    connectionId,
    database,
    collection,
  });
}

export async function vectorDropDatabase(connectionId: string, database: string): Promise<void> {
  return invoke("vector_drop_database", { connectionId, database });
}

export async function vectorDropCollection(connectionId: string, database: string, collection: string): Promise<void> {
  return invoke("vector_drop_collection", { connectionId, database, collection });
}

export async function vectorRenameCollection(connectionId: string, database: string, collection: string, newName: string): Promise<void> {
  return invoke("vector_rename_collection", { connectionId, database, collection, newName });
}

/** Lists every Meilisearch index visible to the current connection credentials. */

export async function vectorListCollections(connectionId: string, database?: string): Promise<CollectionInfo[]> {
  return documentListCollections(connectionId, database || "default");
}

export async function documentFindDocuments(
  connectionId: string,
  database: string,
  collection: string,
  skip: number,
  limit: number,
  filter?: string,
  projection?: string,
  sort?: string,
  collation?: string,
  executionId?: string,
  cursor?: string,
  cursorPagination?: boolean,
): Promise<DocumentQueryResult> {
  return invoke("document_find_documents", {
    connectionId,
    database,
    collection,
    skip,
    limit,
    filter,
    projection,
    sort,
    collation,
    cursor,
    cursorPagination,
    executionId,
  });
}

export async function documentCountDocuments(connectionId: string, collection: string, filter?: string, executionId?: string): Promise<number> {
  return invoke("document_count_documents", {
    connectionId,
    collection,
    filter,
    executionId,
  });
}

export async function dynamodbDescribeTable(connectionId: string, table: string): Promise<DynamoDbTableDescription> {
  return invoke("dynamodb_describe_table", { connectionId, table });
}

/** Read-only index metadata endpoints exposed on the Elasticsearch index context menu. */
export type ElasticsearchIndexMetadataKind = "mapping" | "settings" | "stats";

/** Outcome of clearing an index: mapping and settings are kept, documents are not. */
export interface ElasticsearchDeleteByQueryResult {
  total: number;
  deleted: number;
  versionConflicts: number;
  timedOut: boolean;
  failures: string[];
}

export async function documentListGridFsFiles(connectionId: string, database: string, bucket: string, filter?: string, sort?: string): Promise<MongoGridFsFileInfo[]> {
  return invoke("document_list_gridfs_files", {
    connectionId,
    database,
    bucket,
    filter,
    sort,
  });
}

export async function documentListGridFsBuckets(connectionId: string, database: string, filter?: string, sort?: string): Promise<MongoGridFsBucketInfo[]> {
  return invoke("document_list_gridfs_buckets", {
    connectionId,
    database,
    filter,
    sort,
  });
}

export async function documentCreateGridFsBucket(connectionId: string, database: string, bucket: string): Promise<void> {
  return invoke("document_create_gridfs_bucket", {
    connectionId,
    database,
    bucket,
  });
}

export async function documentDeleteGridFsBucket(connectionId: string, database: string, bucket: string): Promise<void> {
  return invoke("document_delete_gridfs_bucket", {
    connectionId,
    database,
    bucket,
  });
}

export async function documentDownloadGridFsFile(connectionId: string, database: string, bucket: string, fileId: string): Promise<Uint8Array> {
  const data = await invoke<number[]>("document_download_gridfs_file", {
    connectionId,
    database,
    bucket,
    fileId,
  });
  return new Uint8Array(data);
}

export async function documentUploadGridFsFile(connectionId: string, database: string, bucket: string, fileName: string, data: Uint8Array, contentType?: string): Promise<string> {
  return invoke("document_upload_gridfs_file", {
    connectionId,
    database,
    bucket,
    fileName,
    data: Array.from(data),
    contentType,
  });
}

export async function documentDeleteGridFsFile(connectionId: string, database: string, bucket: string, fileId: string): Promise<void> {
  return invoke("document_delete_gridfs_file", {
    connectionId,
    database,
    bucket,
    fileId,
  });
}

export async function documentInsertDocument(connectionId: string, database: string, collection: string, docJson: string, routing?: string, preserveBsonTypes?: boolean): Promise<string> {
  return invoke("document_insert_document", {
    connectionId,
    database,
    collection,
    docJson,
    routing,
    preserveBsonTypes,
  });
}

export async function documentUpdateDocument(connectionId: string, database: string, collection: string, id: string, docJson: string, routing?: string): Promise<number> {
  return invoke("document_update_document", {
    connectionId,
    database,
    collection,
    id,
    docJson,
    routing,
  });
}

export async function documentDeleteDocument(connectionId: string, database: string, collection: string, id: string, routing?: string, documentType?: string): Promise<number> {
  return invoke("document_delete_document", {
    connectionId,
    database,
    collection,
    id,
    routing,
    documentType,
  });
}

export async function documentSaveMeilisearchBatch(connectionId: string, collection: string, updates: Array<{ id: string; docJson: string }>, deleteIds: string[], inserts: string[]): Promise<number> {
  return invoke("document_save_meilisearch_batch", {
    connectionId,
    collection,
    updates,
    deleteIds,
    inserts,
  });
}

export interface MeilisearchIndexSettings {
  [key: string]: unknown;
  pagination?: {
    maxTotalHits?: number;
  };
}

export interface MeilisearchIndexOverview {
  uid: string;
  primaryKey: string | null;
  createdAt: string | null;
  updatedAt: string | null;
  numberOfDocuments: number;
  isIndexing: boolean;
  /** Raw document store size of this index (Meilisearch >= 1.14); null on older servers. */
  documentSize: number | null;
  /** Average document size of this index (Meilisearch >= 1.14); null on older servers. */
  avgDocumentSize: number | null;
  /** Instance-wide database size; every index shares it, so it is only a fallback. */
  databaseSize: number | null;
}

// --- History ---
export interface HistoryEntry {
  id: string;
  connection_id?: string;
  connection_name: string;
  database: string;
  sql: string;
  executed_at: string;
  execution_time_ms: number;
  success: boolean;
  error?: string;
  activity_kind?: "query" | "data_change" | "schema_change" | "import" | "transfer";
  operation?: string;
  target?: string;
  affected_rows?: number | null;
  rollback_sql?: string | null;
  details_json?: string | null;
  source?: "sql" | "mcp" | "other";
  mcp_tool_name?: string | null;
  mcp_request_json?: string | null;
  mcp_response_json?: string | null;
  mcp_session_id?: string | null;
}

export interface HistoryConnectionFilter {
  connection_id: string;
  connection_name: string;
}

export interface HistoryDatabaseFilter extends HistoryConnectionFilter {
  database: string;
}

export interface HistoryCursor {
  executed_at: string;
  id: string;
}

export interface HistorySearchRequest {
  search_text: string;
  connections: HistoryConnectionFilter[];
  databases: HistoryDatabaseFilter[];
  activity_kind?: string;
  success?: boolean;
  started_at?: string;
  ended_at?: string;
  cursor?: HistoryCursor;
  limit: number;
  source?: "sql" | "mcp" | "other";
  mcp_tool_name?: string;
}

export interface HistorySearchResult {
  entries: HistoryEntry[];
  next_cursor?: HistoryCursor | null;
  total: number;
}

export interface HistoryConnectionOption extends HistoryConnectionFilter {
  databases: string[];
}

export async function saveHistory(entry: HistoryEntry): Promise<void> {
  return invoke("save_history", { entry });
}

export async function loadHistory(limit: number, offset: number, activityKind?: string): Promise<HistoryEntry[]> {
  return invoke("load_history", {
    limit,
    offset,
    activityKind: activityKind ?? null,
  });
}

export async function searchHistory(request: HistorySearchRequest): Promise<HistorySearchResult> {
  return invoke("search_history", { request });
}

export async function loadHistoryConnectionOptions(): Promise<HistoryConnectionOption[]> {
  return invoke("load_history_connection_options");
}

export async function loadRedisHistory(limit = 100, offset = 0): Promise<HistoryEntry[]> {
  return loadHistory(limit, offset, "redis_command");
}

export async function clearHistory(): Promise<void> {
  return invoke("clear_history");
}

export async function clearHistoryBySource(source: string): Promise<void> {
  return invoke("clear_history_by_source", { source });
}

export async function cleanupMcpHistoryRetention(): Promise<number> {
  return invoke("cleanup_mcp_history_retention");
}

export async function clearRedisHistory(): Promise<void> {
  const entries = await loadRedisHistory(1000, 0);
  await Promise.all(entries.map((e) => deleteHistoryEntry(e.id)));
}

export async function deleteHistoryEntry(id: string): Promise<void> {
  return invoke("delete_history_entry", { id });
}

// --- SQL File Execution ---
export type SqlFileStatus = "started" | "running" | "statementDone" | "statementFailed" | "done" | "error" | "cancelled";

export interface SqlFileRequest {
  executionId: string;
  connectionId: string;
  database: string;
  schema?: string;
  filePath: string;
  continueOnError: boolean;
  txnSessionId?: string;
  selectedTables?: SqlFileTable[];
  partCooldownMs?: number;
  skipRelationalConstraints?: boolean;
}

export interface SqlFileTable {
  database: string | null;
  name: string;
}

export async function inspectSqlFileTables(filePath: string): Promise<SqlFileTable[]> {
  return invoke("inspect_sql_file_tables", { filePath });
}

export interface SqlFilePreview {
  fileName: string;
  filePath: string;
  sizeBytes: number;
  preview: string;
  canExecuteWithoutSelectedDatabase: boolean;
  establishesDatabaseContext?: boolean;
  packageFilePaths?: string[];
  packagePartCount?: number;
  cleanupToken?: string;
}

export async function releaseSqlFilePreview(_cleanupToken: string): Promise<void> {}

export interface SqlFileProgress {
  executionId: string;
  status: SqlFileStatus;
  statementIndex: number;
  successCount: number;
  failureCount: number;
  affectedRows: number;
  elapsedMs: number;
  statementSummary: string;
  error?: string | null;
  bytesRead?: number;
  totalBytes?: number;
  phase?: "preparing" | "reading" | "executing";
  fileIndex?: number;
  fileName?: string;
}

export async function previewSqlFile(filePath: string): Promise<SqlFilePreview> {
  return invoke("preview_sql_file", { filePath });
}

export async function executeSqlFile(request: SqlFileRequest): Promise<void> {
  return invoke("execute_sql_file", { request });
}

export async function executeSqlFiles(request: SqlFileRequest, filePaths: string[]): Promise<void> {
  return invoke("execute_sql_files", { request, filePaths });
}

export async function cancelSqlFileExecution(executionId: string): Promise<boolean> {
  return invoke("cancel_sql_file_execution", { executionId });
}

export async function listenSqlFileProgress(handler: (progress: SqlFileProgress) => void): Promise<UnlistenFn> {
  return listen<SqlFileProgress>("sql-file-progress", (event) => handler(event.payload));
}

// --- Data Transfer ---
export type TransferMode = "append" | "overwrite" | "upsert";
export type TransferTableNameCase = "preserve" | "lower" | "upper";
export type TransferOwnershipPolicy = "preserve" | "skip" | "reassignMissing";
export type TransferContent = "structureAndData" | "structureOnly" | "dataOnly";
export type TransferObjectKind = "TABLE" | "VIEW" | "MATERIALIZED_VIEW" | "PROCEDURE" | "FUNCTION" | "TRIGGER" | "SEQUENCE" | "EVENT";

export interface TransferObjectSelection {
  objectType: TransferObjectKind;
  names: string[];
}

export interface TransferRequest {
  transferId: string;
  sourceConnectionId: string;
  sourceDatabase: string;
  sourceSchema: string;
  sourceCatalog?: string;
  targetConnectionId: string;
  targetDatabase: string;
  targetSchema: string;
  targetCatalog?: string;
  tables: string[];
  createTable: boolean;
  content: TransferContent;
  objects: TransferObjectSelection[];
  mode: TransferMode;
  targetTableNameCase: TransferTableNameCase;
  quoteTargetColumnNames: boolean;
  ownershipPolicy?: TransferOwnershipPolicy;
  batchSize: number;
  /**
   * Optional per-source-table transfer filter.
   * Key = source table name; value = a bare `WHERE` predicate
   * (`id <= 90000`) or a complete `SELECT`
   * (`select * from t_order where id <= 90000`). Missing/empty = full table.
   */
  tableFilters?: Record<string, string>;
  dropTargetBeforeCreate: boolean;
  dropTargetConfirmed: boolean;
}

export interface TransferStructurePreviewTable {
  sourceTable: string;
  targetTable: string;
  /** The target table already exists, so this transfer plans no structure DDL for it. */
  preexisting: boolean;
  sql: string;
}

export type TransferStructureOperationKind = "createSchema" | "createTable" | "skipExistingTable" | "rebuildTable" | "createIndex" | "addForeignKey" | "createSequence" | "bindSequence" | "addComment";

export interface TransferStructureOperation {
  kind: TransferStructureOperationKind;
  objectName?: string;
  sourceTable?: string;
  targetTable?: string;
}

export interface TransferStructurePreview {
  sql: string;
  tables: TransferStructurePreviewTable[];
  operations: TransferStructureOperation[];
}

export interface TransferOwnershipPreview {
  missingOwners: string[];
  targetOwner: string;
  rebuild?: {
    sql: string;
    tables: Array<{ sourceTable: string; targetTable: string; backupTable?: string }>;
    /** The rename phase on its own, so the structure plan can sit between rename and cleanup. */
    backupSql?: string;
    /** The drop-backups phase on its own. */
    cleanupSql?: string;
  };
  /** Structure-plan preview: present for structure-only transfers. */
  structure?: TransferStructurePreview;
}

export interface TransferProgress {
  transferId: string;
  table: string;
  tableIndex: number;
  totalTables: number;
  rowsTransferred: number;
  totalRows: number | null;
  status: "running" | "tableDone" | "done" | "error" | "cancelled";
  error: string | null;
  terminal: boolean;
  transferFailuresOmitted?: number;
}

export async function startTransfer(request: TransferRequest, onProgress: (progress: TransferProgress) => void, onStarted?: () => void): Promise<void> {
  return new Promise((resolve, reject) => {
    let unlisten: UnlistenFn | null = null;
    void (async () => {
      try {
        unlisten = await listen<TransferProgress>("transfer-progress", (event) => {
          if (event.payload.transferId !== request.transferId) return;
          onProgress(event.payload);
          if (isTerminalTransferProgress(event.payload)) {
            unlisten?.();
            resolve();
          }
        });

        await invoke("start_transfer", { request });
        onStarted?.();
      } catch (e) {
        unlisten?.();
        reject(e instanceof BackendErrorException ? e : new BackendErrorException(e));
      }
    })();
  });
}

export async function cancelTransfer(transferId: string): Promise<void> {
  return invoke("cancel_transfer", { transferId });
}

export async function previewTransferOwnership(request: TransferRequest): Promise<TransferOwnershipPreview> {
  return invoke("preview_transfer_ownership", { request });
}

export interface SortTablesByFkOptions {
  connectionId: string;
  database: string;
  schema: string;
  tables: string[];
  parentsFirst: boolean;
}

export async function sortTablesByFkDependency(options: SortTablesByFkOptions): Promise<string[]> {
  return invoke("sort_tables_by_fk_dependency", {
    connectionId: options.connectionId,
    database: options.database,
    schema: options.schema,
    tables: options.tables,
    parentsFirst: options.parentsFirst,
  });
}

// --- Table File Import ---
export type TableImportMode = "append" | "truncate";
export type TableImportConflictPolicy = "error" | "skip" | "updateExisting";
export type TableImportStatus = "running" | "done" | "error" | "cancelled";
export type TableImportPhase = "preparing" | "detectingEncoding" | "reading" | "writing" | "finalizing" | "done";
export type TableImportSourceFormat = "csv" | "tsv" | "delimited" | "json" | "excel" | "sql" | "parquet";
export type TableImportJsonShape = "auto" | "objects" | "arrays";
export type TableImportTextEncoding = "auto" | "utf8" | "gbk" | "utf16Le" | "utf16Be";

export interface TableImportColumnMapping {
  sourceColumn: string;
  targetColumn: string;
  targetDataType?: string | null;
}

export interface TableImportParseOptions {
  delimiter?: string | null;
  decimalSeparator?: string | null;
  encoding?: TableImportTextEncoding | null;
  hasHeader?: boolean | null;
  titleRow?: number | null;
  dataStartRow?: number | null;
  lastDataRow?: number | null;
  trimValues?: boolean | null;
  emptyStringAsNull?: boolean | null;
  /** 分隔文本里代表 NULL 的字面量。缺省表示用后端默认值 `\N`；空串表示关闭字面量，退回「空字段即 NULL」。 */
  nullLiteral?: string | null;
  sheetName?: string | null;
  sheetIndex?: number | null;
  jsonShape?: TableImportJsonShape | null;
  sqlDialect?: DatabaseType | null;
}

export interface TableImportPreviewRequest {
  filePath: string;
  connectionId?: string | null;
  database?: string | null;
  sourceRef?: string | null;
  sourceFormat?: TableImportSourceFormat | null;
  parseOptions?: TableImportParseOptions | null;
  previewLimit?: number | null;
}

export interface TableImportPreview {
  fileName: string;
  filePath: string;
  sourceRef?: string | null;
  fileType: string;
  sizeBytes: number;
  columns: string[];
  rows: unknown[][];
  totalRows: number;
  totalRowsExact?: boolean;
  sourceFingerprint: string;
  effectiveEncoding?: TableImportTextEncoding | null;
  sheets?: string[];
}

export interface TableImportPreparedSource {
  fingerprint: string;
  columns: string[];
  rows: unknown[][];
  totalRows: number;
  totalRowsExact?: boolean;
  effectiveEncoding?: TableImportTextEncoding | null;
}

export interface TableImportRequest {
  importId: string;
  connectionId: string;
  database: string;
  schema: string;
  table: string;
  filePath: string;
  sourceRef?: string | null;
  sourceFormat?: TableImportSourceFormat | null;
  parseOptions?: TableImportParseOptions | null;
  mappings: TableImportColumnMapping[];
  mode: TableImportMode;
  createTable?: boolean;
  batchSize: number;
  dateTimeFormat?: string;
  preparedSource?: TableImportPreparedSource | null;
  retainSource?: boolean;
  conflictPolicy?: TableImportConflictPolicy;
  skipDuplicateRows?: boolean;
}

export interface TableImportSummary {
  importId: string;
  rowsImported: number;
  totalRows: number;
  elapsedMs: number;
}

export interface TableImportProgress {
  importId: string;
  status: TableImportStatus;
  phase?: TableImportPhase;
  rowsImported: number;
  totalRows: number;
  totalRowsExact?: boolean;
  bytesRead?: number;
  totalBytes?: number;
  elapsedMs: number;
  error?: string | null;
}

export async function previewTableImportFile(filePathOrRequest: string | File | TableImportPreviewRequest, options: Partial<TableImportPreviewRequest> = {}): Promise<TableImportPreview> {
  if (typeof filePathOrRequest !== "string" && !("filePath" in filePathOrRequest)) {
    throw new Error("previewTableImportFile in desktop mode requires a file path, not a File object");
  }
  const request: TableImportPreviewRequest = typeof filePathOrRequest === "string" ? { ...options, filePath: filePathOrRequest } : filePathOrRequest;
  return invoke("preview_table_import_file", { request });
}

export async function importTableFile(request: TableImportRequest, onProgress: (progress: TableImportProgress) => void): Promise<TableImportSummary> {
  const unlisten: UnlistenFn = await listen<TableImportProgress>("table-import-progress", (event) => {
    if (event.payload.importId === request.importId) {
      onProgress(event.payload);
      if (event.payload.status === "done" || event.payload.status === "error" || event.payload.status === "cancelled") {
        unlisten();
      }
    }
  });
  try {
    const summary = await invoke<TableImportSummary>("import_table_file", {
      request,
    });
    unlisten();
    return summary;
  } catch (e) {
    unlisten();
    throw e instanceof BackendErrorException ? e : new BackendErrorException(e);
  }
}

export async function cancelTableImport(importId: string): Promise<boolean> {
  return invoke("cancel_table_import", { importId });
}

export async function releaseTableImportSource(_sourceRef: string): Promise<boolean> {
  return false;
}

export function inspectMongodbDatabaseDump(connectionId: string, database: string): Promise<MongoDumpCatalog> {
  return invoke("inspect_mongodb_database_dump", { connectionId, database });
}
export function prepareMongodbRestoreSource(source: MongoDumpSourceInput, format: MongoDumpFormat, gzip: boolean, _options?: MongoSourceReadOptions): Promise<MongoRestoreSourcePreview> {
  if (typeof source !== "string") throw new Error("Desktop restores require a file or directory path");
  return invoke("prepare_mongodb_restore_source", { request: { path: source, format, gzip } });
}
export function releaseMongodbRestoreSource(sourceRef: string): Promise<boolean> {
  return invoke("release_mongodb_restore_source", { sourceRef });
}
async function runMongodbDatabaseTask(command: string, request: MongoDatabaseDumpRequest | MongoDatabaseRestoreRequest, onProgress: (progress: MongoDatabaseDumpProgress) => void): Promise<MongoDatabaseDumpProgress> {
  const unlisten = await listen<MongoDatabaseDumpProgress>("mongo-database-dump-progress", (event) => {
    if (event.payload.taskId === request.taskId) onProgress(event.payload);
  });
  try {
    return await invoke(command, { request });
  } finally {
    unlisten();
  }
}
export function dumpMongodbDatabase(request: MongoDatabaseDumpRequest, onProgress: (progress: MongoDatabaseDumpProgress) => void) {
  return runMongodbDatabaseTask("dump_mongodb_database", request, onProgress);
}
export function restoreMongodbDatabase(request: MongoDatabaseRestoreRequest, onProgress: (progress: MongoDatabaseDumpProgress) => void, _upload?: MongoRestoreUpload) {
  return runMongodbDatabaseTask("restore_mongodb_database", request, onProgress);
}
export function cancelMongodbDatabaseDump(taskId: string): Promise<boolean> {
  return invoke("cancel_mongodb_database_dump", { taskId });
}

export type MongoImportFormat = "csv" | "json" | "ndjson" | "bson";
export type MongoImportTypeMode = "string" | "auto" | "extendedJson";
export type MongoImportInferredType = "boolean" | "integer" | "decimal" | "date" | "objectId" | "object" | "array" | "mixed" | "string";
export type MongoImportStatus = "running" | "done" | "error" | "cancelled";
export type MongoImportPhase = "preparing" | "parsing" | "writing" | "done";
export type MongoExportFormat = "csv" | "ndjson" | "bson";
export type MongoExportStatus = "running" | "done" | "error" | "cancelled";

export interface MongoImportIssue {
  code: string;
  message: string;
  row?: number | null;
  column?: string | null;
  value?: string | null;
  batch?: number | null;
  retryable?: boolean;
}

export interface MongoImportParseOptions {
  encoding?: TableImportTextEncoding | null;
  delimiter?: string | null;
  hasHeader?: boolean | null;
  trim?: boolean | null;
  emptyAsNull?: boolean | null;
  typeMode?: MongoImportTypeMode | null;
  recognizeObjectIdHex?: boolean | null;
  skipErrorRows?: boolean | null;
  columnTypes?: Partial<Record<string, MongoImportInferredType>> | null;
}

export interface MongoImportPreviewRequest {
  filePath: string;
  sourceRef?: string | null;
  format: MongoImportFormat;
  parseOptions?: MongoImportParseOptions;
  previewLimit?: number | null;
}

export interface MongoImportColumn {
  name: string;
  inferredType: MongoImportInferredType;
  sampleValues?: unknown[];
}

export interface MongoImportPreview {
  sourceRef?: string | null;
  format: MongoImportFormat;
  detectedEncoding?: TableImportTextEncoding | null;
  fileName: string;
  filePath: string;
  sizeBytes: number;
  columns: MongoImportColumn[];
  rows: Record<string, unknown>[];
  rowNumbers?: number[];
  warnings: MongoImportIssue[];
  errors: MongoImportIssue[];
  estimatedRows?: number | null;
  estimatedRowsExact: boolean;
}

export interface MongoImportRequest {
  importId: string;
  connectionId: string;
  database: string;
  collection: string;
  filePath: string;
  sourceRef?: string | null;
  format: MongoImportFormat;
  parseOptions?: MongoImportParseOptions;
  batchSize: number;
  executionId?: string | null;
}

export interface MongoImportProgress {
  importId: string;
  phase: MongoImportPhase;
  status: MongoImportStatus;
  rowsRead: number;
  rowsInserted: number;
  rowsFailed: number;
  batchesCommitted: number;
  totalRows?: number | null;
  errorRows?: MongoImportIssue[];
  errorMessage?: string | null;
  elapsedMs: number;
}

export interface MongoImportSummary {
  importId: string;
  rowsInserted: number;
  rowsFailed: number;
  batchesCommitted: number;
  elapsedMs: number;
}

export interface MongoExportRequest {
  exportId: string;
  connectionId: string;
  database: string;
  collection: string;
  filter?: string | null;
  sort?: string | null;
  projection?: string | null;
  collation?: string | null;
  format: MongoExportFormat;
  includeHeader?: boolean;
  gzip?: boolean;
  filePath: string;
  executionId?: string | null;
}

export interface MongoExportProgress {
  exportId: string;
  status: MongoExportStatus;
  documentsRead: number;
  bytesWritten: number;
  totalDocuments?: number | null;
  errorMessage?: string | null;
  elapsedMs: number;
}

export interface MongoExportSummary {
  exportId: string;
  documentsExported: number;
  filePath: string;
  elapsedMs: number;
}

export async function previewMongodbImportFile(filePathOrRequest: string | File | MongoImportPreviewRequest, options: Partial<MongoImportPreviewRequest> = {}): Promise<MongoImportPreview> {
  if (typeof filePathOrRequest !== "string" && !("filePath" in filePathOrRequest)) {
    throw new Error("previewMongodbImportFile in desktop mode requires a file path, not a File object");
  }
  const request: MongoImportPreviewRequest = typeof filePathOrRequest === "string" ? { format: options.format ?? "csv", ...options, filePath: filePathOrRequest } : filePathOrRequest;
  return invoke("preview_mongodb_import_file", { request });
}

export async function importMongodbFile(request: MongoImportRequest, onProgress: (progress: MongoImportProgress) => void): Promise<MongoImportSummary> {
  const unlisten: UnlistenFn = await listen<MongoImportProgress>("mongo-import-progress", (event) => {
    if (event.payload.importId === request.importId) {
      onProgress(event.payload);
      if (event.payload.status === "done" || event.payload.status === "error" || event.payload.status === "cancelled") {
        unlisten();
      }
    }
  });
  try {
    const summary = await invoke<MongoImportSummary>("import_mongodb_file", { request });
    unlisten();
    return summary;
  } catch (e) {
    unlisten();
    throw e instanceof BackendErrorException ? e : new BackendErrorException(e);
  }
}

export async function cancelMongodbImport(importId: string): Promise<boolean> {
  return invoke("cancel_mongodb_import", { importId });
}

export async function releaseMongodbImportSource(_sourceRef: string): Promise<boolean> {
  return false;
}

export async function exportMongodbQuery(request: MongoExportRequest, onProgress: (progress: MongoExportProgress) => void): Promise<MongoExportSummary> {
  const unlisten: UnlistenFn = await listen<MongoExportProgress>("mongo-export-progress", (event) => {
    if (event.payload.exportId === request.exportId) {
      onProgress(event.payload);
      if (event.payload.status === "done" || event.payload.status === "error" || event.payload.status === "cancelled") {
        unlisten();
      }
    }
  });
  try {
    const summary = await invoke<MongoExportSummary>("export_mongodb_query", { request });
    unlisten();
    return summary;
  } catch (e) {
    unlisten();
    throw e instanceof BackendErrorException ? e : new BackendErrorException(e);
  }
}

export async function cancelMongodbExport(exportId: string): Promise<boolean> {
  return invoke("cancel_mongodb_export", { exportId });
}

// --- Database Export ---
export interface DatabaseExportRequest {
  exportId: string;
  connectionId: string;
  database: string;
  schema: string;
  filePath: string;
  selectedTables?: string[];
  excludedTables?: string[];
  includeStructure: boolean;
  includeData: boolean;
  includeObjects: boolean;
  includeCreateDatabase?: boolean;
  dropTableIfExists?: boolean;
  omitAutoIncrement?: boolean;
  preserveOriginalLanguage?: boolean;
  failOnError?: boolean;
  preventOverwrite?: boolean;
  outputCompression?: "none" | "gzip";
  insertDialect?: SqlInsertDialect;
  insertMode?: "batch" | "single";
  snapshotSessionId?: string;
  batchSize: number;
  splitMaxMb?: number;
}

export interface DatabaseBackupSnapshot {
  sessionId: string;
  schemas: string[];
}

export interface ExportProgress {
  exportId: string;
  currentObject: string;
  objectIndex: number;
  totalObjects: number;
  rowsExported: number;
  totalRows: number | null;
  status: "Running" | "Done" | "Error" | "Cancelled";
  error: string | null;
  /** True while listing schema / prefetching metadata before objects are written. */
  preparing?: boolean;
  /** Per-object failures written into the file as `-- ERROR` comments (lenient mode). */
  errorCount?: number;
  /** First lenient failure, for completion warnings without opening the file. */
  errorSummary?: string | null;
}

// --- Table Export ---
export type TableExportStatus = "Running" | "Writing" | "Done" | "Error" | "Cancelled";

export interface TableExportRequest {
  exportId: string;
  connectionId: string;
  database: string;
  schema?: string;
  identifierQuote?: string;
  tableName: string;
  filePath: string;
  format: "csv" | "xlsx" | "json" | "markdown" | "sql" | "txt";
  insertMode?: SqlInsertMode;
  insertDialect?: SqlInsertDialect;
  csvQuoteMode?: CsvQuoteMode;
  /** CSV 里 NULL 写成什么。缺省表示用后端默认值 `\N`；空串表示关闭字面量。 */
  nullLiteral?: string;
  columns?: string[];
  selectedColumns?: SqlExportColumnSelection[];
  columnTypes?: Array<string | null | undefined>;
  /** 与 `columns` 对齐的列 EXTRA 元数据（identity 等），用于 SQL INSERT 导出的 `SET IDENTITY_INSERT`。 */
  columnExtras?: Array<string | null | undefined>;
  columnComments?: Array<string | null> | null;
  primaryKeys?: string[];
  /** 导出 SQL 时是否排除主键列（对应数据提取设置里的“排除主键”）。 */
  excludePrimaryKeys?: boolean;
  whereInput?: string;
  orderBy?: string;
  skipCount?: boolean;
  batchSize?: number;
  rowLimit?: number | null;
  dateTimeFormat?: string;
  numericColumnRightAlign?: boolean;
  autoFilter?: boolean;
  splitMaxMb?: number;
  /**
   * SQL 导出时省略 INSERT 目标的库/模式限定（前端按「生成 SQL 时包含数据库名」设置 +
   * `dropsSchemaQualifier` 引擎规则解析；仅影响 INSERT 目标，读取 SQL 不变）。
   */
  omitDatabaseQualifier?: boolean;
}

export interface TableCsvExportOptions {
  filePath: string;
  connectionId: string;
  database: string;
  schema?: string;
  tableName: string;
  columns?: string[];
  pageSize?: number;
  timeoutSecs?: number;
  csvQuoteMode?: CsvQuoteMode;
  /** CSV 里 NULL 写成什么。缺省表示用后端默认值 `\N`；空串表示关闭字面量。 */
  nullLiteral?: string;
}

export interface TableExportProgress {
  exportId: string;
  tableName: string;
  rowsExported: number;
  totalRows: number | null;
  status: TableExportStatus;
  errorMessage?: string;
}

export interface QueryResultExportRequest {
  exportId: string;
  connectionId: string;
  database: string;
  schema?: string;
  catalog?: string;
  sql: string;
  queryBaseSql: string;
  setupSql?: string[];
  databaseType: DatabaseType;
  useAgentCursor: boolean;
  filePath: string;
  format: "csv" | "xlsx" | "json" | "txt" | "sql";
  insertMode?: SqlInsertMode;
  csvQuoteMode?: CsvQuoteMode;
  /** CSV 里 NULL 写成什么。缺省表示用后端默认值 `\N`；空串表示关闭字面量。 */
  nullLiteral?: string;
  includeSqlSheet?: boolean;
  pageSize: number;
  rowLimit?: number | null;
  totalRows?: number | null;
  timeoutSecs?: number;
  keysetOptimizationEnabled: boolean;
  clientSessionId?: string;
  executionId?: string;
  dateTimeFormat?: string;
  exportTableName?: string;
  exportColumnTypes?: Array<string | null | undefined>;
  selectedColumns?: SqlExportColumnSelection[];
  /**
   * 结果列对应的原表 EXTRA 元数据（identity 等）。后端据此为 SQL INSERT 导出
   * 补上 `SET IDENTITY_INSERT` 包裹，缺省表示未知。
   */
  exportColumnExtras?: Array<string | null | undefined>;
  numericColumnRightAlign?: boolean;
  columnComments?: Array<string | null> | null;
  autoFilter?: boolean;
  identifierQuote?: string;
  /** 导出 SQL 时是否排除主键列（对应数据提取设置里的“排除主键”）。 */
  excludePrimaryKeys?: boolean;
  /** 结果集对应的原表主键列名，由前端从表元数据带入。 */
  primaryKeys?: string[];
}

export async function startTableExport(request: TableExportRequest, onProgress: (progress: TableExportProgress) => void): Promise<TableExportProgress> {
  let unlisten: UnlistenFn | undefined;
  let settled = false;
  let resolveTerminal: (progress: TableExportProgress) => void = () => {};
  let rejectTerminal: (error: unknown) => void = () => {};

  const terminalProgress = new Promise<TableExportProgress>((resolve, reject) => {
    resolveTerminal = resolve;
    rejectTerminal = reject;
  });

  const finish = (callback: () => void) => {
    if (settled) return;
    settled = true;
    unlisten?.();
    callback();
  };

  try {
    unlisten = await listen<TableExportProgress>("table-export-progress", (event) => {
      if (event.payload.exportId !== request.exportId) return;
      onProgress(event.payload);
      if (event.payload.status === "Done" || event.payload.status === "Error" || event.payload.status === "Cancelled") {
        if (event.payload.status === "Error") {
          finish(() => rejectTerminal(new BackendErrorException(event.payload.errorMessage || "Export failed")));
        } else {
          finish(() => resolveTerminal(event.payload));
        }
      }
    });
    await invoke("start_table_export", { request });
    return await terminalProgress;
  } catch (error) {
    if (!settled) {
      settled = true;
      unlisten?.();
    }
    throw error instanceof BackendErrorException ? error : new BackendErrorException(error);
  }
}

export async function cancelTableExport(exportId: string): Promise<void> {
  return invoke("cancel_table_export", { exportId });
}

export async function startQueryResultExport(request: QueryResultExportRequest, onProgress: (progress: TableExportProgress) => void): Promise<TableExportProgress> {
  let unlisten: UnlistenFn | undefined;
  let settled = false;
  let resolveTerminal: (progress: TableExportProgress) => void = () => {};
  let rejectTerminal: (error: unknown) => void = () => {};

  const terminalProgress = new Promise<TableExportProgress>((resolve, reject) => {
    resolveTerminal = resolve;
    rejectTerminal = reject;
  });

  const finish = (callback: () => void) => {
    if (settled) return;
    settled = true;
    unlisten?.();
    callback();
  };

  try {
    unlisten = await listen<TableExportProgress>("query-result-export-progress", (event) => {
      if (event.payload.exportId !== request.exportId) return;
      onProgress(event.payload);
      if (event.payload.status === "Done" || event.payload.status === "Error" || event.payload.status === "Cancelled") {
        if (event.payload.status === "Error") {
          finish(() => rejectTerminal(new BackendErrorException(event.payload.errorMessage || "Export failed")));
        } else {
          finish(() => resolveTerminal(event.payload));
        }
      }
    });
    await invoke("start_query_result_export", { request });
    return await terminalProgress;
  } catch (error) {
    if (!settled) {
      settled = true;
      unlisten?.();
    }
    throw error instanceof BackendErrorException ? error : new BackendErrorException(error);
  }
}

export async function cancelQueryResultExport(exportId: string, executionId?: string): Promise<void> {
  return invoke("cancel_query_result_export", {
    exportId,
    executionId: executionId || null,
  });
}

export async function createQueryResultTempFile(extension = "xlsx"): Promise<string> {
  return invoke("create_query_result_temp_file", { extension });
}

export async function beginDatabaseBackupSnapshot(connectionId: string, database: string, exportId?: string): Promise<DatabaseBackupSnapshot> {
  return invoke("begin_database_backup_snapshot", { connectionId, database, exportId: exportId || null });
}

export async function exportDatabaseSql(request: DatabaseExportRequest, onProgress: (progress: ExportProgress) => void): Promise<void> {
  const unlisten: UnlistenFn = await listen<ExportProgress>("database-export-progress", (event) => {
    if (event.payload.exportId === request.exportId) {
      onProgress(event.payload);
      if (event.payload.status === "Done" || event.payload.status === "Error" || event.payload.status === "Cancelled") {
        unlisten();
      }
    }
  });
  try {
    await invoke("export_database_sql", { request });
  } catch (e) {
    unlisten();
    throw e;
  }
}

export async function cancelDatabaseExport(exportId: string): Promise<void> {
  await invoke("cancel_database_export", { exportId });
}

export async function clearDatabaseExportCancellation(exportId: string): Promise<void> {
  await invoke("clear_database_export_cancellation", { exportId });
}

export async function databaseExportDestinationNeedsConfirmation(directory: string): Promise<boolean> {
  return invoke("database_export_destination_needs_confirmation", { directory });
}

export async function recordDatabaseExportDestination(directory: string): Promise<void> {
  await invoke("record_database_export_destination", { directory });
}

export async function exportQueryResultCsv(filePath: string, columns: string[], rows: readonly (readonly XlsxCellValue[])[], csvQuoteMode: CsvQuoteMode = "all", nullLiteral?: string): Promise<void> {
  return invoke("export_query_result_csv", {
    request: {
      filePath,
      columns,
      rows,
      csvQuoteMode,
      ...(nullLiteral === undefined ? {} : { nullLiteral }),
    },
  });
}

export async function exportTableDataCsv(options: TableCsvExportOptions): Promise<number> {
  return invoke("export_table_data_csv", { request: options });
}

export async function exportQueryResultXlsx(
  filePath: string,
  sheetName: string | undefined,
  columns: string[],
  columnTypes: string[],
  columnComments: readonly (string | null)[] | undefined,
  rows: readonly (readonly XlsxCellValue[])[],
  numericColumnRightAlign?: boolean,
  autoFilter?: boolean,
  dateTimeFormat?: string,
): Promise<void> {
  return invoke("export_query_result_xlsx", {
    request: {
      filePath,
      sheetName,
      columns,
      columnTypes,
      columnComments,
      rows,
      numericColumnRightAlign,
      autoFilter,
      dateTimeFormat,
    },
  });
}

export async function exportQueryResultsXlsx(
  filePath: string,
  worksheets: readonly {
    sheetName?: string;
    columns: readonly string[];
    columnTypes?: readonly string[];
    columnComments?: readonly (string | null)[];
    rows: readonly (readonly XlsxCellValue[])[];
    numericColumnRightAlign?: boolean;
    autoFilter?: boolean;
  }[],
  autoFilter?: boolean,
  dateTimeFormat?: string,
): Promise<void> {
  return invoke("export_query_results_xlsx", {
    request: {
      filePath,
      worksheets,
      autoFilter,
      dateTimeFormat,
    },
  });
}

export async function exportQueryResultJson(filePath: string, columns: string[], rows: readonly (readonly XlsxCellValue[])[]): Promise<void> {
  return invoke("export_query_result_json", {
    request: {
      filePath,
      columns,
      rows,
    },
  });
}

export async function exportQueryResultMarkdown(filePath: string, columns: string[], rows: readonly (readonly XlsxCellValue[])[]): Promise<void> {
  return invoke("export_query_result_markdown", {
    request: {
      filePath,
      columns,
      rows,
    },
  });
}

export async function exportQueryResultHtml(filePath: string, title: string | undefined, columns: string[], rows: readonly (readonly XlsxCellValue[])[]): Promise<void> {
  return invoke("export_query_result_html", {
    request: {
      filePath,
      title,
      columns,
      rows,
    },
  });
}

export async function openQueryResultTempFile(path: string): Promise<void> {
  return invoke("open_query_result_temp_file", { path });
}
