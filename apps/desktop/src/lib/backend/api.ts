import { isTauriRuntime } from "@/lib/backend/tauriRuntime";
import type * as TauriModule from "@/lib/backend/tauri";
import { appendDebugLog } from "@/lib/backend/debugLog";
import type { AiConfigItem } from "@/types/ai";

// ---------------------------------------------------------------------------
// Lazy backend resolution (avoids top-level await)
// ---------------------------------------------------------------------------

type Backend = typeof TauriModule;

let _backend: Backend | null = null;

async function getBackend(): Promise<Backend> {
  if (_backend) return _backend;
  _backend = isTauriRuntime(globalThis) ? await import("@/lib/backend/tauri") : await import("@/lib/backend/http");
  return _backend;
}

// ---------------------------------------------------------------------------
// Helper: create a forwarding function that lazily resolves the backend
// ---------------------------------------------------------------------------

function forward<K extends keyof Backend>(name: K): Backend[K] {
  // SAFETY: the resolved module is one of exactly two implementations of the same
  // surface — `http.ts` imports its request/response types from `tauri.ts` on
  // purpose, so both transports are kept signature-compatible by construction.
  // TypeScript cannot express that cross-module equivalence, hence the cast.
  return (async (...args: unknown[]) => {
    const startedAt = performance.now();
    const operation = String(name);
    appendDebugLog("debug", "[DBX][api:start]", operation);
    const b = await getBackend();
    try {
      const result = await (b[name] as (...a: unknown[]) => unknown)(...args);
      appendDebugLog("debug", "[DBX][api:success]", {
        operation,
        elapsedMs: Math.round(performance.now() - startedAt),
      });
      return result;
    } catch (error) {
      appendDebugLog("error", "[DBX][api:error]", {
        operation,
        elapsedMs: Math.round(performance.now() - startedAt),
        error,
      });
      throw error;
    }
  }) as unknown as Backend[K];
}

// ---------------------------------------------------------------------------
// Re-export all functions via lazy forwarding
// ---------------------------------------------------------------------------

// Connection
export const testConnection = forward("testConnection");
export const testSshTunnel = forward("testSshTunnel");
export const testConnectionWithInfo = forward("testConnectionWithInfo");

export const connectDb = forward("connectDb");
export const connectionDatabaseInfo = forward("connectionDatabaseInfo");
export const saveConnectionDatabaseInfo = forward("saveConnectionDatabaseInfo");
export const unlockConnectionWrites = forward("unlockConnectionWrites");
export const lockConnectionWrites = forward("lockConnectionWrites");
export const connectionWriteUnlockState = forward("connectionWriteUnlockState");
export const connectionFinalProxyPort = forward("connectionFinalProxyPort");
export const disconnectDb = forward("disconnectDb");
export const sessionCredentialStatus = forward("sessionCredentialStatus");
export const forgetSessionCredential = forward("forgetSessionCredential");
export const replaceNacosSessionCredential = forward("replaceNacosSessionCredential");
export const checkConnectionHealth = forward("checkConnectionHealth");
export const connectionIsOpen = forward("connectionIsOpen");
export const subscribeConnectionLiveness = forward("subscribeConnectionLiveness");
export const prewarmConnection = forward("prewarmConnection");
export const connectionIdentifierQuote = forward("connectionIdentifierQuote");
export const closeDatabaseConnection = forward("closeDatabaseConnection");
export const refreshConnections = forward("refreshConnections");
export const saveConnections = forward("saveConnections");
export const loadConnections = forward("loadConnections");
export const loadTunnelProfiles = forward("loadTunnelProfiles");
export const saveTunnelProfiles = forward("saveTunnelProfiles");
export const testTunnelProfile = forward("testTunnelProfile");
export const resolveSshPrompt = forward("resolveSshPrompt");
export const readKeychainPassword = forward("readKeychainPassword");
export const readKeychainPasswords = forward("readKeychainPasswords");
export const decryptConfig = forward("decryptConfig");
export const listPlugins = forward("listPlugins");
export const listPluginTrustedKeys = forward("listPluginTrustedKeys");
export const savePluginTrustedKey = forward("savePluginTrustedKey");
export const removePluginTrustedKey = forward("removePluginTrustedKey");
export const listPluginRepositories = forward("listPluginRepositories");
export const savePluginRepository = forward("savePluginRepository");
export const removePluginRepository = forward("removePluginRepository");
export const fetchPluginMarketplaceCatalogs = forward("fetchPluginMarketplaceCatalogs");
export const installMarketplacePlugin = forward("installMarketplacePlugin");
export const installPluginPackage = forward("installPluginPackage");
export const installPluginPackageFromUrl = forward("installPluginPackageFromUrl");
export const rollbackPlugin = forward("rollbackPlugin");
export const uninstallPlugin = forward("uninstallPlugin");
export const activatePlugin = forward("activatePlugin");
export const listActivePlugins = forward("listActivePlugins");
export const stopPlugin = forward("stopPlugin");
export const invokePlugin = forward("invokePlugin");
export const invokePluginConnectionAction = forward("invokePluginConnectionAction");
export const notifyPlugin = forward("notifyPlugin");
export const sendPluginBinary = forward("sendPluginBinary");
export const listPluginFilesystemEntries = forward("listPluginFilesystemEntries");
export const readPluginFilesystemFile = forward("readPluginFilesystemFile");
export const writePluginFilesystemFile = forward("writePluginFilesystemFile");
export const createPluginFilesystemDirectory = forward("createPluginFilesystemDirectory");
export const deletePluginFilesystemEntry = forward("deletePluginFilesystemEntry");
export const renamePluginFilesystemEntry = forward("renamePluginFilesystemEntry");
export const readPluginAsset = forward("readPluginAsset");
export const readPluginUiEntry = forward("readPluginUiEntry");
export const readPluginUiAsset = forward("readPluginUiAsset");
export const subscribePluginEvents = forward("subscribePluginEvents");
export const listJdbcDrivers = forward("listJdbcDrivers");
export const listJdbcMavenBundles = forward("listJdbcMavenBundles");
export const listJdbcLocalBundles = forward("listJdbcLocalBundles");
export const importJdbcDrivers = forward("importJdbcDrivers");
export const installJdbcDriverFromMaven = forward("installJdbcDriverFromMaven");
export const installPrestoSqlJdbcDriver = forward("installPrestoSqlJdbcDriver");
export const deleteJdbcDriver = forward("deleteJdbcDriver");
export const deleteJdbcMavenBundle = forward("deleteJdbcMavenBundle");
export const deleteJdbcLocalBundle = forward("deleteJdbcLocalBundle");
export const jdbcPluginStatus = forward("jdbcPluginStatus");
export const installJdbcPlugin = forward("installJdbcPlugin");
export const installJdbcPluginLocal = forward("installJdbcPluginLocal");
export const uninstallJdbcPlugin = forward("uninstallJdbcPlugin");
export const listInstalledAgentsLocal = forward("listInstalledAgentsLocal");
export async function listInstalledAgents() {
  const backend = await getBackend();
  const { useSettingsStore } = await import("@/stores/settingsStore");
  return backend.listInstalledAgents(useSettingsStore().editorSettings.updateDownloadSource);
}
export const isAgentInstalled = forward("isAgentInstalled");
export const getDriverStoreUsage = forward("getDriverStoreUsage");
export const clearDriverDownloadCache = forward("clearDriverDownloadCache");
export const getDriverRuntimeSummary = forward("getDriverRuntimeSummary");
export const stopDriverRuntime = forward("stopDriverRuntime");
export const restartDriverRuntime = forward("restartDriverRuntime");
export async function installAgent(dbType: string, operationId?: string) {
  const backend = await getBackend();
  const { useSettingsStore } = await import("@/stores/settingsStore");
  return backend.installAgent(dbType, useSettingsStore().editorSettings.updateDownloadSource, operationId);
}
export async function upgradeAllAgents(operationId?: string) {
  const backend = await getBackend();
  const { useSettingsStore } = await import("@/stores/settingsStore");
  return backend.upgradeAllAgents(useSettingsStore().editorSettings.updateDownloadSource, operationId);
}
export const cancelAgentInstall = forward("cancelAgentInstall");
export const cancelAgentUpgradeAll = forward("cancelAgentUpgradeAll");
export const checkAgentUpdateBlockers = forward("checkAgentUpdateBlockers");
export const uninstallAgent = forward("uninstallAgent");
export const getAgentJavaRuntimeConfig = forward("getAgentJavaRuntimeConfig");
export const setAgentJavaRuntimeConfig = forward("setAgentJavaRuntimeConfig");
export const invalidateAgentRegistryCache = forward("invalidateAgentRegistryCache");
export async function importAgentsFromZip(fileOrPath: string | File, operationId?: string) {
  const backend = await getBackend();
  return backend.importAgentsFromZip(fileOrPath, operationId);
}
export const previewAgentOfflineExport = forward("previewAgentOfflineExport");
export const exportAgentsOffline = forward("exportAgentsOffline");
export const importAgentDriver = forward("importAgentDriver");
export const importAgentJar = importAgentDriver;
export async function reinstallJre(jreKey?: string, operationId?: string) {
  const backend = await getBackend();
  const { useSettingsStore } = await import("@/stores/settingsStore");
  return backend.reinstallJre(jreKey, useSettingsStore().editorSettings.updateDownloadSource, operationId);
}
export const uninstallJre = forward("uninstallJre");
export const listenAgentInstallProgress = forward("listenAgentInstallProgress");
export const loadSavedSqlLibrary = forward("loadSavedSqlLibrary");
export const loadSavedSqlFilesForSync = forward("loadSavedSqlFilesForSync");
export const loadSavedSqlFile = forward("loadSavedSqlFile");
export const saveSavedSqlFolder = forward("saveSavedSqlFolder");
export const deleteSavedSqlFolder = forward("deleteSavedSqlFolder");
export const saveSavedSqlFile = forward("saveSavedSqlFile");
export const deleteSavedSqlFile = forward("deleteSavedSqlFile");
export const savedSqlStorageDir = forward("savedSqlStorageDir");
export const openSavedSqlStorageDir = forward("openSavedSqlStorageDir");
export const revealPathInFileManager = forward("revealPathInFileManager");
export const deleteDatabaseBackupFiles = forward("deleteDatabaseBackupFiles");
export const databaseBackupCommand = forward("databaseBackupCommand");
export const databaseBackupBackground = forward("databaseBackupBackground");
export const downloadDatabaseBackupFile = forward("downloadDatabaseBackupFile");
export const prepareDatabaseBackupRestore = forward("prepareDatabaseBackupRestore");
export const isSqliteDatabaseFile = forward("isSqliteDatabaseFile");
export const backupSqliteDatabase = forward("backupSqliteDatabase");
export const restoreSqliteDatabase = forward("restoreSqliteDatabase");
export const syncSavedSqlDirectory = forward("syncSavedSqlDirectory");

// Schema
export const listDatabases = forward("listDatabases");
export const listDatabaseMetadata = forward("listDatabaseMetadata");
export const listDatabaseStorage = forward("listDatabaseStorage");
export const listXuguTablespaces = forward("listXuguTablespaces");
export const getSqlServerCompletionContext = forward("getSqlServerCompletionContext");
export const listDorisCatalogs = forward("listDorisCatalogs");
export const listDorisCatalogDatabases = forward("listDorisCatalogDatabases");
export const listSqlServerLinkedServers = forward("listSqlServerLinkedServers");
export const listSqlServerLinkedServerCatalogs = forward("listSqlServerLinkedServerCatalogs");
export const listSqlServerLinkedServerSchemas = forward("listSqlServerLinkedServerSchemas");
export const listSqlServerLinkedServerTables = forward("listSqlServerLinkedServerTables");
export const saveSchemaCache = forward("saveSchemaCache");
export const loadSchemaCache = forward("loadSchemaCache");
export const deleteSchemaCachePrefix = forward("deleteSchemaCachePrefix");
export const listSchemas = forward("listSchemas");
export const listSchemaInfos = forward("listSchemaInfos");
export const listTables = forward("listTables");
export const getTableComment = forward("getTableComment");
export const getMysqlTableAutoIncrement = forward("getMysqlTableAutoIncrement");
export const listObjects = forward("listObjects");
export const listObjectStatistics = forward("listObjectStatistics");
export const listCompletionObjects = forward("listCompletionObjects");
export const completionAssistantSearch = forward("completionAssistantSearch");
export const getObjectSource = forward("getObjectSource");
export const getEventInfo = forward("getEventInfo");
export const getCustomTypeDetails = forward("getCustomTypeDetails");
export const getColumns = forward("getColumns");
export const getPluginTableMetadata = forward("getPluginTableMetadata");
export const getAllColumns = forward("getAllColumns");
export const getSqlServerColumnMetadata = forward("getSqlServerColumnMetadata");
export const listDataTypes = forward("listDataTypes");
export const listIndexes = forward("listIndexes");
export const listReferenceKeyColumns = forward("listReferenceKeyColumns");
export const listReferenceKeys = forward("listReferenceKeys");
export const listForeignKeys = forward("listForeignKeys");
export const listForeignKeysForDatabase = forward("listForeignKeysForDatabase");
export const listTriggers = forward("listTriggers");
export const listConstraints = forward("listConstraints");
export const listPartitions = forward("listPartitions");
export const getTablePartitionStatus = forward("getTablePartitionStatus");
export const getTablePartitioning = forward("getTablePartitioning");
export const listInvalidIndexes = forward("listInvalidIndexes");
export const listSubpartitions = forward("listSubpartitions");
export const getTableDdl = forward("getTableDdl");
export const getTableDisplayDdl = forward("getTableDisplayDdl");
export const listFunctions = forward("listFunctions");
export const listSequences = forward("listSequences");
export const listRules = forward("listRules");
export const listOwners = forward("listOwners");
export const getTableOwner = forward("getTableOwner");
export const listExtensions = forward("listExtensions");
export const listAvailableExtensions = forward("listAvailableExtensions");
export const listEventTriggers = forward("listEventTriggers");
export const prepareSchemaDiff = forward("prepareSchemaDiff");
export const generateSchemaSyncSql = forward("generateSchemaSyncSql");
export const generateSchemaSyncPlan = forward("generateSchemaSyncPlan");
export const listDialectDataTypes = forward("listDialectDataTypes");

// Docs
export const collectDocsSnapshot = forward("collectDocsSnapshot");
export const collectDocsSnapshotForExport = forward("collectDocsSnapshotForExport");
export const loadDocsAnnotations = forward("loadDocsAnnotations");
export const applyDocsAnnotations = forward("applyDocsAnnotations");
export const saveDocsAnnotations = forward("saveDocsAnnotations");
export const exportDocsHtml = forward("exportDocsHtml");

// Query
export const executeQuery = forward("executeQuery");
export const executeConditionalUpdate = forward("executeConditionalUpdate");
export const executeMulti = forward("executeMulti");
export const executeMultiWithProgress = forward("executeMultiWithProgress");
export const executeBatch = forward("executeBatch");
export const executeScript = forward("executeScript");
export const executeScriptWith2pc = forward("executeScriptWith2pc");
export const executeInTransaction = forward("executeInTransaction");
export const beginManualTransaction = forward("beginManualTransaction");
export const executeInManualTransaction = forward("executeInManualTransaction");
export const commitManualTransaction = forward("commitManualTransaction");
export const rollbackManualTransaction = forward("rollbackManualTransaction");
export const cancelQuery = forward("cancelQuery");
export const cancelQueryAndWait = forward("cancelQueryAndWait");
export const cancelConditionalUpdate = forward("cancelConditionalUpdate");
export const closeQuerySession = forward("closeQuerySession");
export const closeClientConnectionSession = forward("closeClientConnectionSession");
export const analyzeSqlReferences = forward("analyzeSqlReferences");
export const findStatementAtCursor = forward("findStatementAtCursor");
export const prepareQueryPaginationExecutionPlan = forward("prepareQueryPaginationExecutionPlan");
export const buildSortedQuerySql = forward("buildSortedQuerySql");
export const buildExplainSql = forward("buildExplainSql");
export const getExplainInfo = forward("getExplainInfo");
export const getPluginPlanCapabilities = forward("getPluginPlanCapabilities");
export const getPluginEstimatedPlan = forward("getPluginEstimatedPlan");
export const queryPluginData = forward("queryPluginData");
export const getPluginDataGrants = forward("getPluginDataGrants");
export const setPluginDataGrant = forward("setPluginDataGrant");
export const buildCreateUserSql = forward("buildCreateUserSql");
export const buildDroppedFilePreviewSql = forward("buildDroppedFilePreviewSql");
export const buildTableSelectSql = forward("buildTableSelectSql");
export const buildDatabaseSearchSql = forward("buildDatabaseSearchSql");
export const buildSearchResultWhere = forward("buildSearchResultWhere");
export const buildRenameObjectSql = forward("buildRenameObjectSql");
export const buildRenameDatabaseSql = forward("buildRenameDatabaseSql");
export const buildRenameDatabasePreflightSql = forward("buildRenameDatabasePreflightSql");
export const buildCreateDatabaseSql = forward("buildCreateDatabaseSql");
export const buildDuckDbAttachDatabaseSql = forward("buildDuckDbAttachDatabaseSql");
export const buildSqliteAttachDatabaseSql = forward("buildSqliteAttachDatabaseSql");
export const buildDropObjectSql = forward("buildDropObjectSql");
export const buildDropTableSql = forward("buildDropTableSql");
export const buildDropTableChildObjectSql = forward("buildDropTableChildObjectSql");
export const buildEmptyTableSql = forward("buildEmptyTableSql");
export const buildTruncateTableSql = forward("buildTruncateTableSql");
export const buildVacuumTableSql = forward("buildVacuumTableSql");
export const buildMysqlAutoIncrementSql = forward("buildMysqlAutoIncrementSql");
export const buildDropDatabaseSql = forward("buildDropDatabaseSql");
export const buildCreateSchemaSql = forward("buildCreateSchemaSql");
export const buildUpdateDatabasePropertiesSql = forward("buildUpdateDatabasePropertiesSql");
export const buildDropSchemaSql = forward("buildDropSchemaSql");
export const buildDuplicateTableStructureSql = forward("buildDuplicateTableStructureSql");
export const buildCopyTableDataSql = forward("buildCopyTableDataSql");
export const buildExecutableObjectSourceStatements = forward("buildExecutableObjectSourceStatements");
export const buildExecutableObjectSourceSql = forward("buildExecutableObjectSourceSql");
export const buildEditableObjectSource = forward("buildEditableObjectSource");
export const buildRoutineRenameObjectSourceStatements = forward("buildRoutineRenameObjectSourceStatements");
export const buildViewDdlSql = forward("buildViewDdlSql");
export const buildTableStructureChangeSql = forward("buildTableStructureChangeSql");
export const buildTableOwnerChangeSql = forward("buildTableOwnerChangeSql");
export const buildTablePartitionOperationSql = forward("buildTablePartitionOperationSql");
export const buildCreatePartitionedTableSql = forward("buildCreatePartitionedTableSql");
export const previewSqliteTableStructureChange = forward("previewSqliteTableStructureChange");
export const applySqliteTableStructureChange = forward("applySqliteTableStructureChange");
export const buildCreateTableSql = forward("buildCreateTableSql");
export const buildSingleColumnAlterSql = forward("buildSingleColumnAlterSql");
export const analyzeEditableQueryEditability = forward("analyzeEditableQueryEditability");
export const prepareDataGridSave = forward("prepareDataGridSave");
export const extractDataGridSelection = forward("extractDataGridSelection");
export const buildDataGridCopyUpdateStatements = forward("buildDataGridCopyUpdateStatements");
export const buildDataGridCopyInsertStatement = forward("buildDataGridCopyInsertStatement");
export const buildDmlChangePreviewSql = forward("buildDmlChangePreviewSql");
export const buildDataGridContextFilterCondition = forward("buildDataGridContextFilterCondition");
export const buildDataGridColumnValueFilterCondition = forward("buildDataGridColumnValueFilterCondition");
export const buildDataGridColumnValuesFilterCondition = forward("buildDataGridColumnValuesFilterCondition");
export const buildDataGridColumnDistinctValuesSql = forward("buildDataGridColumnDistinctValuesSql");
export const buildDataGridCountSql = forward("buildDataGridCountSql");
export const buildDataGridConditionalUpdateSql = forward("buildDataGridConditionalUpdateSql");
export const buildHiveTablePropertiesSql = forward("buildHiveTablePropertiesSql");
export const buildExportInsertStatements = forward("buildExportInsertStatements");
export const buildExportSqlInsert = forward("buildExportSqlInsert");
export const buildDatabaseSqlExport = forward("buildDatabaseSqlExport");
export const prepareDataCompare = forward("prepareDataCompare");
export const prepareDataCompareFromTables = forward("prepareDataCompareFromTables");
export const prepareDataCompareMissingTarget = forward("prepareDataCompareMissingTarget");
export const buildDataCompareSyncPlan = forward("buildDataCompareSyncPlan");

// AI
export const aiComplete = forward("aiComplete");
export const aiStream = forward("aiStream");
export const aiAgentStream = forward("aiAgentStream");
export const aiCancelStream = forward("aiCancelStream");
export const resolveAiToolApproval = forward("resolveAiToolApproval");
export const getAiPluginToolPlugins = forward("getAiPluginToolPlugins");
export const setAiPluginToolPluginEnabled = forward("setAiPluginToolPluginEnabled");
export const previewPluginAiTools = forward("previewPluginAiTools");
export const aiTestConnection = forward("aiTestConnection");
export const aiListModels = forward("aiListModels");
export const aiResolveModelEffort = forward("aiResolveModelEffort");
export const saveAiChatSelection = forward("saveAiChatSelection");
export const loadAiChatSelection = forward("loadAiChatSelection");
export const saveAiConfig = forward("saveAiConfig");
export const loadAiConfig = forward("loadAiConfig");
export const saveAiConfigs = forward("saveAiConfigs");
export const loadAiConfigs = forward("loadAiConfigs");
export const setDefaultAiConfig = forward("setDefaultAiConfig");
export const saveAiConfigItem = forward("saveAiConfigItem");
export const deleteAiConfig = forward("deleteAiConfig");
export const saveAiProviderConfig = forward("saveAiProviderConfig");
export const loadAiProviderConfigs = forward("loadAiProviderConfigs");
export const loadDesktopSettings = forward("loadDesktopSettings");
export const saveDesktopSettings = forward("saveDesktopSettings");
export const loadMcpGlobalPolicy = forward("loadMcpGlobalPolicy");
export const saveMcpGlobalPolicy = forward("saveMcpGlobalPolicy");
export const loadMaxAgentTurns = forward("loadMaxAgentTurns");
export const loadSqlFileUploadMaxBytes = forward("loadSqlFileUploadMaxBytes");
export const saveSqlFileUploadMaxMb = forward("saveSqlFileUploadMaxMb");
export const saveMaxAgentTurns = forward("saveMaxAgentTurns");
export const loadHistoryRetentionLimit = forward("loadHistoryRetentionLimit");
export const saveHistoryRetentionLimit = forward("saveHistoryRetentionLimit");
export const loadMcpHistoryRetentionLimit = forward("loadMcpHistoryRetentionLimit");
export const saveMcpHistoryRetentionLimit = forward("saveMcpHistoryRetentionLimit");
export const loadMaxRetries = forward("loadMaxRetries");
export const saveMaxRetries = forward("saveMaxRetries");
export const completeAppClose = forward("completeAppClose");
export const requestAppClose = forward("requestAppClose");
export const setDriverStoreDir = forward("setDriverStoreDir");
export const setPluginStoreDir = forward("setPluginStoreDir");
export const setAgentStoreDir = forward("setAgentStoreDir");
export const getDriverStorePath = forward("getDriverStorePath");
export const loadPinnedTreeNodeIds = forward("loadPinnedTreeNodeIds");
export const savePinnedTreeNodeIds = forward("savePinnedTreeNodeIds");
export const loadEditorSettings = forward("loadEditorSettings");
export const saveEditorSettings = forward("saveEditorSettings");
export const saveBackgroundImage = forward("saveBackgroundImage");
export const clearBackgroundImage = forward("clearBackgroundImage");
export const readBackgroundImage = forward("readBackgroundImage");
export const checkBackgroundImage = forward("checkBackgroundImage");
export const loadOpenTabsState = forward("loadOpenTabsState");
export const saveOpenTabsState = forward("saveOpenTabsState");
export const saveDetachedTabHandoff = forward("saveDetachedTabHandoff");
export const loadDetachedTabHandoff = forward("loadDetachedTabHandoff");
export const listDetachedTabHandoffs = forward("listDetachedTabHandoffs");
export const deleteDetachedTabHandoff = forward("deleteDetachedTabHandoff");
export const approveDetachedWindowClose = forward("approveDetachedWindowClose");
export const loadSavedSqlEditorPositions = forward("loadSavedSqlEditorPositions");
export const saveSavedSqlEditorPositions = forward("saveSavedSqlEditorPositions");
export const loadTransferTaskLibrary = forward("loadTransferTaskLibrary");
export const saveTransferTaskLibrary = forward("saveTransferTaskLibrary");
export const webdavSyncTest = forward("webdavSyncTest");
export const migrationStatus = forward("migrationStatus");
export const migrationStart = forward("migrationStart");
export const migrationRetry = forward("migrationRetry");
export const migrationCleanupBackups = forward("migrationCleanupBackups");
export const webdavPasswordStatus = forward("webdavPasswordStatus");
export const saveWebdavSavedPassword = forward("saveWebdavSavedPassword");
export const forgetWebdavSavedPassword = forward("forgetWebdavSavedPassword");
export const webdavSyncSecretsStatus = forward("webdavSyncSecretsStatus");
export const saveWebdavSyncSecretsPreference = forward("saveWebdavSyncSecretsPreference");
export const forgetWebdavSyncSecretsPassphrase = forward("forgetWebdavSyncSecretsPassphrase");
export const cloudSyncLocalCatalog = forward("cloudSyncLocalCatalog");
export const localBackupExport = forward("localBackupExport");
export const localBackupInspect = forward("localBackupInspect");
export const localBackupImport = forward("localBackupImport");
export const webdavSyncInspect = forward("webdavSyncInspect");
export const webdavSyncUpload = forward("webdavSyncUpload");
export const webdavSyncDownload = forward("webdavSyncDownload");
export const snippetSyncTest = forward("snippetSyncTest");
export const snippetTokenStatus = forward("snippetTokenStatus");
export const saveSnippetSavedToken = forward("saveSnippetSavedToken");
export const forgetSnippetSavedToken = forward("forgetSnippetSavedToken");
export const snippetSyncSettings = forward("snippetSyncSettings");
export const snippetSyncInspect = forward("snippetSyncInspect");
export const saveSnippetSyncId = forward("saveSnippetSyncId");
export const retrySnippetLegacyCleanup = forward("retrySnippetLegacyCleanup");
export const snippetSyncUpload = forward("snippetSyncUpload");
export const snippetSyncDownload = forward("snippetSyncDownload");
export const saveAiConversation = forward("saveAiConversation");
export const loadAiConversations = forward("loadAiConversations");
export const deleteAiConversation = forward("deleteAiConversation");
export const saveAiRun = forward("saveAiRun");
export const saveAiRunState = forward("saveAiRunState");
export const loadAiRuns = forward("loadAiRuns");

// Prompt Templates
export const loadPromptTemplates = forward("loadPromptTemplates");
export const savePromptTemplate = forward("savePromptTemplate");
export const deletePromptTemplate = forward("deletePromptTemplate");
export const getAiGlobalCustomInstructions = forward("getAiGlobalCustomInstructions");
export const setAiGlobalCustomInstructions = forward("setAiGlobalCustomInstructions");

// User Skills (read-only Codex-compatible SKILL.md library)
export const listUserSkills = forward("listUserSkills");
export const readUserSkills = forward("readUserSkills");

// System
export const listSystemFonts = forward("listSystemFonts");
export const listSshConfigHosts = forward("listSshConfigHosts");
export const listLocalSshKeys = forward("listLocalSshKeys");

// SQL File Execution
export const previewSqlFile = forward("previewSqlFile");
export const releaseSqlFilePreview = forward("releaseSqlFilePreview");
export const executeSqlFile = forward("executeSqlFile");
export const executeSqlFiles = forward("executeSqlFiles");
export const cancelSqlFileExecution = forward("cancelSqlFileExecution");
export const inspectSqlFileTables = forward("inspectSqlFileTables");
export const listenSqlFileProgress = forward("listenSqlFileProgress");
export const pendingOpenSqlFiles = forward("pendingOpenSqlFiles");
export const pendingOpenDbFiles = forward("pendingOpenDbFiles");
export const pendingOpenConnectionLinks = forward("pendingOpenConnectionLinks");
export const pendingOpenAiConfigLinks = forward("pendingOpenAiConfigLinks");
export const pendingOpenPluginInstallLinks = forward("pendingOpenPluginInstallLinks");
export const readExternalSqlFile = forward("readExternalSqlFile");
export const readExternalSqlFileSnapshot = forward("readExternalSqlFileSnapshot");
export const inspectExternalSqlFile = forward("inspectExternalSqlFile");
export const writeExternalSqlFile = forward("writeExternalSqlFile");
export const saveExternalSqlFile = forward("saveExternalSqlFile");
export const listSqlFilesInFolder = forward("listSqlFilesInFolder");
export const globalSearch = forward("globalSearch");
export const loadGlobalSearchSettings = forward("loadGlobalSearchSettings");
export const saveGlobalSearchSettings = forward("saveGlobalSearchSettings");
export const createSqlFileInFolder = forward("createSqlFileInFolder");
export const renameSqlFileInFolder = forward("renameSqlFileInFolder");
export const deleteSqlFileInFolder = forward("deleteSqlFileInFolder");

// Nacos

// Data Transfer
export const startTransfer = forward("startTransfer");
export const cancelTransfer = forward("cancelTransfer");
export const previewTransferOwnership = forward("previewTransferOwnership");
export const sortTablesByFkDependency = forward("sortTablesByFkDependency");

// Table File Import
export const previewTableImportFile = forward("previewTableImportFile");
export const importTableFile = forward("importTableFile");
export const cancelTableImport = forward("cancelTableImport");
export const releaseTableImportSource = forward("releaseTableImportSource");
export const previewMongodbImportFile = forward("previewMongodbImportFile");
export const importMongodbFile = forward("importMongodbFile");
export const cancelMongodbImport = forward("cancelMongodbImport");
export const releaseMongodbImportSource = forward("releaseMongodbImportSource");
export const exportMongodbQuery = forward("exportMongodbQuery");
export const cancelMongodbExport = forward("cancelMongodbExport");
export const inspectMongodbDatabaseDump = forward("inspectMongodbDatabaseDump");
export const prepareMongodbRestoreSource = forward("prepareMongodbRestoreSource");
export const releaseMongodbRestoreSource = forward("releaseMongodbRestoreSource");
export const dumpMongodbDatabase = forward("dumpMongodbDatabase");
export const restoreMongodbDatabase = forward("restoreMongodbDatabase");
export const cancelMongodbDatabaseDump = forward("cancelMongodbDatabaseDump");
export type { MongoDumpFormat, MongoDumpSourceInput, MongoDumpCatalog, MongoDumpCollection, MongoRestoreSourcePreview, MongoDatabaseDumpRequest, MongoDatabaseRestoreRequest, MongoDatabaseDumpProgress } from "./mongodbDumpTypes";

// Database Export
export const beginDatabaseBackupSnapshot = forward("beginDatabaseBackupSnapshot");
export const exportDatabaseSql = forward("exportDatabaseSql");
export const cancelDatabaseExport = forward("cancelDatabaseExport");
export const clearDatabaseExportCancellation = forward("clearDatabaseExportCancellation");
export const databaseExportDestinationNeedsConfirmation = forward("databaseExportDestinationNeedsConfirmation");
export const recordDatabaseExportDestination = forward("recordDatabaseExportDestination");
export const exportQueryResultCsv = forward("exportQueryResultCsv");
export const exportTableDataCsv = forward("exportTableDataCsv");
export const exportQueryResultXlsx = forward("exportQueryResultXlsx");
export const exportQueryResultsXlsx = forward("exportQueryResultsXlsx");
export const exportQueryResultJson = forward("exportQueryResultJson");
export const exportQueryResultMarkdown = forward("exportQueryResultMarkdown");
export const exportQueryResultHtml = forward("exportQueryResultHtml");
export const startTableExport = forward("startTableExport");
export const cancelTableExport = forward("cancelTableExport");
export const startQueryResultExport = forward("startQueryResultExport");
export const cancelQueryResultExport = forward("cancelQueryResultExport");
export const openQueryResultTempFile = forward("openQueryResultTempFile");
export const createQueryResultTempFile = forward("createQueryResultTempFile");

// Redis

// etcd

// ZooKeeper

// Consul KV

// HBase

// Message Queue

// MongoDB
export const documentListDatabases = forward("documentListDatabases");

export const documentListCollections = forward("documentListCollections");

export const documentListGridFsBuckets = forward("documentListGridFsBuckets");
export const documentCreateGridFsBucket = forward("documentCreateGridFsBucket");
export const documentDeleteGridFsBucket = forward("documentDeleteGridFsBucket");
export const documentListGridFsFiles = forward("documentListGridFsFiles");
export const documentDownloadGridFsFile = forward("documentDownloadGridFsFile");
export const documentUploadGridFsFile = forward("documentUploadGridFsFile");
export const documentDeleteGridFsFile = forward("documentDeleteGridFsFile");
export const vectorGetCollectionDetail = forward("vectorGetCollectionDetail");
export const vectorDropDatabase = forward("vectorDropDatabase");
export const vectorDropCollection = forward("vectorDropCollection");
export const vectorRenameCollection = forward("vectorRenameCollection");

export const documentFindDocuments = forward("documentFindDocuments");
export const documentCountDocuments = forward("documentCountDocuments");
export const dynamodbDescribeTable = forward("dynamodbDescribeTable");

export const documentInsertDocument = forward("documentInsertDocument");

export const documentUpdateDocument = forward("documentUpdateDocument");

export const documentDeleteDocument = forward("documentDeleteDocument");
export const documentSaveMeilisearchBatch = forward("documentSaveMeilisearchBatch");

// Elasticsearch

export const vectorListCollections = forward("vectorListCollections");

// History
export const saveHistory = forward("saveHistory");
export const loadHistory = forward("loadHistory");
export const searchHistory = forward("searchHistory");
export const loadHistoryConnectionOptions = forward("loadHistoryConnectionOptions");
export const loadRedisHistory = forward("loadRedisHistory");
export const clearHistory = forward("clearHistory");
export const clearHistoryBySource = forward("clearHistoryBySource");
export const cleanupMcpHistoryRetention = forward("cleanupMcpHistoryRetention");
export const clearRedisHistory = forward("clearRedisHistory");
export const deleteHistoryEntry = forward("deleteHistoryEntry");

// Updates
export const checkMcpServerStatus = forward("checkMcpServerStatus");

export const loadMcpHttpServerSettings = forward("loadMcpHttpServerSettings");
export const saveMcpHttpServerSettings = forward("saveMcpHttpServerSettings");
export const mcpHttpServerStatus = forward("mcpHttpServerStatus");
export const rotateMcpHttpServerToken = forward("rotateMcpHttpServerToken");
export const loadWebMcpHttpStatus = forward("loadWebMcpHttpStatus");
export const saveWebMcpHttpSettings = forward("saveWebMcpHttpSettings");
export const rotateWebMcpToken = forward("rotateWebMcpToken");
export const checkForUpdates = forward("checkForUpdates");
export const fetchChangelog = forward("fetchChangelog");
export const getSystemProxyUrl = forward("getSystemProxyUrl");
export const downloadUpdate = forward("downloadUpdate");
export const cancelUpdateDownload = forward("cancelUpdateDownload");
export const getDownloadedUpdate = forward("getDownloadedUpdate");
export const discardDownloadedUpdate = forward("discardDownloadedUpdate");
export const installDownloadedUpdate = forward("installDownloadedUpdate");
export const getAppVersion = forward("getAppVersion");
export const getAppSupportInfo = forward("getAppSupportInfo");

// Layout
export const saveSidebarLayout = forward("saveSidebarLayout");
export const loadSidebarLayout = forward("loadSidebarLayout");
export const saveTableVGroups = forward("saveTableVGroups");
export const loadTableVGroups = forward("loadTableVGroups");
export const deleteTableVGroupsForConnection = forward("deleteTableVGroupsForConnection");

// ---------------------------------------------------------------------------
// Re-export all types from tauri.ts (shared between both backends)
// ---------------------------------------------------------------------------

export type { AiConfigItem };

export type {
  AppSupportInfo,
  AiMessage,
  AiCompletionRequest,
  AiTaskContract,
  AiStreamChunk,
  AiModelInfo,
  AiChatMessage,
  AiConversation,
  AiRun,
  AiRunStatus,
  PromptTemplate,
  AgentDriverInfo,
  AgentOfflineArtifactKind,
  AgentOfflineExportUnavailableReason,
  AgentOfflineExportCandidate,
  AgentOfflineExportPreview,
  AgentOfflineExportResult,
  AgentOfflineImportFailure,
  AgentOfflineImportResult,
  DriverStoreUsage,
  DriverStoreUsageItem,
  DriverRuntimeHealth,
  DriverRuntimeStatus,
  DriverRuntimeInfo,
  DriverRuntimeSummary,
  JavaRuntimeMode,
  JavaRuntimeConfig,
  DriverInstallProgress,
  DriverStoreMigrationResult,
  DriverStorePathInfo,
  WebDavConfig,
  WebDavPasswordStatus,
  WebDavSyncSummary,
  SyncCatalogItem,
  PluginUiStorageItemRef,
  SyncSelection,
  SyncSnapshotCatalog,
  WebDavDownloadResult,
  SnippetProvider,
  SnippetSyncConfig,
  SnippetSyncSettings,
  SnippetSyncSummary,
  SnippetDownloadResult,
  SnippetTokenStatus,
  McpServerStatus,
  McpHttpServerSettings,
  McpHttpServerStatus,
  WebMcpHttpStatus,
  WebMcpHttpSettings,
  UpdateInfo,
  DownloadedUpdate,
  RedisBlob,
  RedisCollectionPage,
  RedisDatabaseInfo,
  RedisHashItem,
  RedisKeyInfo,
  RedisKeysExpiryResult,
  RedisListItem,
  RedisSetItem,
  RedisStreamConsumer,
  RedisStreamEntry,
  RedisStreamField,
  RedisStreamGroup,
  RedisStreamMetric,
  RedisStreamPage,
  RedisStreamPendingEntry,
  RedisStreamPendingPage,
  RedisValue,
  RedisValueData,
  RedisZsetItem,
  RedisScanResult,
  RedisCommandSafety,
  RedisCommandResult,
  RedisSlowlogEntry,
  RedisNodeEndpoint,
  KvValueEncoding,
  KvInt64,
  KvValue,
  KvKeyMetadata,
  KvKeySummary,
  KvListPrefixResponse,
  KvListPrefixOptions,
  KvGetOptions,
  KvGetResponse,
  KvWriteMode,
  KvCreateMode,
  KvPutOptions,
  KvPutResponse,
  KvDeleteOptions,
  KvDeleteResponse,
  KvHistoryEventType,
  KvHistoryEvent,
  KvHistoryResponse,
  KvStatusMember,
  KvPrometheusMetrics,
  KvStatusResponse,
  EtcdDefragResponse,
  EtcdDefragMemberResult,
  EtcdWatchStartRequest,
  EtcdWatchStartResponse,
  EtcdWatchPollResponse,
  EtcdLeaseListResponse,
  EtcdLeaseDetail,
  EtcdAuthUserListResponse,
  EtcdAuthUserDetail,
  EtcdAuthPermission,
  EtcdAuthRoleListResponse,
  EtcdAuthRoleDetail,
  EtcdPreflightResponse,
  EtcdDangerousApproval,
  DocumentQueryResult,
  DynamoDbKeyInfo,
  DynamoDbIndexInfo,
  DynamoDbTableDescription,
  MongoDocumentResult,
  HistoryEntry,
  HistoryConnectionFilter,
  HistoryDatabaseFilter,
  HistoryCursor,
  HistorySearchRequest,
  HistorySearchResult,
  HistoryConnectionOption,
  SqlFileStatus,
  SqlFileRequest,
  SqlFilePreview,
  SqlFileTable,
  SqlFileProgress,
  TransferRequest,
  TransferProgress,
  TransferMode,
  TransferContent,
  TransferObjectKind,
  TransferObjectSelection,
  TransferTableNameCase,
  TransferOwnershipPolicy,
  TransferStructureOperation,
  TransferStructureOperationKind,
  TransferOwnershipPreview,
  TableImportMode,
  TableImportConflictPolicy,
  TableImportStatus,
  TableImportSourceFormat,
  TableImportJsonShape,
  TableImportTextEncoding,
  TableImportColumnMapping,
  TableImportParseOptions,
  TableImportPreviewRequest,
  TableImportPreview,
  TableImportPreparedSource,
  TableImportRequest,
  TableImportSummary,
  TableImportProgress,
  MongoImportFormat,
  MongoImportTypeMode,
  MongoImportInferredType,
  MongoImportColumn,
  MongoImportIssue,
  MongoImportParseOptions,
  MongoImportPreviewRequest,
  MongoImportPreview,
  MongoImportRequest,
  MongoImportProgress,
  MongoImportSummary,
  MongoExportFormat,
  MongoExportRequest,
  MongoExportProgress,
  MongoExportSummary,
  DatabaseExportRequest,
  ExportProgress,
  TableExportProgress,
  TableExportStatus,
  TableExportRequest,
  QueryResultExportRequest,
  AgentEvent,
  SqlFileEntry,
  GlobalSearchRequest,
  GlobalSearchMatch,
  GlobalSearchSettings,
} from "@/lib/backend/tauri";

// MQTT
