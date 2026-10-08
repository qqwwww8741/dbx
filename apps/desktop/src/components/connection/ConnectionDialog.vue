<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { uuid } from "@/lib/common/utils";
import { useI18n } from "vue-i18n";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

import { Input } from "@/components/ui/input";

import type { ConnectionConfig, ConnectionTestResult, DatabaseConnectionInfo, DatabaseType, HttpTunnelConfig, InstalledPlugin, PluginFormFieldValue, ProxyTunnelConfig, SshConfigHostEntry, SshTunnelConfig, TransportLayerConfig } from "@/types/database";
import { CONNECTION_PICKER_OPTIONS, CONNECTION_PROFILES, CONNECTION_PROFILE_ICONS, type ConnectionPickerOption, type ConnectionProfileCategory, type ConnectionProfileDefinition } from "@/types/generated/connectionProfiles";
import type { InfluxDbExternalConfig, InfluxDbVersion } from "@/types/influxdb";
import type { VictoriaMetricsExternalConfig } from "@/types/victoriametrics";

import { useConnectionStore } from "@/stores/connectionStore";
import { useTunnelProfileStore } from "@/stores/tunnelProfileStore";

import { sanitizeConnectionCredentials } from "@/lib/connection/credentialSanitizer";
import { inferSshAuthMethod } from "@/lib/connection/sshAuthMethod";

import { canPersistConnectionTestResult, connectionEditDraftSyncAction } from "./connectionEditDraftSync";
import { createConnectionNoteVisibilityDraft, resetConnectionNoteVisibilityDraft, syncConnectionNoteVisibilityDraft } from "./connectionNoteVisibilityDraft";

import { useSettingsStore } from "@/stores/settingsStore";
import { useToast } from "@/composables/useToast";
import DatabaseIcon from "@/components/icons/DatabaseIcon.vue";

import PluginIcon from "@/components/plugins/PluginIcon.vue";
import * as api from "@/lib/backend/api";
import { createFrontendPluginRegistry, parsePluginConnectionProviderOptionValue, pluginConnectionProviderIcon, pluginConnectionProviderOptionValue } from "@/lib/plugins/frontendPlugin";
import type { PluginCenterFocus } from "@/lib/plugins/pluginCenterNavigation";

import { applyParsedConnectionUrl, parseConnectionUrl } from "@/lib/connection/connectionUrl";

import { DEFAULT_QUERY_TIMEOUT_SECS } from "@/lib/connection/timeoutLimits";

import { applyConnectionDeepLinkUpdate } from "@/lib/connection/connectionDeepLinkUpdate";
import { parseConnectionDeepLink, parseConnectionDeepLinkUpdate, type ConnectionDeepLinkDraft, type ConnectionDeepLinkUpdate } from "@/lib/connection/connectionDeepLink";
import { connectionUrlPlaceholder as getUrlPlaceholder } from "@/lib/connection/connectionPresentation";

import { copyToClipboard } from "@/lib/common/clipboard";
import { connectionConfigFingerprint, normalizeDatabaseConnectionInfo } from "@/lib/connection/connectionDatabaseInfo";

import { ensureJdbcxRuntimeDrivers } from "@/lib/database/jdbcxBuiltinDriver";
import { notifyComponentUpdatesChanged } from "@/lib/updates/componentUpdateEvents";

import { connectionAttemptOriginalErrorMessage, connectionAttemptTimeoutMessage, connectionAttemptTimeoutMs } from "@/lib/connection/connectionAttemptTimeout";

import { appendConnectionErrorHints } from "@/lib/connection/connectionErrorHints";

import { savedMysqlTlsFormFields, supportsMysqlTlsOptions as mysqlTlsOptionsSupported, supportsMysqlTlsTab } from "@/lib/connection/mysqlTlsCapabilities";
import { copyDialogPasswordFieldValue, preventDialogDocumentSelectAll } from "@/lib/connection/dialogTextSelection";

import { assertCompleteDatabaseCategories, databaseSelectionForCategory } from "@/lib/connection/databaseCategoryOptions";
import { loadConnectionPickerView, saveConnectionPickerView, type DbPickerView } from "@/lib/connection/connectionPickerViewPreference";

import { ArrowLeft, Check, CheckSquare, ChevronRight, Copy, Grid3X3, List, ListFilter, Loader2, RefreshCw, Search, Square } from "@lucide/vue";
import { buildDraftVisibleDatabasesConnectionId, connectionCanChooseVisibleDatabases, initialVisibleDatabaseSelection, visibleObjectFiltersNeedReset } from "@/lib/connection/connectionVisibleDatabases";
import { resolveVisibleDatabaseSaveAction } from "@/components/sidebar/visibleDatabasesDialogState";
import { canSaveVisibleDatabaseSelection, connectionUsesVisibleSchemaFilter, filterDatabaseNamesForVisiblePicker, filterSchemaNamesForVisiblePicker, normalizeVisibleSchemaSelection } from "@/lib/database/visibleDatabases";

import { normalizeConnectionScope, normalizeConnectionTimeouts } from "@/lib/connection/connectionSubmitNormalization";

import VisibleSchemasDialog from "@/components/sidebar/VisibleSchemasDialog.vue";

import { translateBackendError } from "@/i18n/backend-errors";

import { gaussdbConnectionMode } from "@/lib/database/jdbcDialect";
import { normalizeStoredConnectionDatabase } from "@/lib/database/connectionDatabase";

type DbOption = Omit<ConnectionPickerOption, "category"> & { category?: DbCategoryKey; plugin?: boolean; pluginId?: string; pluginIcon?: string };
type DbCategoryKey = ConnectionProfileCategory | "plugins";
type DbCategory = { key: DbCategoryKey; title: string; options: DbOption[] };
type DialogStep = "select" | "config";
export type ConfigTab = "connection" | "advanced" | "tls" | "transport";

const DEFAULT_SSH_USER = "root";

// The picker merges the Ignite 2.x/3.x cards into a single "Apache Ignite"
// entry; the version is picked inside the connection form instead.
const MERGED_PICKER_OPTION_FOR_TYPE: Record<string, string> = {};
const PICKER_SEARCH_ALIASES: Record<string, string[]> = {};

type LegacyTransportFields = {
  ssh_enabled?: boolean;
  ssh_host?: string;
  ssh_port?: number;
  ssh_user?: string;
  ssh_password?: string;
  ssh_key_path?: string;
  ssh_key_passphrase?: string;
  ssh_expose_lan?: boolean;
  ssh_connect_timeout_secs?: number;
  ssh_tunnels?: SshTunnelConfig[];
  proxy_enabled?: boolean;
  proxy_type?: "socks5" | "http";
  proxy_host?: string;
  proxy_port?: number;
  proxy_username?: string;
  proxy_password?: string;
};
type LegacyConnectionConfig = ConnectionConfig & LegacyTransportFields;
type ConnectionForm = Omit<ConnectionConfig, "id">;
type ConnectionTestState = ConnectionTestResult & { ok: boolean; scope?: "connection" | "ssh" };
type PluginActionStatus = { ok: boolean; message: string };
type SaveConnectionOptions = { connectAfterSave?: boolean; closeOnSuccess?: boolean };

const { t, locale: appLocale } = useI18n();
const { toast } = useToast();
const settingsStore = useSettingsStore();
const connectionNoteVisibilityDraft = reactive(createConnectionNoteVisibilityDraft(settingsStore.editorSettings.sidebarShowConnectionNotes));

const editGlobalConnectTimeoutSecs = ref(settingsStore.editorSettings.globalConnectTimeoutSecs);
const editGlobalQueryTimeoutSecs = ref(settingsStore.editorSettings.globalQueryTimeoutSecs);
const open = defineModel<boolean>("open", { default: false });

const props = defineProps<{
  editConfig?: ConnectionConfig;
  prefillConfig?: ConnectionDeepLinkDraft | null;
  updatePrefill?: ConnectionDeepLinkUpdate | null;
  pluginProvider?: PluginCenterFocus | null;
  initialTab?: ConfigTab;
}>();

const store = useConnectionStore();

const selectedConnectionGroupId = ref<string | null>(null);

const connectionGroupOptions = computed(() => store.connectionGroupOptions.map((group) => ({ id: group.id, label: group.path.join(" / ") })));

function initialConnectionGroupId(): string | null {
  const preferred = store.newConnectionGroupId ?? store.selectedConnectionGroupId;
  return preferred && connectionGroupOptions.value.some((group) => group.id === preferred) ? preferred : null;
}
const tunnelProfileStore = useTunnelProfileStore();
const isTesting = ref(false);
const isTestingSshTunnel = ref(false);
const isSaving = ref(false);
const testResult = ref<ConnectionTestState | null>(null);
const testedConfigFingerprint = ref("");
const testedConfigId = ref("");
const testedGeneratedName = ref("");
const savedDatabaseInfo = ref<DatabaseConnectionInfo | null>(null);
const savedDatabaseInfoFingerprint = ref("");
const savedConnectionConfigFingerprint = ref("");

/** Set when the user cancels from the modal, so the pending promise's
 * "canceled by user" error is treated as a non-failure by its caller. */

const showConnectionErrorDialog = ref(false);
const connectionErrorRawDetail = ref("");
const connectionErrorDetail = ref("");
const testResultCopied = ref(false);
const connectionErrorCopied = ref(false);
const editingId = ref<string | null>(null);
const pluginFormValues = ref<Record<string, PluginFormFieldValue>>({});
const pluginLoadError = ref("");

const pluginActionStatus = ref<PluginActionStatus | null>(null);
const draftTestConnectionId = ref(uuid());
const showVisibleDatabasesDialog = ref(false);
const isLoadingVisibleDatabases = ref(false);
const visibleDatabaseNames = ref<string[]>([]);
const visibleDatabaseSelection = ref<Set<string>>(new Set());
const visibleDatabaseSearchText = ref("");
const visibleDatabaseError = ref("");
const visibleDatabaseShowSystem = ref(false);

const showProductionDatabasesDialog = ref(false);
const isLoadingProductionDatabases = ref(false);
const productionDatabaseNames = ref<string[]>([]);
const productionDatabaseSelection = ref<Set<string>>(new Set());
const productionDatabaseSearchText = ref("");
const productionDatabaseError = ref("");
const productionProtectionEnabled = ref(false);
const showVisibleSchemasDialog = ref(false);
const isLoadingVisibleSchemas = ref(false);
const visibleSchemaNames = ref<string[]>([]);
const visibleSchemaInitialSelection = ref<string[]>([]);
const visibleSchemaError = ref("");
const installedPlugins = ref<InstalledPlugin[]>([]);
let testRunId = 0;

function initialConfigTab(): ConfigTab {
  return props.initialTab ?? "connection";
}

const defaultForm = (): ConnectionForm => ({
  name: "",
  note: "",
  db_type: "mysql",
  driver_profile: "mysql",
  driver_label: "MySQL",
  url_params: "",

  host: "127.0.0.1",
  port: 3306,
  username: "root",
  password: "",
  database: undefined,
  color: "",
  transport_layers: [],
  connect_timeout_secs: settingsStore.editorSettings.globalConnectTimeoutSecs,
  connect_timeout_inherit: true,
  query_timeout_secs: settingsStore.editorSettings.globalQueryTimeoutSecs,
  query_timeout_inherit: true,
  idle_timeout_secs: 60,
  keepalive_interval_secs: 30,
  ssl: false,
  ca_cert_path: "",
  client_cert_path: "",
  client_key_path: "",
  sysdba: false,

  connection_string: undefined,

  external_config: undefined,
  init_script: undefined,
  docs_notes_path: undefined,
  read_only: false,
  show_system_schemas: false,
  show_database_links: true,
  sidebar_auto_load_all_tables: false,
  is_production: false,
  production_databases: [],
  visible_databases: undefined,
  save_password: true,
});

function normalizeSshTunnel(hop: Partial<SshTunnelConfig>): SshTunnelConfig {
  return {
    id: hop.id || uuid(),
    name: hop.name || "",
    enabled: hop.enabled !== false,
    host: hop.host || "",
    port: Number(hop.port) || 22,
    user: hop.user?.trim() || DEFAULT_SSH_USER,
    password: hop.password || "",
    key_path: hop.key_path || "",
    key_passphrase: hop.key_passphrase || "",
    connect_timeout_secs: Number(hop.connect_timeout_secs) || 5,
    expose_lan: !!hop.expose_lan,

    auth_method: hop.auth_method || inferSshAuthMethod(hop),
    allow_exec_channel_proxy: !!hop.allow_exec_channel_proxy,
    proxy_command: hop.proxy_command || "",
    profile_id: hop.profile_id || undefined,
  };
}

function normalizeProxyTunnel(layer: Partial<ProxyTunnelConfig>): ProxyTunnelConfig {
  return {
    id: layer.id || uuid(),
    name: layer.name || "",
    enabled: layer.enabled !== false,
    proxy_type: layer.proxy_type || "socks5",
    host: layer.host || "",
    port: Number(layer.port) || 1080,
    username: layer.username || "",
    password: layer.password || "",
    profile_id: layer.profile_id || undefined,
  };
}

function normalizeHttpTunnel(layer: Partial<HttpTunnelConfig>): HttpTunnelConfig {
  return {
    id: layer.id || uuid(),
    name: layer.name || "",
    enabled: layer.enabled !== false,
    url: layer.url || "",
    token: layer.token || "",
    connect_timeout_secs: Number(layer.connect_timeout_secs) || 10,
    profile_id: layer.profile_id || undefined,
  };
}

function normalizeTransportLayer(layer: Partial<TransportLayerConfig>): TransportLayerConfig {
  if (layer.type === "proxy") {
    return { type: "proxy", ...normalizeProxyTunnel(layer) };
  }
  if (layer.type === "http_tunnel") {
    return { type: "http_tunnel", ...normalizeHttpTunnel(layer) };
  }
  return { type: "ssh", ...normalizeSshTunnel(layer as Partial<SshTunnelConfig>) };
}

function transportLayersForConfig(config: LegacyConnectionConfig): TransportLayerConfig[] {
  if (config.transport_layers?.length) {
    return config.transport_layers.map(normalizeTransportLayer);
  }
  const layers: TransportLayerConfig[] = sshLayersForConfig(config).map((hop) => ({ type: "ssh", ...hop }));
  if (config.proxy_enabled || config.proxy_host || config.proxy_username || config.proxy_password) {
    layers.push({
      type: "proxy",
      ...normalizeProxyTunnel({
        id: "legacy-proxy",
        enabled: true,
        proxy_type: config.proxy_type || "socks5",
        host: config.proxy_host || "",
        port: config.proxy_port || 1080,
        username: config.proxy_username || "",
        password: config.proxy_password || "",
      }),
    });
  }
  return layers;
}

function sshLayersForConfig(config: LegacyConnectionConfig): SshTunnelConfig[] {
  if (config.ssh_tunnels?.length) {
    return config.ssh_tunnels.map(normalizeSshTunnel);
  }
  if (config.ssh_enabled || config.ssh_host || config.ssh_user || config.ssh_password || config.ssh_key_path || config.ssh_key_passphrase) {
    return [
      normalizeSshTunnel({
        id: "legacy",
        enabled: true,
        host: config.ssh_host || "",
        port: config.ssh_port || 22,
        user: config.ssh_user || "",
        password: config.ssh_password || "",
        key_path: config.ssh_key_path || "",
        key_passphrase: config.ssh_key_passphrase || "",
        connect_timeout_secs: config.ssh_connect_timeout_secs || 5,
        expose_lan: config.ssh_expose_lan || false,
      }),
    ];
  }
  return [];
}

const form = ref(defaultForm());

const noteTextareaRef = ref<HTMLTextAreaElement | null>(null);

function resizeNoteTextarea() {
  const textarea = noteTextareaRef.value;
  if (!textarea) return;

  const style = window.getComputedStyle(textarea);
  const lineHeight = Number.parseFloat(style.lineHeight) || 20;
  const paddingHeight = (Number.parseFloat(style.paddingTop) || 0) + (Number.parseFloat(style.paddingBottom) || 0);
  const borderHeight = (Number.parseFloat(style.borderTopWidth) || 0) + (Number.parseFloat(style.borderBottomWidth) || 0);
  const maxContentHeight = lineHeight * 3 + paddingHeight;

  textarea.style.height = "auto";
  textarea.style.height = `${Math.min(textarea.scrollHeight, maxContentHeight) + borderHeight}px`;
  textarea.style.overflowY = textarea.scrollHeight > maxContentHeight ? "auto" : "hidden";
}

const selectedTransportLayerId = ref<string | null>(null);
const draggedTransportLayerId = ref<string | null>(null);
const selectedType = ref("mysql");
const pluginRegistry = computed(() => createFrontendPluginRegistry(installedPlugins.value, appLocale.value));
const pluginConnectionProviders = computed(() => pluginRegistry.value.listConnectionProviders());
const selectedPluginProvider = computed(() => pluginProviderEntryForOption(selectedType.value));

const customDriverName = ref("");

const sshConfigHosts = ref<SshConfigHostEntry[]>([]);

const connectionUrlInput = ref("");
const appliedConnectionUrlInput = ref("");

/** 每次进入 OCI 模式只提醒一次 Instant Client 目录，避免反复点“测试”时刷屏。 */

const oceanbaseSubMode = ref<"mysql">("mysql");

const dialogStep = ref<DialogStep>("select");
const dbPickerView = ref<DbPickerView>(loadConnectionPickerView());
const dbSearchQuery = ref("");
const selectedDbCategory = ref<DbCategoryKey>("sql");
const configTab = ref<ConfigTab>("connection");

// 对话框拖动功能
const dragOffset = ref({ x: 0, y: 0 });
const isDraggingDialog = ref(false);
const dragStartPos = ref({ x: 0, y: 0 });
const dragStartOffset = ref({ x: 0, y: 0 });
const activePointerId = ref<number | null>(null);

// 计算对话框的定位样式（通过 transform 实现拖动）
const dialogContentStyle = computed(() => {
  if (isDraggingDialog.value || dragOffset.value.x !== 0 || dragOffset.value.y !== 0) {
    return {
      transform: `translate(${dragOffset.value.x}px, ${dragOffset.value.y}px)`,
      transition: isDraggingDialog.value ? "none" : "transform 0.15s ease-out",
    };
  }
  return {};
});

// 开始拖动（在 DialogHeader 上按下鼠标/触摸）
function onDialogHeaderPointerDown(e: PointerEvent) {
  if (e.button !== undefined && e.button !== 0) return;
  isDraggingDialog.value = true;
  activePointerId.value = e.pointerId;
  dragStartPos.value = { x: e.clientX, y: e.clientY };
  dragStartOffset.value = { ...dragOffset.value };
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
}

// 拖动中
function onDialogHeaderPointerMove(e: PointerEvent) {
  if (!isDraggingDialog.value || e.pointerId !== activePointerId.value) return;
  const dx = e.clientX - dragStartPos.value.x;
  const dy = e.clientY - dragStartPos.value.y;
  dragOffset.value = {
    x: dragStartOffset.value.x + dx,
    y: dragStartOffset.value.y + dy,
  };
}

// 结束拖动
function onDialogHeaderPointerEnd(e: PointerEvent) {
  if (!isDraggingDialog.value || e.pointerId !== activePointerId.value) return;
  isDraggingDialog.value = false;
  activePointerId.value = null;
  try {
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
  } catch {
    // 忽略 release 失败的错误
  }
}

// 重置拖动位置
function resetDialogDragOffset() {
  dragOffset.value = { x: 0, y: 0 };
  isDraggingDialog.value = false;
  activePointerId.value = null;
}

// 监听对话框 open 状态，重置位置
watch(open, (isOpen) => {
  if (isOpen) {
    nextTick(() => resetDialogDragOffset());
  } else {
    resetDialogDragOffset();
  }
});
watch([() => form.value.note, configTab, dialogStep, open], () => {
  void nextTick(resizeNoteTextarea);
});

// Nacos 2 and 3 expose different API planes. New connections must therefore
// choose an explicit version instead of relying on endpoint-shape guessing.

// --- MQTT-specific form fields ---

const customColorInput = ref("");

const driverProfiles: Record<string, ConnectionProfileDefinition> = {
  ...CONNECTION_PROFILES,
};

function profileForConfig(config: ConnectionConfig) {
  {}
  {}
  if (config.driver_profile && driverProfiles[config.driver_profile]) {
    {
    }
    return config.driver_profile;
  }
  {}
  {}
  {}
  return config.db_type;
}

function selectedProfile() {
  {}
  const profile = selectedType.value;
  return driverProfiles[profile] ?? driverProfiles.mysql;
}

function pluginProviderEntry(pluginId: string, providerId: string) {
  return pluginConnectionProviders.value.find((entry) => entry.plugin.manifest.id === pluginId && entry.contribution.id === providerId) || null;
}

function pluginProviderEntryForOption(value: string) {
  const target = parsePluginConnectionProviderOptionValue(value);
  return target ? pluginProviderEntry(target.pluginId, target.providerId) : null;
}

const influxDbVersion = ref<InfluxDbVersion>("1");
const influxDbOrg = ref("");
const victoriaMetricsApiPath = ref("/prometheus");
const victoriaMetricsLookback = ref("1h");

function resetInfluxDbFields(config?: Partial<InfluxDbExternalConfig>, versionHint?: InfluxDbVersion) {
  const version = versionHint ?? (config?.version === "2" ? "2" : config?.version === "3" ? "3" : "1");
  influxDbVersion.value = version;
  influxDbOrg.value = config?.org?.trim() || "";
}

function resetVictoriaMetricsFields(config?: Partial<VictoriaMetricsExternalConfig>) {
  victoriaMetricsApiPath.value = config?.apiPath?.trim() || "/prometheus";
  victoriaMetricsLookback.value = config?.lookback?.trim() || "1h";
}

// ---------------------------------------------------------------------------
// Salesforce OAuth / device-code flow state
// ---------------------------------------------------------------------------

// Username-password (ROPC) mode credentials. The password is never
// round-tripped from the backend; blank on edit unless re-typed.

// When editing an existing connection whose credentials live in the backend's
// secret store, refreshToken/clientSecret are intentionally not sent back to
// the UI. Track "we have previously authorized" separately from whether we
// still hold the tokens in-memory so the form can show a status chip.

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

function connectionErrorWithDriverUpdateHint(config: ConnectionConfig, message: string): string {
  message = appendConnectionErrorHints(config, message, t);
  {
    return message;
  }
}

/**
 * Clear the install dialog, unless a newer operation now owns it. Stale
 * operation promises (cancelled, then retried before settling) must not
 * finish/reset the retry's dialog state.
 */

/**
 * Abort an in-flight agent driver install from the modal's Cancel button.
 * The backend stops the download; the pending `installAgent` promise resolves
 * with a "canceled by user" error, which callers treat as a non-failure.
 */

function showConnectionError(message: string) {
  connectionErrorRawDetail.value = message;
  connectionErrorDetail.value = translateBackendError(t, message);
  connectionErrorCopied.value = false;
  showConnectionErrorDialog.value = true;
}

async function ensureRequiredJdbcxDriverInstalled(config: ConnectionConfig): Promise<void> {
  const result = await ensureJdbcxRuntimeDrivers(config, api, () => {
    testResult.value = { ok: true, message: "Installing JDBC plugin..." };
  });
  if (!result) return;
  notifyComponentUpdatesChanged();

  if (result.paths.length > 0) {
  }
}

function clearTestedConnectionInfo() {
  testedConfigFingerprint.value = "";
  testedConfigId.value = "";
  testedGeneratedName.value = "";
}

function clearSavedDatabaseInfo() {
  savedDatabaseInfo.value = null;
  savedDatabaseInfoFingerprint.value = "";
  savedConnectionConfigFingerprint.value = "";
}

function applySavedDatabaseInfo(config: ConnectionConfig) {
  clearSavedDatabaseInfo();
  try {
    const current = connectionConfigForSubmit(config.id, config.name);
    savedConnectionConfigFingerprint.value = connectionConfigFingerprint(current, form.value.name);
    const info = normalizeDatabaseConnectionInfo(config.database_info);
    if (info) {
      savedDatabaseInfo.value = info;
      savedDatabaseInfoFingerprint.value = savedConnectionConfigFingerprint.value;
    }
  } catch {
    clearSavedDatabaseInfo();
  }
}

function applySuccessfulConnectionTest(result: ConnectionTestResult, config: ConnectionConfig, sourceName: string) {
  testResult.value = { ok: true, ...result };
  testedConfigFingerprint.value = connectionConfigFingerprint(config, sourceName);
  testedConfigId.value = config.id;
  testedGeneratedName.value = config.name;
}

async function persistSuccessfulConnectionTest(result: ConnectionTestResult, config: ConnectionConfig, sourceName: string, runId: number) {
  if (!editingId.value || !result.databaseInfo || !savedConnectionConfigFingerprint.value) return;
  const fingerprint = connectionConfigFingerprint(config, sourceName);
  let currentDraftFingerprint: string;
  try {
    const currentDraft = connectionConfigForSubmit(editingId.value, form.value.name);
    currentDraftFingerprint = connectionConfigFingerprint(currentDraft, form.value.name);
  } catch {
    return;
  }
  // An in-flight test must not publish its saved snapshot after the user edits,
  // switches, or closes the draft that initiated it.
  if (
    !canPersistConnectionTestResult({
      testConfigId: config.id,
      activeDraftId: editingId.value,
      testRunId: runId,
      activeTestRunId: testRunId,
      submittedFingerprint: fingerprint,
      savedFingerprint: savedConnectionConfigFingerprint.value,
      currentDraftFingerprint,
    })
  ) {
    return;
  }
  const persistedDraftId = editingId.value;
  try {
    await store.updateConnectionDatabaseInfo(persistedDraftId, result.databaseInfo);
    if (runId !== testRunId || editingId.value !== persistedDraftId) return;
    savedDatabaseInfo.value = { ...result.databaseInfo };
    savedDatabaseInfoFingerprint.value = fingerprint;
  } catch {
    // The successful test remains valid even when optional metadata persistence fails.
  }
}

async function testConnectionWithTimeout(config: ConnectionConfig, runId: number): Promise<ConnectionTestResult> {
  await tunnelProfileStore.init();
  const timeoutMs = connectionAttemptTimeoutMs(config, tunnelProfileStore.profileById);
  const timeoutMessage = connectionAttemptTimeoutMessage(timeoutMs);
  const promise = api.testConnectionWithInfo(config);
  let timedOut = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  void promise.catch((error) => {
    if (!timedOut) return;
    if (runId !== testRunId) return;
    clearTestedConnectionInfo();
    testResult.value = {
      ok: false,
      message: connectionErrorWithDriverUpdateHint(config, connectionAttemptOriginalErrorMessage(timeoutMessage, errorMessage(error))),
    };
  });
  try {
    return await Promise.race([
      promise,
      new Promise<ConnectionTestResult>((_, reject) => {
        timer = setTimeout(() => {
          timedOut = true;
          reject(new Error(timeoutMessage));
        }, timeoutMs);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

function isCustomCompatibleProfile() {
  return selectedType.value === "custom_mysql" || selectedType.value === "custom_postgres";
}

function connectionUrlPreferredProfile() {
  {}
  return selectedType.value;
}

function applyProfile(val: string, preserveConnectionFields = false) {
  const profile = driverProfiles[val];
  if (!profile) return;

  selectedType.value = val;
  form.value.db_type = profile.type;
  form.value.driver_profile = val;
  form.value.driver_label = isCustomCompatibleProfile() ? customDriverName.value.trim() || profile.label : profile.label;

  {
    form.value.external_config = undefined;
  }
  {}
  {}
  if (!preserveConnectionFields) {
    form.value.port = profile.port;

    form.value.username = profile.user;
    form.value.url_params = profile.urlParams || "";

    if (profile.host) {
      form.value.host = profile.host;
    }

    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
    {
    }
  }
  {}
}

async function loadInstalledPlugins() {
  pluginLoadError.value = "";
  try {
    installedPlugins.value = await api.listPlugins();
  } catch (cause) {
    installedPlugins.value = [];
    pluginLoadError.value = cause instanceof Error ? cause.message : String(cause);
  }
}

let applyingConnectionUpdate = false;
let appliedConnectionUpdate: ConnectionDeepLinkUpdate | null = null;

function finishApplyingConnectionUpdate() {
  void nextTick(() => {
    applyingConnectionUpdate = false;
  });
}

// The external_config shaped by an update link must survive submit verbatim
// only while the submitted values are still the ones the link patched: once
// the user edits the port away from the patched value, or the patch targeted
// another connection, the flag is re-derived from the form like any edit.
function connectionUpdateExternalConfigPreserved(config: Pick<ConnectionConfig, "id" | "port">): boolean {
  if (!appliedConnectionUpdate || appliedConnectionUpdate.connectionId !== config.id) return false;
  const patchedPort = appliedConnectionUpdate.patch.port;
  return patchedPort === undefined || patchedPort === config.port;
}

watch(
  [() => props.editConfig, open],
  ([savedConfig, isOpen]) => {
    const syncAction = connectionEditDraftSyncAction(savedConfig?.id ?? null, isOpen, editingId.value);
    if (syncAction === "preserve") return;
    // Hydrate a detached edit draft before form watchers observe it. Do not
    // mutate the saved record or apply create defaults to an ID update. Only
    // flag the update as applied when the prefill really targeted this
    // connection, so an ID mismatch cannot freeze port-flag recomputation.
    const connectionUpdate = savedConfig && props.updatePrefill?.connectionId === savedConfig.id ? props.updatePrefill : null;
    const config = connectionUpdate && savedConfig ? applyConnectionDeepLinkUpdate(savedConfig, connectionUpdate) : savedConfig;
    resetConnectionNoteVisibilityDraft(connectionNoteVisibilityDraft, settingsStore.editorSettings.sidebarShowConnectionNotes);
    editGlobalConnectTimeoutSecs.value = settingsStore.editorSettings.globalConnectTimeoutSecs;
    editGlobalQueryTimeoutSecs.value = settingsStore.editorSettings.globalQueryTimeoutSecs;
    if (syncAction === "hydrate" && config) {
      appliedConnectionUpdate = connectionUpdate;
      if (connectionUpdate) applyingConnectionUpdate = true;
      clearSavedDatabaseInfo();
      const legacyConfig = config as LegacyConnectionConfig;
      const profile = profileForConfig(config);

      editingId.value = config.id;
      const profileConfig = driverProfiles[profile];
      form.value = {
        name: config.name,
        note: config.note || "",
        db_type: profileConfig?.type || config.db_type,
        driver_profile: config.driver_profile || profile,
        driver_label: config.driver_label || undefined || driverProfiles[profile]?.label || config.db_type,
        ...savedMysqlTlsFormFields(config),

        host: config.host,
        port: config.port,
        username: config.username,
        password: config.password,
        // Show the index the backend actually connects with; legacy dirty values
        // (e.g. redis-cli flags in the field) are healed when the form is saved.
        database: config.database,
        color: config.color || "",
        transport_layers: transportLayersForConfig(legacyConfig),
        connect_timeout_secs: config.connect_timeout_inherit === true ? settingsStore.editorSettings.globalConnectTimeoutSecs : config.connect_timeout_secs || 10,
        connect_timeout_inherit: config.connect_timeout_inherit === true,
        query_timeout_secs: config.query_timeout_inherit === true ? settingsStore.editorSettings.globalQueryTimeoutSecs : (config.query_timeout_secs ?? DEFAULT_QUERY_TIMEOUT_SECS),
        query_timeout_inherit: config.query_timeout_inherit === true,
        idle_timeout_secs: config.idle_timeout_secs ?? 60,
        keepalive_interval_secs: config.keepalive_interval_secs ?? 30,
        sysdba: config.sysdba,

        connection_string: config.connection_string,

        external_config: config.external_config,
        attached_databases: config.attached_databases || [],
        init_script: config.init_script,
        docs_notes_path: config.docs_notes_path,
        read_only: config.read_only || false,
        show_system_schemas: config.show_system_schemas || false,
        show_database_links: config.show_database_links !== false,
        sidebar_auto_load_all_tables: config.sidebar_auto_load_all_tables === true,
        is_production: config.is_production || false,
        production_databases: config.production_databases || [],
        visible_databases: config.visible_databases,
        visible_schemas: config.visible_schemas,
        save_password: config.save_password !== false,
      };

      productionProtectionEnabled.value = !!config.is_production || (config.production_databases?.length ?? 0) > 0;
      connectionUrlInput.value = "";
      appliedConnectionUrlInput.value = connectionUrlInput.value.trim();
      {
      }
      {
      }

      {
      }
      {
      }
      {
      }
      {
        resetInfluxDbFields();
      }
      {
        resetVictoriaMetricsFields();
      }
      {
      }

      customColorInput.value = config.color || "";
      selectedTransportLayerId.value = form.value.transport_layers?.[0]?.id || null;
      selectedType.value = profile;
      {
        pluginFormValues.value = {};
      }
      {
      }
      {
      }

      customDriverName.value = isCustomCompatibleProfile() ? config.driver_label || "" : "";
      dialogStep.value = "config";
      configTab.value = initialConfigTab();
      if (props.updatePrefill) finishApplyingConnectionUpdate();
      // Form/profile watchers normalize derived fields in this flush. Capture
      // the saved baseline afterwards so those initial changes are not treated
      // as user edits that invalidate persisted database metadata.
      void nextTick(() => {
        if (open.value && props.editConfig?.id === config.id && !props.updatePrefill) applySavedDatabaseInfo(config);
      });
    } else {
      appliedConnectionUpdate = null;
      clearSavedDatabaseInfo();
      editingId.value = null;
      selectedConnectionGroupId.value = initialConnectionGroupId();
      form.value = defaultForm();

      productionProtectionEnabled.value = false;
      selectedTransportLayerId.value = null;
      selectedType.value = "mysql";
      pluginFormValues.value = {};
      customDriverName.value = "";

      resetInfluxDbFields();

      oceanbaseSubMode.value = "mysql";

      dialogStep.value = "select";
      configTab.value = "connection";
    }
    resetTestState();
  },
  { immediate: true },
);

watch(
  () => settingsStore.editorSettings.sidebarShowConnectionNotes,
  (value) => syncConnectionNoteVisibilityDraft(connectionNoteVisibilityDraft, value),
);

const isEditing = ref(false);
watch(
  () => editingId.value,
  (v) => {
    isEditing.value = !!v;
  },
);

// 删除连接时若开启了「记住连接名与数据库」，新建同名**同类型**连接会自动选中记住的数据库。
// 只在数据库字段为空、或仍是上一次自动回填的值时才覆盖，避免抢走用户手输的内容。
const lastRememberedDatabaseAutofill = ref("");
watch(
  () => [open.value, editingId.value, form.value.name, form.value.db_type] as const,
  ([isOpen, editing, rawName, dbType]) => {
    if (!isOpen || editing) {
      lastRememberedDatabaseAutofill.value = "";
      return;
    }
    const name = (rawName ?? "").trim();
    const remembered = name ? settingsStore.rememberedDatabaseForConnection(name, dbType) : "";
    const current = (form.value.database ?? "").trim();
    if (current === remembered) {
      lastRememberedDatabaseAutofill.value = remembered;
      return;
    }
    if (current && current !== lastRememberedDatabaseAutofill.value) return;
    form.value.database = remembered || undefined;
    lastRememberedDatabaseAutofill.value = remembered;
  },
  { immediate: true },
);

function transportLayerDefaultName(layer: TransportLayerConfig, index: number): string {
  if (layer.type === "proxy") return `Proxy ${index + 1}`;
  if (layer.type === "http_tunnel") return t("connection.httpTunnelDefaultName", { index: index + 1 });
  return t("connection.sshHopDefaultName", { index: index + 1 });
}

function onDbTypeChange(val: string) {
  if (!editingId.value && val === selectedType.value) return;
  if (!editingId.value) {
    resetForm({ preservePickerState: true });
  }
  const category = dbCategoryForOption(val);
  if (category) selectedDbCategory.value = category;
  // Keep in sync with PLUGIN_CONNECTION_PROVIDER_OPTION_PREFIX in lib/plugins/frontendPlugin.
  if (val.startsWith("plugin-provider:")) {
    onPluginProviderOptionChange(val);
    return;
  }
  customDriverName.value = "";
  applyProfile(val, !!editingId.value);
  resetTestState();
  resetVisibleSchemasState();
}

function onPluginProviderOptionChange(val: string) {
  const pluginTarget = parsePluginConnectionProviderOptionValue(val);
  if (pluginTarget) {
    false;
  }
  resetTestState();
  resetVisibleSchemasState();
}

/**
 * Oracle driver mode: thin (go-ora agent, default) or OCI (thick driver,
 * requires a globally configured Oracle Instant Client). The oci.dll path
 * lives in the global settings on purpose — one configuration is shared by
 * every OCI connection, and new OCI connections backfill it automatically.
 */
// OCI（thick）驱动目前只发布 Windows x64 产物：非 Windows 平台不显示模式切换，
// 避免用户选到无法安装的驱动。已保存的 OCI 连接仍按原样打开（连接时会得到
// 明确的“驱动未安装”错误），并把 Thin 按钮留在原地便于切回。

/**
 * NLS_LANG is a per-connection override: an empty value follows the global
 * default, a filled value makes this connection own a dedicated agent process
 * (the variable is process-scoped for OCI). The dropdown lists the common
 * client character sets; the save button promotes the current value to the
 * global default for reuse.
 */

/** A stored value that predates the preset list must stay selectable. */

/** Names the global default in effect, so promoting a value becomes visible in place. */

/**
 * 连接级 TNS_ADMIN（tnsnames.ora / sqlnet.ora / 钱包目录）：留空跟随全局默认，
 * 填写后本连接使用自己的目录——ADB 钱包、sqlnet.ora 网络选项因此不依赖
 * TNS 连接方式。目录同样是进程级的，随 agent 启动注入。
 */

/**
 * Prompts for the Oracle Instant Client directory. The desktop shell is the
 * only runtime that can resolve a local path, so the picker explains itself
 * on the web build instead of silently doing nothing.
 */

const iconTypeMap: Record<string, string> = {
  ...CONNECTION_PROFILE_ICONS,
};

const dbCategoryMetadata: Array<{ key: DbCategoryKey; titleKey: string }> = [
  { key: "sql", titleKey: "connection.databaseCategorySql" },
  { key: "analytics", titleKey: "connection.databaseCategoryAnalytics" },
  { key: "domestic", titleKey: "connection.databaseCategoryDomestic" },
  { key: "lightweight", titleKey: "connection.databaseCategoryLightweight" },
  { key: "document", titleKey: "connection.databaseCategoryDocument" },
  { key: "graph_ai", titleKey: "connection.databaseCategoryGraphAi" },
  { key: "timeseries", titleKey: "connection.databaseCategoryTimeseries" },
  { key: "mq", titleKey: "connection.databaseCategoryMq" },
  { key: "registry_config", titleKey: "connection.databaseCategoryRegistryConfig" },
];

// `influxdb3` is presented as a version option inside the InfluxDB card
// (see the version <Select> below), not as a standalone picker entry.
const dbOptions: DbOption[] = [...CONNECTION_PICKER_OPTIONS.filter((_option) => true)];

const dbCategoryDefinitions = dbCategoryMetadata.map((category) => ({
  ...category,
  optionValues: dbOptions.filter((option) => option.category === category.key).map((option) => option.value),
}));

// Keep the picker exhaustive as database drivers are added or reorganized.
assertCompleteDatabaseCategories(
  dbOptions.map((option) => option.value),
  dbCategoryDefinitions.map((category) => category.optionValues),
);

const hiddenPickerOptionTypes = new Set(Object.keys(MERGED_PICKER_OPTION_FOR_TYPE));

const dbCategories = computed<DbCategory[]>(() => {
  const categories: DbCategory[] = dbCategoryDefinitions.map((category) => ({
    key: category.key,
    title: t(category.titleKey),
    options: dbOptions.filter((option) => category.optionValues.includes(option.value) && !hiddenPickerOptionTypes.has(option.value)),
  }));
  const pluginOptions: DbOption[] = pluginConnectionProviders.value.map((entry) => ({
    value: pluginConnectionProviderOptionValue(entry.plugin.manifest.id, entry.contribution.id),
    label: entry.contribution.label,
    plugin: true,
    pluginId: entry.plugin.manifest.id,
    pluginIcon: pluginConnectionProviderIcon(entry),
  }));
  if (pluginOptions.length) {
    categories.push({ key: "plugins", title: t("connection.databaseCategoryPlugins"), options: pluginOptions });
  }
  return categories;
});

function matchesDbOption(option: DbOption, keyword: string, categoryTitle = "") {
  const profile = driverProfiles[option.value];
  return [option.label, option.value, profile?.label, profile?.type, categoryTitle, ...(PICKER_SEARCH_ALIASES[option.value] ?? [])].some((value) =>
    String(value || "")
      .toLowerCase()
      .includes(keyword),
  );
}

const isDbSearchActive = computed(() => !!dbSearchQuery.value.trim());

const filteredDbCategories = computed<DbCategory[]>(() => {
  const keyword = dbSearchQuery.value.trim().toLowerCase();
  if (!isDbSearchActive.value) return dbCategories.value;

  return dbCategories.value
    .map((category) => ({
      ...category,
      options: category.options.filter((option) => matchesDbOption(option, keyword, category.title)),
    }))
    .filter((category) => category.options.length > 0);
});

const visibleDbCategories = computed<DbCategory[]>(() => {
  if (isDbSearchActive.value) return filteredDbCategories.value;
  return filteredDbCategories.value.filter((category) => category.key === selectedDbCategory.value);
});
const hasDbPickerResults = computed(() => visibleDbCategories.value.some((category) => category.options.length > 0));
function isPickerOptionSelected(optionValue: string): boolean {
  return selectedType.value === optionValue || MERGED_PICKER_OPTION_FOR_TYPE[selectedType.value] === optionValue;
}

const selectedDbOptionIsVisible = computed(() => visibleDbCategories.value.some((category) => category.options.some((option) => isPickerOptionSelected(option.value))));

function selectDbCategory(category: DbCategoryKey) {
  selectedDbCategory.value = category;
  dbSearchQuery.value = "";
  const categoryOptions = dbCategories.value.find((definition) => definition.key === category)?.options.map((option) => option.value) ?? [];
  const nextSelection = databaseSelectionForCategory(selectedType.value, categoryOptions);
  if (nextSelection && nextSelection !== selectedType.value) onDbTypeChange(nextSelection);
}

function selectDbPickerView(view: DbPickerView) {
  dbPickerView.value = view;
  saveConnectionPickerView(view);
}

function dbCategoryForOption(value: string): DbCategoryKey | undefined {
  const pickerValue = MERGED_PICKER_OPTION_FOR_TYPE[value] ?? value;
  return dbCategories.value.find((category) => category.options.some((option) => option.value === pickerValue))?.key;
}

const selectedDbIcon = computed(() => iconTypeMap[selectedType.value] || selectedProfile().icon || selectedType.value);

computed(() => getUrlPlaceholder(form.value.db_type, form.value.driver_profile));

const tlsCapableDatabaseTypes = new Set<DatabaseType>(["mysql"]);
const supportsTlsToggle = computed(() => tlsCapableDatabaseTypes.has(form.value.db_type) || supportsMysqlTlsTab(form.value.db_type, selectedType.value));

computed(() => mysqlTlsOptionsSupported(form.value.db_type, selectedType.value));

// DM8 configures SSL through JDBC URL parameters, so the TLS form and Advanced tab share one source of truth.

// Firebird 的 Java 驱动（Jaybird）默认按 JVM 编码（UTF-8）解码 CHARACTER SET NONE
// 字段，历史数据若以 GBK 等编码存放就会显示为乱码。这里把连接字符集映射到 JDBC URL 的
// charSet 参数（复用通用 URL 参数通道），交给用户按数据实际编码选择。

const canUseTransportLayers = computed(() => {
  {}
  {}
  return true;
});

const canChooseVisibleDatabases = computed(() => connectionCanChooseVisibleDatabases(form.value));
const visibleFilterUsesSchemas = computed(() => connectionUsesVisibleSchemaFilter(form.value));
const hasVisibleDatabaseFilter = computed(() => Array.isArray(form.value.visible_databases));
const visibleDatabaseSummary = computed(() => {
  const configured = form.value.visible_databases;
  if (!Array.isArray(configured)) return t("visibleDatabases.showAll");
  return t("visibleDatabases.selectedCount", { selected: configured.length, total: visibleDatabaseNames.value.length });
});
const defaultListedVisibleDatabaseNames = computed(() => {
  const connection = connectionConfigSnapshotForVisibleDatabases();
  if (visibleFilterUsesSchemas.value) return filterSchemaNamesForVisiblePicker(visibleDatabaseNames.value, connection);
  return filterDatabaseNamesForVisiblePicker(visibleDatabaseNames.value, connection);
});
const listedVisibleDatabaseNames = computed(() => (visibleDatabaseShowSystem.value ? visibleDatabaseNames.value : defaultListedVisibleDatabaseNames.value));
const filteredVisibleDatabaseNames = computed(() => {
  const query = visibleDatabaseSearchText.value.trim().toLowerCase();
  if (!query) return listedVisibleDatabaseNames.value;
  return listedVisibleDatabaseNames.value.filter((name) => name.toLowerCase().includes(query));
});
const visibleDatabaseSelectedCount = computed(() => visibleDatabaseSelection.value.size);
const visibleDatabaseTotalCount = computed(() => listedVisibleDatabaseNames.value.length);
const visibleDatabaseCanSave = computed(() => canSaveVisibleDatabaseSelection([...visibleDatabaseSelection.value]));
const visibleDatabaseHasSystemObjects = computed(() => defaultListedVisibleDatabaseNames.value.length < visibleDatabaseNames.value.length);
const visibleSystemObjectsLabelKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.showSystemSchemas" : "visibleDatabases.showSystemDatabases"));

const filteredProductionDatabaseNames = computed(() => {
  const query = productionDatabaseSearchText.value.trim().toLowerCase();
  if (!query) return productionDatabaseNames.value;
  return productionDatabaseNames.value.filter((name) => name.toLowerCase().includes(query));
});
const productionDatabaseSelectedCount = computed(() => productionDatabaseSelection.value.size);
const productionDatabaseCanSave = computed(() => productionDatabaseNames.value.length > 0 && productionDatabaseSelection.value.size > 0);

const productionPickerTitleKey = computed(() => "production.databasePickerTitle");
const productionPickerDescriptionKey = computed(() => "production.databasePickerDescription");
const productionPickerSearchPlaceholderKey = computed(() => "production.databaseSearchPlaceholder");
const productionPickerSelectionRequiredKey = computed(() => "production.databaseSelectionRequired");
const productionPickerLoadFailedKey = computed(() => "production.databaseLoadFailed");
const productionPickerEmptyKey = computed(() => "production.noDatabasesAvailable");

// MQ/MQTT have no database list — production protection is always connection-scoped.

const visibleSchemasDatabaseKey = computed(() => form.value.database || "");

const visibleSchemaObjectSelection = computed(() => {
  const configured = form.value.visible_schemas?.[visibleSchemasDatabaseKey.value];
  if (Array.isArray(configured)) return configured;
  if (visibleFilterUsesSchemas.value && Array.isArray(form.value.visible_databases)) return form.value.visible_databases;
  return undefined;
});

const hasVisibleObjectFilter = computed(() => (visibleFilterUsesSchemas.value ? Array.isArray(visibleSchemaObjectSelection.value) : hasVisibleDatabaseFilter.value));
const visibleObjectSummary = computed(() => {
  if (!visibleFilterUsesSchemas.value) return visibleDatabaseSummary.value;
  const configured = visibleSchemaObjectSelection.value;
  if (!Array.isArray(configured)) return t("visibleSchemas.showAll");
  return t("visibleSchemas.selectedCount", { selected: configured.length, total: visibleDatabaseNames.value.length });
});
const visibleObjectTitleKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.title" : "visibleDatabases.title"));
const visibleObjectDescriptionKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.description" : "visibleDatabases.description"));
const visibleObjectSearchPlaceholderKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.searchPlaceholder" : "visibleDatabases.searchPlaceholder"));
const visibleObjectSelectedCountKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.selectedCount" : "visibleDatabases.selectedCount"));
const visibleObjectEmptySelectionKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.emptySelection" : "visibleDatabases.emptySelection"));
const visibleObjectLoadFailedKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.loadFailed" : "visibleDatabases.loadFailed"));
const visibleObjectSaveKey = computed(() => (visibleFilterUsesSchemas.value ? "visibleSchemas.save" : "visibleDatabases.save"));

const testResultMessage = computed(() => {
  if (!testResult.value) return "";
  if (!testResult.value.ok) return translateBackendError(t, testResult.value.message);
  return testResult.value.scope === "ssh" ? t("connection.sshTunnelTestSuccess") : t("connection.testSuccess");
});
const pluginActionStatusMessage = computed(() => {
  if (!pluginActionStatus.value) return "";
  return pluginActionStatus.value.ok ? pluginActionStatus.value.message : translateBackendError(t, pluginActionStatus.value.message);
});

const shouldUseWideConnectionDialog = computed(() => dialogStep.value === "config" && (canChooseVisibleDatabases.value || (selectedPluginProvider.value?.contribution.fields.length ?? 0) >= 6));
const connectionDialogContentClass = computed(() => {
  if (dialogStep.value === "select") return "connection-dialog-content--picker sm:h-[720px] sm:max-w-[880px]";
  const widthClass = shouldUseWideConnectionDialog.value ? "connection-dialog-content--wide sm:max-w-[660px]" : "connection-dialog-content--standard sm:max-w-[560px]";

  return `${widthClass} connection-dialog-content--config${""}`;
});

const hasRequiredConnectionTarget = computed(() => {
  {}
  {}
  {}
  {}
  {}
  {}
  {}
  // Cloud Spanner has no host to fall back on: the resource path is the target.
  {}
  {}
  return !!(form.value.host || false || connectionUrlInput.value.trim());
});

function goToConnectionStep(value = selectedType.value) {
  if (value !== selectedType.value) {
    onDbTypeChange(value);
  }
  dialogStep.value = "config";
  configTab.value = "connection";
  dbSearchQuery.value = "";
}

function backToDatabasePicker() {
  const category = dbCategoryForOption(selectedType.value);
  if (category) selectedDbCategory.value = category;
  dialogStep.value = "select";
  resetTestState();
}

function handleDialogEscape(event: KeyboardEvent) {
  if (dialogStep.value !== "config" || editingId.value) return;
  event.preventDefault();
  backToDatabasePicker();
}

watch(customDriverName, (value) => {
  if (isCustomCompatibleProfile()) {
    form.value.driver_label = value.trim() || selectedProfile().label;
  }
});

async function testConnection() {
  if (isTestingSshTunnel.value) return;
  if (!ensureConnectionHostResolvedFromUrl()) return;

  const runId = ++testRunId;
  isTesting.value = true;
  testResult.value = null;
  testResultCopied.value = false;
  let config: ConnectionConfig | null = null;
  const submittedSourceName = form.value.name;
  try {
    config = connectionConfigForSubmit(editingId.value || draftTestConnectionId.value);

    await ensureRequiredJdbcxDriverInstalled(config);

    const result = await testConnectionWithTimeout(config, runId);
    if (runId !== testRunId) return;
    let successfulConfig = config;
    {
    }
    applySuccessfulConnectionTest(result, successfulConfig, submittedSourceName);
    void persistSuccessfulConnectionTest(result, successfulConfig, submittedSourceName, runId);
    clearEditedConnectionErrorAfterSuccessfulTest();
  } catch (e: any) {
    if (runId !== testRunId) return;
    const rawMessage = errorMessage(e);
    const message = config ? connectionErrorWithDriverUpdateHint(config, rawMessage) : rawMessage;

    {
    }
    clearTestedConnectionInfo();
    testResult.value = { ok: false, message };
    showConnectionError(message);
  } finally {
    if (runId === testRunId) {
      isTesting.value = false;
    }
  }
}

function clearEditedConnectionErrorAfterSuccessfulTest() {
  if (editingId.value) store.clearConnectionError(editingId.value);
}

function applyConnectionUrlToForm(input: string): boolean {
  try {
    const update = parseConnectionDeepLinkUpdate(input);
    if (update) {
      if (update.connectionId !== editingId.value || props.editConfig?.one_time) throw new Error("Open the saved connection specified by this update link before applying it.");
      const draft = applyConnectionDeepLinkUpdate(form.value, update);
      applyingConnectionUpdate = true;
      appliedConnectionUpdate = update;
      form.value = draft;
      finishApplyingConnectionUpdate();
      resetTestState();
      appliedConnectionUrlInput.value = input.trim();
      return true;
    }
    const draft = parseConnectionDeepLink(input);
    if (draft) {
      applyConnectionDraftToForm({ ...draft, oneTime: undefined });
      {
      }
      resetTestState();
      appliedConnectionUrlInput.value = input.trim();
      return true;
    }

    const parsed = parseConnectionUrl(input, connectionUrlPreferredProfile());
    form.value = applyParsedConnectionUrl(form.value, parsed);
    {
    }
    {
    }

    {
      selectedType.value = parsed.driverProfile;
    }
    customDriverName.value = isCustomCompatibleProfile() ? parsed.driverLabel : "";

    {
    }
    if (!form.value.name.trim()) {
      form.value.name = parsed.database || parsed.host || parsed.driverLabel;
    }
    resetTestState();
    appliedConnectionUrlInput.value = input.trim();
    return true;
  } catch (e: any) {
    toast(t("connection.parseConnectionUrlFailed", { message: e?.message || String(e) }), 5000);
    return false;
  }
}

function hasPendingConnectionUrlInput(): boolean {
  const url = connectionUrlInput.value.trim();
  return !!url && url !== appliedConnectionUrlInput.value;
}

function ensureConnectionHostResolvedFromUrl(): boolean {
  if (hasPendingConnectionUrlInput() && !applyConnectionUrlToForm(connectionUrlInput.value.trim())) return false;
  {}
  return true;
}

function formValueForSubmit(): Omit<ConnectionConfig, "id"> {
  const url = connectionUrlInput.value.trim();
  if (url && url !== appliedConnectionUrlInput.value) {
    const update = parseConnectionDeepLinkUpdate(url);
    if (update) {
      if (update.connectionId !== editingId.value || props.editConfig?.one_time) throw new Error("Open the saved connection specified by this update link before applying it.");
      return applyConnectionDeepLinkUpdate(form.value, update);
    }
    const draft = parseConnectionDeepLink(url);
    if (draft) {
      return applyConnectionDraftToConfig(form.value, { ...draft, oneTime: undefined });
    }

    return applyParsedConnectionUrl(form.value, parseConnectionUrl(url, connectionUrlPreferredProfile()));
  }

  {}

  return form.value;
}

function generateConnectionName(): string {
  const label = selectedProfile().label;
  const rand = Math.random().toString(36).slice(2, 6);
  return `${label}_${rand}`;
}

function normalizeTransportLayersForSubmit(config: LegacyConnectionConfig) {
  config.transport_layers = (config.transport_layers || []).map(normalizeTransportLayer);
  config.transport_layers = config.transport_layers.map((layer) => {
    if (layer.type !== "ssh") return layer;
    const normalized = normalizeSshTunnel(layer);
    const timeout = Number(normalized.connect_timeout_secs);
    normalized.connect_timeout_secs = Number.isFinite(timeout) && timeout > 0 ? timeout : 5;
    return { type: "ssh", ...normalized };
  });
  validateTransportLayers(config);
}

function connectionConfigForSubmit(id: string, generatedName = "", _validatePluginRequired = true): ConnectionConfig {
  let config: LegacyConnectionConfig;
  {
    config = { ...formValueForSubmit(), id } as LegacyConnectionConfig;
  }
  {}
  config.database_info = undefined;
  config.database = normalizeStoredConnectionDatabase(config.db_type, config.database);
  config.note = config.note?.trim() || undefined;
  {}
  if (!config.name?.trim()) {
    config.name = generatedName.trim() || generateConnectionName();
  }
  {}
  {}
  {}
  {}
  {}
  {}
  normalizeTransportLayersForSubmit(config);
  {}
  {
    {
    }
  }
  {}
  normalizeConnectionTimeouts(config, editGlobalConnectTimeoutSecs.value, editGlobalQueryTimeoutSecs.value);
  {}
  {}
  {
    {
      {
      }
    }
  }
  {}
  normalizeConnectionScope(config);
  {
    {
      {
        {
          {
            {
              {
                {
                  {
                    {
                      {
                        {
                          {
                            // Plugin connections keep `external_config`: the manifest-driven form
                            // fields land there via buildPluginConnectionConfig. Only the built-in
                            // drivers without an external-config payload get wiped here.
                            if (!connectionUpdateExternalConfigPreserved(config)) config.external_config = undefined;
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
  {
    {
    }
  }
  {}
  {}
  {
    config.sysdba = undefined;
  }
  {}
  {}
  {}
  {
    {
      {
        config.client_cert_path = undefined;
        config.client_key_path = undefined;
      }
    }
  }
  {}
  {
    config.ca_cert_path = config.ca_cert_path?.trim() || "";
  }
  {}
  {}
  if (gaussdbConnectionMode(config) === "m-jdbc") {
    {
      {
        {
        }
      }
    }
  } else {
  }
  {}
  {}
  const legacy = config as LegacyConnectionConfig;
  delete legacy.ssh_enabled;
  delete legacy.ssh_host;
  delete legacy.ssh_port;
  delete legacy.ssh_user;
  delete legacy.ssh_password;
  delete legacy.ssh_key_path;
  delete legacy.ssh_key_passphrase;
  delete legacy.ssh_expose_lan;
  delete legacy.ssh_connect_timeout_secs;
  delete legacy.ssh_tunnels;
  delete legacy.proxy_enabled;
  delete legacy.proxy_type;
  delete legacy.proxy_host;
  delete legacy.proxy_port;
  delete legacy.proxy_username;
  delete legacy.proxy_password;
  if (connectionUsesVisibleSchemaFilter(config)) {
    config.visible_databases = undefined;
  } else {
    config.visible_databases = Array.isArray(config.visible_databases) && config.visible_databases.length > 0 ? config.visible_databases : undefined;
  }
  if (!config.show_system_schemas) config.show_system_schemas = undefined;
  if (config.show_database_links !== false) config.show_database_links = undefined;
  if (!config.sidebar_auto_load_all_tables) config.sidebar_auto_load_all_tables = undefined;
  if (config.visible_schemas && Object.keys(config.visible_schemas).length === 0) config.visible_schemas = undefined;
  if (config.agent_java_options && config.agent_java_options.length === 0) {
  }
  // Pasted credentials may carry invisible characters that trim() keeps (#9043).
  sanitizeConnectionCredentials(config);
  return config as ConnectionConfig;
}

function connectionConfigSnapshotForVisibleDatabases(): ConnectionConfig {
  return {
    ...(form.value as ConnectionConfig),
    id: editingId.value || "draft",
    visible_databases: form.value.visible_databases,
  };
}

function resetTestState() {
  testRunId += 1;
  isTesting.value = false;
  isTestingSshTunnel.value = false;
  testResult.value = null;
  pluginActionStatus.value = null;
  clearTestedConnectionInfo();
  showConnectionErrorDialog.value = false;
  connectionErrorRawDetail.value = "";
  connectionErrorDetail.value = "";
  testResultCopied.value = false;
  connectionErrorCopied.value = false;
}

function resetVisibleDatabaseDraftState() {
  showVisibleDatabasesDialog.value = false;
  isLoadingVisibleDatabases.value = false;
  visibleDatabaseNames.value = [];
  visibleDatabaseSelection.value = new Set();
  visibleDatabaseSearchText.value = "";
  visibleDatabaseError.value = "";
  visibleDatabaseShowSystem.value = false;
}

function resetProductionDatabaseDraftState() {
  showProductionDatabasesDialog.value = false;
  isLoadingProductionDatabases.value = false;
  productionDatabaseNames.value = [];
  productionDatabaseSelection.value = new Set();
  productionDatabaseSearchText.value = "";
  productionDatabaseError.value = "";
  productionProtectionEnabled.value = false;
}

/** Silently load database names so the summary count shows a real total. */
async function preloadVisibleDatabaseNames() {
  if (!ensureConnectionHostResolvedFromUrl()) return;
  if (visibleDatabaseNames.value.length > 0) return;
  isLoadingVisibleDatabases.value = true;
  const draftId = buildDraftVisibleDatabasesConnectionId(uuid());
  try {
    const draftConfig = {
      ...connectionConfigForSubmit(draftId),
      id: draftId,
      one_time: true,
    };
    await api.connectDb(draftConfig);
    visibleDatabaseNames.value = await loadVisibleDatabaseNames(draftId, draftConfig);
  } catch {
    // silently fail
  } finally {
    await api.disconnectDb(draftId).catch(() => undefined);
    isLoadingVisibleDatabases.value = false;
  }
}

async function openVisibleDatabasesPicker() {
  if (!ensureConnectionHostResolvedFromUrl()) return;
  if (!canChooseVisibleDatabases.value || isLoadingVisibleDatabases.value) return;

  isLoadingVisibleDatabases.value = true;
  visibleDatabaseError.value = "";
  visibleDatabaseSearchText.value = "";
  const draftId = buildDraftVisibleDatabasesConnectionId(uuid());

  try {
    const draftConfig = {
      ...connectionConfigForSubmit(draftId),
      id: draftId,
      one_time: true,
    };
    await api.connectDb(draftConfig);
    const names = await loadVisibleDatabaseNames(draftId, draftConfig);
    visibleDatabaseNames.value = names;
    visibleDatabaseShowSystem.value = false;
    const configuredSchemas = visibleSchemaObjectSelection.value;
    const initialSelection = visibleFilterUsesSchemas.value ? (Array.isArray(configuredSchemas) ? normalizeVisibleSchemaSelection(configuredSchemas, names) : filterSchemaNamesForVisiblePicker(names, draftConfig)) : initialVisibleDatabaseSelection(names, form.value.visible_databases, draftConfig);
    visibleDatabaseSelection.value = new Set(initialSelection);
    const defaultVisible = new Set(defaultListedVisibleDatabaseNames.value);
    visibleDatabaseShowSystem.value = initialSelection.some((name) => !defaultVisible.has(name));
    showVisibleDatabasesDialog.value = true;
  } catch (e: any) {
    visibleDatabaseNames.value = [];
    visibleDatabaseSelection.value = new Set();
    visibleDatabaseError.value = errorMessage(e);
    testResult.value = { ok: false, message: visibleDatabaseError.value };
    showVisibleDatabasesDialog.value = true;
  } finally {
    await api.disconnectDb(draftId).catch(() => undefined);
    isLoadingVisibleDatabases.value = false;
  }
}

async function loadVisibleDatabaseNames(connectionId: string, config: ConnectionConfig): Promise<string[]> {
  if (connectionUsesVisibleSchemaFilter(config)) {
    return api.listSchemas(connectionId, config.database || "");
  }
  {}
  {}
  return (await api.listDatabases(connectionId)).map((database) => database.name);
}

function normalizeProductionDatabaseSelection(selectedNames: Iterable<string>, databaseNames: string[]): string[] {
  {}
  const available = new Map(databaseNames.map((name) => [name.toLowerCase(), name]));
  const selected = new Set<string>();
  for (const name of selectedNames) {
    const canonicalName = available.get(name.toLowerCase());
    if (canonicalName) selected.add(canonicalName);
  }
  return [...selected];
}

function initialProductionDatabaseSelection(databaseNames: string[]): string[] {
  const configured = form.value.production_databases || [];
  // A new database-level safeguard starts broad; users can explicitly narrow it in the picker.
  return configured.length ? normalizeProductionDatabaseSelection(configured, databaseNames) : databaseNames;
}

async function loadProductionDatabaseNames(connectionId: string, _config: ConnectionConfig): Promise<string[]> {
  {}
  {}
  {}
  return (await api.listDatabases(connectionId)).map((database) => database.name);
}

async function reloadProductionDatabases() {
  if (isLoadingProductionDatabases.value) return;

  isLoadingProductionDatabases.value = true;
  productionDatabaseError.value = "";
  productionDatabaseSearchText.value = "";
  const draftId = `__production_database_draft_${uuid()}`;
  try {
    const draftConfig = {
      ...connectionConfigForSubmit(draftId),
      id: draftId,
      one_time: true,
    };
    await api.connectDb(draftConfig);
    productionDatabaseNames.value = await loadProductionDatabaseNames(draftId, draftConfig);
    productionDatabaseSelection.value = new Set(initialProductionDatabaseSelection(productionDatabaseNames.value));
  } catch (e: any) {
    productionDatabaseNames.value = [];
    productionDatabaseSelection.value = new Set();
    productionDatabaseError.value = errorMessage(e);
  } finally {
    await api.disconnectDb(draftId).catch(() => undefined);
    isLoadingProductionDatabases.value = false;
  }
}

function toggleProductionDatabase(database: string) {
  const next = new Set(productionDatabaseSelection.value);
  if (next.has(database)) next.delete(database);
  else next.add(database);
  productionDatabaseSelection.value = next;
}

function selectAllProductionDatabases() {
  productionDatabaseSelection.value = new Set(productionDatabaseNames.value);
}

function clearProductionDatabaseSelection() {
  productionDatabaseSelection.value = new Set();
}

function saveProductionDatabaseSelection() {
  if (!productionDatabaseCanSave.value) return;
  // A database selection is always narrower than a connection-wide marker.
  productionProtectionEnabled.value = true;
  form.value.is_production = false;
  form.value.production_databases = normalizeProductionDatabaseSelection(productionDatabaseSelection.value, productionDatabaseNames.value);
  showProductionDatabasesDialog.value = false;
}

function toggleVisibleDatabase(database: string) {
  const next = new Set(visibleDatabaseSelection.value);
  if (next.has(database)) next.delete(database);
  else next.add(database);
  visibleDatabaseSelection.value = next;
}

function selectAllVisibleDatabases() {
  visibleDatabaseSelection.value = new Set(listedVisibleDatabaseNames.value);
}

function clearVisibleDatabaseSelection() {
  visibleDatabaseSelection.value = new Set();
}

function showAllVisibleDatabases() {
  if (visibleFilterUsesSchemas.value) {
    handleDraftSchemasShowAll();
    form.value.visible_databases = undefined;
  } else {
    form.value.visible_databases = undefined;
  }
  visibleDatabaseSelection.value = new Set();
  visibleDatabaseNames.value = [];
  showVisibleDatabasesDialog.value = false;
}

function saveVisibleDatabaseSelection() {
  if (!visibleDatabaseCanSave.value) return;
  if (visibleFilterUsesSchemas.value) {
    const key = visibleSchemasDatabaseKey.value;
    form.value.visible_databases = undefined;
    form.value.visible_schemas = {
      ...form.value.visible_schemas,
      [key]: normalizeVisibleSchemaSelection([...visibleDatabaseSelection.value], visibleDatabaseNames.value),
    };
  } else {
    // "全选"等价于不筛选：存成当时的库名快照会让之后新建的库永远看不到。
    const action = resolveVisibleDatabaseSaveAction({
      selection: visibleDatabaseSelection.value,
      allNames: visibleDatabaseNames.value,
      defaultVisibleNames: defaultListedVisibleDatabaseNames.value,
      configured: form.value.visible_databases,
      configuredPatterns: form.value.visible_database_patterns,
      patterns: form.value.visible_database_patterns ?? [],
    });
    if (action.type === "clear") {
      form.value.visible_databases = undefined;
    } else if (action.type === "set") {
      form.value.visible_databases = action.databaseNames;
    }
  }
  showVisibleDatabasesDialog.value = false;
}

function resetVisibleSchemasState() {
  showVisibleSchemasDialog.value = false;
  isLoadingVisibleSchemas.value = false;
  visibleSchemaNames.value = [];
  visibleSchemaInitialSelection.value = [];
  visibleSchemaError.value = "";
}

function handleDraftSchemasSave(selectedNames: string[]) {
  const key = visibleSchemasDatabaseKey.value;
  form.value.visible_schemas = { ...form.value.visible_schemas, [key]: selectedNames };
}

function handleDraftSchemasShowAll() {
  const key = visibleSchemasDatabaseKey.value;
  if (form.value.visible_schemas) {
    const next = { ...form.value.visible_schemas };
    delete next[key];
    form.value.visible_schemas = Object.keys(next).length > 0 ? next : undefined;
  }
}

async function copyTestResult() {
  if (!testResultMessage.value) return;
  try {
    await copyToClipboard(testResultMessage.value);
    testResultCopied.value = true;
    toast(t("grid.copied"));
  } catch (e: any) {
    toast(t("grid.copyFailed", { message: e?.message || String(e) }), 5000);
  }
}

async function copyConnectionErrorDetail() {
  if (!connectionErrorDetail.value) return;
  try {
    await copyToClipboard(connectionErrorDetail.value);
    connectionErrorCopied.value = true;
    toast(t("grid.copied"));
  } catch (e: any) {
    toast(t("grid.copyFailed", { message: e?.message || String(e) }), 5000);
  }
}

function resetForm(options: { preservePickerState?: boolean } = {}) {
  editingId.value = null;
  selectedConnectionGroupId.value = initialConnectionGroupId();
  form.value = defaultForm();

  resetConnectionNoteVisibilityDraft(connectionNoteVisibilityDraft, settingsStore.editorSettings.sidebarShowConnectionNotes);
  editGlobalConnectTimeoutSecs.value = settingsStore.editorSettings.globalConnectTimeoutSecs;
  editGlobalQueryTimeoutSecs.value = settingsStore.editorSettings.globalQueryTimeoutSecs;
  selectedTransportLayerId.value = null;
  draggedTransportLayerId.value = null;
  selectedType.value = "mysql";
  pluginFormValues.value = {};
  customDriverName.value = "";

  oceanbaseSubMode.value = "mysql";

  connectionUrlInput.value = "";
  appliedConnectionUrlInput.value = "";

  if (!options.preservePickerState) {
    dialogStep.value = "select";
    dbSearchQuery.value = "";
    selectedDbCategory.value = "sql";
    configTab.value = "connection";
  }
  resetVisibleDatabaseDraftState();

  resetProductionDatabaseDraftState();
  resetVisibleSchemasState();
  resetTestState();
}

const submittedOneTimePrefillKey = ref<string | null>(null);

function oneTimePrefillKey(draft: ConnectionDeepLinkDraft) {
  return JSON.stringify([draft.name, draft.dbType, draft.driverProfile, draft.driverLabel, draft.host, draft.port, draft.username, draft.password, draft.database, draft.urlParams, draft.ssl]);
}

function submitOneTimePrefill(draft: ConnectionDeepLinkDraft) {
  if (!draft.oneTime) return;
  const key = oneTimePrefillKey(draft);
  if (submittedOneTimePrefillKey.value === key) return;
  submittedOneTimePrefillKey.value = key;
  void nextTick(() => save());
}

function applyConnectionDraftToConfig(config: Omit<ConnectionConfig, "id">, draft: ConnectionDeepLinkDraft): Omit<ConnectionConfig, "id"> {
  const next = {
    ...config,
    db_type: draft.dbType,
    driver_profile: draft.driverProfile,
    driver_label: draft.driverLabel,
    host: draft.host ?? config.host,
    port: draft.port ?? config.port,
    username: draft.username ?? config.username,
    password: draft.password ?? config.password,
    database: draft.database ?? config.database,
    url_params: draft.urlParams ?? config.url_params,
    ssl: draft.ssl ?? config.ssl,
    external_config: config.external_config,

    one_time: draft.oneTime || undefined,
  };

  return next;
}

function applyConnectionDraftToForm(draft: ConnectionDeepLinkDraft) {
  applyProfile(draft.driverProfile);
  form.value = applyConnectionDraftToConfig(form.value, draft);
  {
    {
    }
  }

  selectedType.value = draft.driverProfile;
  {}

  {}
  {}
  customDriverName.value = isCustomCompatibleProfile() ? draft.driverLabel : "";

  if (draft.name?.trim()) {
    form.value.name = draft.name.trim();
  } else if (!form.value.name.trim()) {
    form.value.name = draft.database || draft.host || draft.driverLabel;
  }
  dialogStep.value = "config";
  configTab.value = "connection";
  resetTestState();
}

function applyConnectionPrefill(draft: ConnectionDeepLinkDraft) {
  resetForm();
  applyConnectionDraftToForm(draft);
  submitOneTimePrefill(draft);
}

watch(
  open,
  (value) => {
    if (!value) {
      const draftId = editingId.value ? null : draftTestConnectionId.value;
      submittedOneTimePrefillKey.value = null;
      resetForm();
      if (draftId) {
        void api.disconnectDb(draftId).catch(() => undefined);
        draftTestConnectionId.value = uuid();
      }
      return;
    }
    if (!props.editConfig) {
      resetForm();
      if (props.prefillConfig) applyConnectionPrefill(props.prefillConfig);
    }
    void loadInstalledPlugins().then(() => {
      if (!open.value) return;
      {
        if (!props.prefillConfig) {
          false;
        }
      }
    });
    if (!props.prefillConfig?.oneTime) {
      void loadSshConfigHosts();
    }
    void loadInstalledPlugins().then(() => {
      if (!open.value) return;
      {
        if (!props.prefillConfig) {
          false;
        }
      }
    });
    // Preload database names so the summary count is accurate right away.
    void nextTick(() => {
      // An external update may change the endpoint while retaining its saved
      // password. Do not send credentials until the user tests or saves it.
      if (!props.updatePrefill && canChooseVisibleDatabases.value && hasVisibleDatabaseFilter.value) {
        void preloadVisibleDatabaseNames();
      }
    });
  },
  { immediate: true },
);

watch(connectionGroupOptions, (groups) => {
  if (selectedConnectionGroupId.value && !groups.some((group) => group.id === selectedConnectionGroupId.value)) selectedConnectionGroupId.value = null;
});

watch(
  () => props.prefillConfig,
  (draft) => {
    if (open.value && draft && !props.editConfig) applyConnectionPrefill(draft);
  },
);

watch(
  () => props.pluginProvider,
  (target) => {
    if (!open.value || props.editConfig || props.prefillConfig || !target) return;
    {
      void loadInstalledPlugins().then(() => false);
    }
  },
  { deep: true },
);

watch(
  () => connectionConfigSnapshotForVisibleDatabases(),
  (current, previous) => {
    if (applyingConnectionUpdate || !previous || !visibleObjectFiltersNeedReset(previous, current)) return;
    form.value.visible_databases = undefined;
    form.value.visible_schemas = undefined;
    resetVisibleDatabaseDraftState();
    resetVisibleSchemasState();
  },
);

watch(visibleDatabaseShowSystem, (show) => {
  if (show) return;
  const connection = connectionConfigSnapshotForVisibleDatabases();
  const visible = new Set(visibleFilterUsesSchemas.value ? filterSchemaNamesForVisiblePicker(visibleDatabaseNames.value, connection) : filterDatabaseNamesForVisiblePicker(visibleDatabaseNames.value, connection));
  visibleDatabaseSelection.value = new Set([...visibleDatabaseSelection.value].filter((name) => visible.has(name)));
});

watch(canUseTransportLayers, (value) => {
  if (!value && configTab.value === "transport") {
    configTab.value = "connection";
  }
});

watch(supportsTlsToggle, (value) => {
  if (!value && configTab.value === "tls") {
    configTab.value = "connection";
  }
});

function validateTransportLayers(config: LegacyConnectionConfig) {
  const layers = config.transport_layers || [];
  {}
  layers.forEach((layer, index) => {
    if (layer.enabled === false) return;
    // Profile-referencing layers are stubs: the shared profile supplies the
    // whole configuration at connect time, so there is nothing to validate.
    if (layer.profile_id) return;
    const label = layer.name?.trim() || transportLayerDefaultName(layer, index);
    if (layer.type === "http_tunnel") {
      if (index !== 0) throw new Error(t("connection.httpTunnelInvalidOrder", { hop: label }));
      if (!layer.url?.trim()) throw new Error(t("connection.httpTunnelInvalidUrl", { hop: label }));
      const timeout = Number(layer.connect_timeout_secs);
      if (!Number.isFinite(timeout) || timeout < 1 || timeout > 300) {
        throw new Error(t("connection.httpTunnelInvalidTimeout", { hop: label }));
      }
      return;
    }
    if (!layer.host?.trim()) throw new Error(t("connection.sshHopInvalidHost", { hop: label }));
    const port = Number(layer.port);
    if (!Number.isFinite(port) || port < 1 || port > 65535) {
      throw new Error(t("connection.sshHopInvalidPort", { hop: label }));
    }
    if (layer.type === "ssh") {
      layer.user = layer.user?.trim() || DEFAULT_SSH_USER;
      // Auth credentials are optional: the backend probes "none" authentication
      // first, so hops that require no credential (e.g. passwordless SSH proxies)
      // are valid with password, key, and agent all left empty.
      const timeout = Number(layer.connect_timeout_secs);
      if (!Number.isFinite(timeout) || timeout < 1 || timeout > 300) {
        throw new Error(t("connection.sshHopInvalidTimeout", { hop: label }));
      }
    }
  });
}

async function save(_options: SaveConnectionOptions = {}): Promise<boolean> {
  if (!ensureConnectionHostResolvedFromUrl()) return false;
  if (isSaving.value) return false;
  {
    testResult.value = null;

    return false;
  }
}

const dialogTitle = ref("");
watch([() => editingId.value, () => open.value], () => {
  dialogTitle.value = editingId.value ? t("connection.editTitle") : t("connection.title");
});

async function loadSshConfigHosts() {
  try {
    sshConfigHosts.value = await api.listSshConfigHosts();
  } catch {
    sshConfigHosts.value = [];
  }
}

onMounted(async () => {
  void tunnelProfileStore.init();
});

onUnmounted(() => {
  // Stop any in-flight Salesforce device-code poll / expiry countdown so
  // closing the dialog does not leave orphan timers running.
});
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent
      :style="dialogContentStyle"
      class="connection-dialog-content"
      :class="connectionDialogContentClass"
      :data-wide="shouldUseWideConnectionDialog ? 'true' : undefined"
      @interact-outside.prevent
      @escape-key-down="handleDialogEscape"
      @keydown="preventDialogDocumentSelectAll"
      @copy="copyDialogPasswordFieldValue"
    >
      <DialogHeader class="cursor-move select-none" @pointerdown="onDialogHeaderPointerDown" @pointermove="onDialogHeaderPointerMove" @pointerup="onDialogHeaderPointerEnd" @pointercancel="onDialogHeaderPointerEnd">
        <DialogTitle>{{ editingId ? t("connection.editTitle") : t("connection.title") }}</DialogTitle>
      </DialogHeader>

      <template v-if="dialogStep === 'select'">
        <div class="flex min-h-0 flex-1 flex-col gap-4">
          <div class="connection-db-picker-toolbar flex flex-col gap-3 p-0.5 sm:flex-row sm:items-center sm:justify-between">
            <div class="flex items-center gap-2">
              <div class="flex shrink-0 rounded-lg border bg-muted/40 p-0.5">
                <Button
                  type="button"
                  size="icon-sm"
                  variant="ghost"
                  :class="dbPickerView === 'icon' ? 'bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground' : undefined"
                  :title="t('connection.iconView')"
                  :aria-label="t('connection.iconView')"
                  :aria-pressed="dbPickerView === 'icon'"
                  @click="selectDbPickerView('icon')"
                >
                  <Grid3X3 class="h-3.5 w-3.5" />
                </Button>
                <Button
                  type="button"
                  size="icon-sm"
                  variant="ghost"
                  :class="dbPickerView === 'list' ? 'bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground' : undefined"
                  :title="t('connection.listView')"
                  :aria-label="t('connection.listView')"
                  :aria-pressed="dbPickerView === 'list'"
                  @click="selectDbPickerView('list')"
                >
                  <List class="h-3.5 w-3.5" />
                </Button>
              </div>
              <div class="connection-db-picker-search relative w-full sm:w-64">
                <Search class="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input data-connection-db-search v-model="dbSearchQuery" v-connection-dialog-auto-focus class="h-9 pl-8" :placeholder="t('connection.searchDatabasePlaceholder')" />
              </div>
            </div>
          </div>

          <div class="connection-db-picker-body min-h-0 flex flex-1 flex-col gap-3 overflow-hidden sm:flex-row sm:gap-4">
            <nav data-connection-category-nav class="flex shrink-0 gap-1 overflow-x-auto border-b px-0.5 pt-0.5 pb-2.5 sm:w-40 sm:flex-col sm:overflow-y-auto sm:border-b-0 sm:border-r sm:py-0.5 sm:pr-3.5" :aria-label="t('connection.databaseCategories')">
              <button
                v-for="category in dbCategories"
                :key="category.key"
                type="button"
                class="connection-db-category-option shrink-0 whitespace-nowrap rounded-[4px] px-3 py-2 text-left text-sm transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:w-full"
                :class="!isDbSearchActive && selectedDbCategory === category.key ? 'connection-db-category-option--selected bg-primary/10 font-medium text-primary hover:bg-primary/10' : 'text-muted-foreground hover:bg-muted/70'"
                :aria-current="!isDbSearchActive && selectedDbCategory === category.key ? 'page' : undefined"
                @click="selectDbCategory(category.key)"
              >
                {{ category.title }}
              </button>
            </nav>

            <div class="connection-db-picker-results min-w-0 flex-1 space-y-5 overflow-y-auto p-0.5 pr-2">
              <div v-if="isDbSearchActive" class="text-sm font-medium">{{ t("connection.searchResults") }}</div>

              <section v-for="category in visibleDbCategories" :key="category.key" class="space-y-2">
                <h3 v-if="isDbSearchActive" class="text-sm font-medium">{{ category.title }}</h3>

                <div v-if="dbPickerView === 'icon'" class="connection-db-picker-grid grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-5">
                  <button
                    v-for="opt in category.options"
                    :key="opt.value"
                    type="button"
                    :title="opt.label"
                    class="connection-db-picker-option group flex min-h-24 flex-col items-center justify-center gap-2 rounded-[4px] border bg-background/70 p-3 text-center transition hover:border-primary/40 hover:bg-muted/40 hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    :class="isPickerOptionSelected(opt.value) ? 'dbx-tile-selected shadow-sm' : 'border-border'"
                    :aria-pressed="isPickerOptionSelected(opt.value)"
                    @click="onDbTypeChange(opt.value)"
                    @dblclick="goToConnectionStep(opt.value)"
                  >
                    <span class="flex h-10 w-10 items-center justify-center rounded-xl bg-muted/60 transition group-hover:bg-background">
                      <PluginIcon v-if="opt.plugin" :plugin-id="opt.pluginId || ''" :icon="opt.pluginIcon" class="h-6 w-6" />
                      <DatabaseIcon v-else :db-type="iconTypeMap[opt.value] || opt.value" class="h-6 w-6" />
                    </span>
                    <span class="flex min-h-8 max-w-full items-center justify-center">
                      <span class="line-clamp-2 text-sm leading-4 font-medium">{{ opt.label }}</span>
                    </span>
                  </button>
                </div>

                <div v-else class="grid gap-2">
                  <button
                    v-for="opt in category.options"
                    :key="opt.value"
                    type="button"
                    class="connection-db-picker-option flex items-center gap-3 rounded-[4px] border bg-background px-3 py-2 text-left transition hover:border-primary/40 hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    :class="isPickerOptionSelected(opt.value) ? 'dbx-tile-selected' : 'border-border'"
                    :aria-pressed="isPickerOptionSelected(opt.value)"
                    @click="onDbTypeChange(opt.value)"
                    @dblclick="goToConnectionStep(opt.value)"
                  >
                    <PluginIcon v-if="opt.plugin" :plugin-id="opt.pluginId || ''" :icon="opt.pluginIcon" class="h-5 w-5 shrink-0" />
                    <DatabaseIcon v-else :db-type="iconTypeMap[opt.value] || opt.value" class="h-5 w-5 shrink-0" />
                    <span class="min-w-0 flex-1 truncate text-sm font-medium">{{ opt.label }}</span>
                    <span v-if="isDbSearchActive" class="text-xs text-muted-foreground">{{ category.title }}</span>
                  </button>
                </div>
              </section>

              <div v-if="!hasDbPickerResults" class="rounded-xl border border-dashed py-12 text-center text-sm text-muted-foreground">
                {{ t("connection.noDatabaseMatches") }}
              </div>
            </div>
          </div>
        </div>

        <DialogFooter class="flex shrink-0 items-center gap-2">
          <div class="mr-auto flex min-w-0 items-center gap-2 text-sm text-muted-foreground">
            <DatabaseIcon :db-type="selectedDbIcon" class="h-4 w-4 shrink-0" />
            <span class="truncate">{{ t("connection.selectedDatabase") }}: {{ selectedProfile().label }}</span>
          </div>
          <Button :disabled="!hasDbPickerResults || !selectedDbOptionIsVisible" @click="goToConnectionStep()">
            {{ t("connection.next") }}
            <ChevronRight class="h-4 w-4" />
          </Button>
        </DialogFooter>
      </template>

      <template v-else>
        <DialogFooter class="connection-dialog-footer flex min-w-0 shrink-0 items-center gap-2 sm:flex-nowrap">
          <div class="connection-dialog-test-status mr-auto flex min-w-0 flex-1 basis-0 items-center gap-2 overflow-hidden">
            <Button v-if="!editingId" variant="outline" class="shrink-0" :disabled="isSaving || isTestingSshTunnel" @click="backToDatabasePicker">
              <ArrowLeft class="h-4 w-4" />
              {{ t("connection.back") }}
            </Button>
            <template v-if="pluginActionStatus">
              <span class="block min-w-0 flex-1 basis-0 truncate text-xs" :class="pluginActionStatus.ok ? 'text-green-600' : 'text-red-600'" :title="pluginActionStatusMessage" role="status" aria-live="polite">
                {{ pluginActionStatusMessage }}
              </span>
            </template>
            <template v-else-if="testResult">
              <span class="block min-w-0 flex-1 basis-0 truncate text-xs" :class="testResult.ok ? 'text-green-600' : 'text-red-600'" :title="testResultMessage" role="status" aria-live="polite">
                {{ testResultMessage }}
              </span>
              <Button v-if="!testResult.ok" variant="ghost" size="icon-xs" class="h-5 w-5 shrink-0" :title="testResultCopied ? t('grid.copied') : t('connection.copyTestResult')" :aria-label="testResultCopied ? t('grid.copied') : t('connection.copyTestResult')" @click="copyTestResult">
                <Check v-if="testResultCopied" class="h-3 w-3" />
                <Copy v-else class="h-3 w-3" />
              </Button>
            </template>
          </div>

          <template>
            <Button v-if="canChooseVisibleDatabases" variant="outline" class="shrink-0" :disabled="isTesting || isTestingSshTunnel || isSaving || isLoadingVisibleDatabases || !hasRequiredConnectionTarget" @click="openVisibleDatabasesPicker">
              <Loader2 v-if="isLoadingVisibleDatabases" class="mr-1.5 h-4 w-4 animate-spin" />
              <ListFilter v-else class="mr-1.5 h-4 w-4" />
              {{ hasVisibleObjectFilter ? visibleObjectSummary : visibleFilterUsesSchemas ? t("contextMenu.configureVisibleObjects") : t("contextMenu.selectVisibleDatabases") }}
            </Button>

            <Button variant="outline" class="shrink-0" :disabled="isTesting || isTestingSshTunnel || isSaving" @click="testConnection">
              {{ isTesting ? t("connection.testing") : t("connection.test") }}
            </Button>
            <Button class="shrink-0" @click="save()" :disabled="isSaving || isTestingSshTunnel || !hasRequiredConnectionTarget">
              {{ isSaving ? t("common.loading") : editingId ? t("connection.save") : t("connection.saveAndConnect") }}
            </Button>
          </template>
        </DialogFooter>
      </template>
    </DialogContent>
  </Dialog>

  <Dialog v-model:open="showConnectionErrorDialog">
    <DialogContent class="min-w-0 sm:max-w-[680px]">
      <DialogHeader>
        <DialogTitle>{{ t("connection.connectFailedTitle") }}</DialogTitle>
      </DialogHeader>

      <div class="min-w-0 space-y-2">
        <div class="text-sm text-muted-foreground">{{ t("connection.fullErrorMessage") }}</div>
        <pre class="max-h-72 min-w-0 max-w-full overflow-x-hidden overflow-y-auto whitespace-pre-wrap break-all [overflow-wrap:anywhere] rounded-md border bg-muted/30 p-3 text-xs leading-5 text-destructive">{{ connectionErrorDetail }}</pre>
      </div>

      <DialogFooter class="gap-2">
        <Button variant="outline" @click="copyConnectionErrorDetail">
          <Check v-if="connectionErrorCopied" class="mr-1.5 h-3.5 w-3.5" />
          <Copy v-else class="mr-1.5 h-3.5 w-3.5" />
          {{ connectionErrorCopied ? t("grid.copied") : t("connection.copyError") }}
        </Button>
        <Button @click="showConnectionErrorDialog = false">{{ t("common.close") }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <Dialog v-model:open="showVisibleDatabasesDialog">
    <DialogContent class="sm:max-w-[520px]" @keydown="preventDialogDocumentSelectAll">
      <DialogHeader>
        <DialogTitle>{{ t(visibleObjectTitleKey) }}</DialogTitle>
        <p class="text-sm text-muted-foreground">
          {{ t(visibleObjectDescriptionKey, { connection: form.name || selectedProfile().label }) }}
        </p>
      </DialogHeader>

      <div class="flex items-center gap-2 rounded-md border bg-background px-2">
        <Search class="h-4 w-4 shrink-0 text-muted-foreground" />
        <Input v-model="visibleDatabaseSearchText" :placeholder="t(visibleObjectSearchPlaceholderKey)" class="h-8 border-0 px-0 shadow-none focus-visible:ring-0" :disabled="isLoadingVisibleDatabases || !!visibleDatabaseError" />
      </div>

      <div class="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          {{
            t(visibleObjectSelectedCountKey, {
              selected: visibleDatabaseSelectedCount,
              total: visibleDatabaseTotalCount,
            })
          }}
        </span>
        <div class="flex items-center gap-2">
          <button class="hover:text-foreground disabled:opacity-50" :disabled="isLoadingVisibleDatabases" @click="selectAllVisibleDatabases">
            {{ t("visibleDatabases.selectAll") }}
          </button>
          <button class="hover:text-foreground disabled:opacity-50" :disabled="isLoadingVisibleDatabases" @click="clearVisibleDatabaseSelection">
            {{ t("visibleDatabases.clear") }}
          </button>
          <button class="hover:text-foreground disabled:opacity-50" :disabled="isLoadingVisibleDatabases" @click="showAllVisibleDatabases">
            {{ t("visibleDatabases.showAll") }}
          </button>
        </div>
      </div>
      <p v-if="!isLoadingVisibleDatabases && !visibleDatabaseError && !visibleDatabaseCanSave" class="text-xs text-destructive">
        {{ t(visibleObjectEmptySelectionKey) }}
      </p>

      <label v-if="visibleDatabaseHasSystemObjects" class="flex h-8 items-center gap-2 rounded-md px-1 text-xs text-muted-foreground">
        <input v-model="visibleDatabaseShowSystem" type="checkbox" class="h-3.5 w-3.5 accent-primary" :disabled="isLoadingVisibleDatabases || !!visibleDatabaseError" />
        <span>{{ t(visibleSystemObjectsLabelKey) }}</span>
      </label>

      <div class="h-72 overflow-y-auto rounded-md border bg-background/50 p-1">
        <div v-if="isLoadingVisibleDatabases" class="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
          <Loader2 class="h-4 w-4 animate-spin" />
          {{ t("common.loading") }}
        </div>
        <textarea v-else-if="visibleDatabaseError" class="h-full w-full resize-none overflow-auto border-0 bg-transparent p-3 text-sm leading-5 text-destructive outline-none" :value="t(visibleObjectLoadFailedKey, { message: visibleDatabaseError })" readonly />
        <div v-else-if="!filteredVisibleDatabaseNames.length" class="p-3 text-sm text-muted-foreground">
          {{ t("grid.noSearchResults") }}
        </div>
        <template v-else>
          <button
            v-for="database in filteredVisibleDatabaseNames"
            :key="database"
            type="button"
            class="flex h-8 w-full min-w-0 items-center gap-2 rounded-sm px-2 text-left text-sm hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground focus-visible:outline-none"
            @click="toggleVisibleDatabase(database)"
          >
            <CheckSquare v-if="visibleDatabaseSelection.has(database)" class="h-4 w-4 shrink-0 text-primary" />
            <Square v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span class="truncate">{{ database }}</span>
          </button>
        </template>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="showVisibleDatabasesDialog = false">{{ t("dangerDialog.cancel") }}</Button>
        <Button :disabled="isLoadingVisibleDatabases || !!visibleDatabaseError || !visibleDatabaseCanSave" @click="saveVisibleDatabaseSelection">
          {{ t(visibleObjectSaveKey) }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <Dialog v-model:open="showProductionDatabasesDialog">
    <DialogContent class="sm:max-w-[460px]">
      <DialogHeader>
        <DialogTitle>{{ t(productionPickerTitleKey) }}</DialogTitle>
        <p class="text-sm text-muted-foreground">
          {{ t(productionPickerDescriptionKey, { connection: form.name || selectedProfile().label }) }}
        </p>
      </DialogHeader>

      <div class="flex items-center gap-2 rounded-md border bg-background px-2">
        <Search class="h-4 w-4 shrink-0 text-muted-foreground" />
        <Input v-model="productionDatabaseSearchText" :placeholder="t(productionPickerSearchPlaceholderKey)" class="h-8 border-0 px-0 shadow-none focus-visible:ring-0" :disabled="isLoadingProductionDatabases || !!productionDatabaseError" />
      </div>

      <div class="flex items-center justify-between text-xs text-muted-foreground">
        <span>{{ t("production.databasesSelectedCount", { selected: productionDatabaseSelectedCount, total: productionDatabaseNames.length }) }}</span>
        <div class="flex items-center gap-2">
          <button class="hover:text-foreground disabled:opacity-50" :disabled="isLoadingProductionDatabases || !!productionDatabaseError" @click="selectAllProductionDatabases">
            {{ t("visibleDatabases.selectAll") }}
          </button>
          <button class="hover:text-foreground disabled:opacity-50" :disabled="isLoadingProductionDatabases || !!productionDatabaseError" @click="clearProductionDatabaseSelection">
            {{ t("visibleDatabases.clear") }}
          </button>
        </div>
      </div>
      <p v-if="!isLoadingProductionDatabases && !productionDatabaseError && !productionDatabaseCanSave" class="text-xs text-destructive">
        {{ t(productionPickerSelectionRequiredKey) }}
      </p>

      <div class="h-72 overflow-y-auto rounded-md border bg-background/50 p-1">
        <div v-if="isLoadingProductionDatabases" class="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
          <Loader2 class="h-4 w-4 animate-spin" />
          {{ t("common.loading") }}
        </div>
        <div v-else-if="productionDatabaseError" class="flex h-full flex-col items-start justify-center gap-3 p-3 text-sm text-destructive">
          <p>{{ t(productionPickerLoadFailedKey, { message: productionDatabaseError }) }}</p>
          <Button type="button" variant="outline" size="sm" @click="reloadProductionDatabases">
            <RefreshCw class="mr-1.5 h-3.5 w-3.5" />
            {{ t("production.retry") }}
          </Button>
        </div>
        <div v-else-if="!filteredProductionDatabaseNames.length" class="p-3 text-sm text-muted-foreground">
          {{ productionDatabaseNames.length ? t("grid.noSearchResults") : t(productionPickerEmptyKey) }}
        </div>
        <template v-else>
          <button
            v-for="database in filteredProductionDatabaseNames"
            :key="database"
            type="button"
            class="flex h-8 w-full min-w-0 items-center gap-2 rounded-sm px-2 text-left text-sm hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:text-accent-foreground focus-visible:outline-none"
            @click="toggleProductionDatabase(database)"
          >
            <CheckSquare v-if="productionDatabaseSelection.has(database)" class="h-4 w-4 shrink-0 text-primary" />
            <Square v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
            <span class="truncate">{{ database }}</span>
          </button>
        </template>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="showProductionDatabasesDialog = false">{{ t("dangerDialog.cancel") }}</Button>
        <Button :disabled="isLoadingProductionDatabases || !!productionDatabaseError || !productionDatabaseCanSave" @click="saveProductionDatabaseSelection">
          {{ t("visibleDatabases.save") }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <VisibleSchemasDialog
    v-model:open="showVisibleSchemasDialog"
    draft-mode
    :connection-id="''"
    :connection-name="form.name || selectedProfile().label"
    :database="visibleSchemasDatabaseKey"
    :database-type="form.db_type"
    :username="form.username"
    :draft-schema-names="visibleSchemaNames"
    :draft-initial-selection="visibleSchemaInitialSelection"
    :draft-loading="isLoadingVisibleSchemas"
    :draft-error="visibleSchemaError"
    @draft:save="handleDraftSchemasSave"
    @draft:show-all="handleDraftSchemasShowAll"
  />
</template>

<style>
.connection-dialog-content {
  display: flex;
  flex-direction: column;
  max-height: calc(var(--dbx-viewport-height) - 2rem);
}

.connection-dialog-content--config {
  min-height: 0;
}

.connection-dialog-content--scrollable {
  height: min(720px, calc(var(--dbx-viewport-height) - 2rem));
}

.connection-dialog-content--config .connection-form-body {
  /* Preserve every form section's natural height; the form viewport owns
   * scrolling and must never shrink cards into collapsed grid rows. */
  align-content: start;
}

.connection-form-body--nacos {
  /* Authentication fields are conditional. Keep every Nacos card at its
   * max-content height when they appear, and scroll the form as a whole. */
  grid-auto-rows: max-content;
}

@media (max-height: 720px) {
  .connection-dialog-content--config {
    /* A definite flex height lets tab bodies shrink and scroll above the fixed footer. */
    height: calc(var(--dbx-viewport-height) - 2rem);
  }
}

/* Legacy responsive layout rules live in public/connection-dialog-legacy.css
 * so the production build cannot rewrite their classic media queries. */
html.dbx-legacy-webview .connection-db-category-option--selected {
  color: rgb(23, 23, 23) !important;
  background-color: rgba(23, 23, 23, 0.08) !important;
}

html.dbx-legacy-webview .connection-db-category-option--selected:hover {
  color: rgb(23, 23, 23) !important;
  background-color: rgba(23, 23, 23, 0.12) !important;
}

html.dbx-legacy-webview .connection-transport-layer-option--selected {
  color: rgb(23, 23, 23) !important;
  border-color: rgb(23, 23, 23) !important;
  background-color: rgba(23, 23, 23, 0.08) !important;
}

html.dbx-legacy-webview .connection-transport-layer-option--selected:hover {
  background-color: rgba(23, 23, 23, 0.12) !important;
}

html.dbx-legacy-webview.dark .connection-db-category-option--selected {
  color: rgb(244, 244, 245) !important;
  background-color: rgba(255, 255, 255, 0.1) !important;
}

html.dbx-legacy-webview.dark .connection-db-category-option--selected:hover {
  color: rgb(244, 244, 245) !important;
  background-color: rgba(255, 255, 255, 0.14) !important;
}

html.dbx-legacy-webview.dark .connection-transport-layer-option--selected {
  color: rgb(244, 244, 245) !important;
  border-color: rgb(244, 244, 245) !important;
  background-color: rgba(255, 255, 255, 0.1) !important;
}

html.dbx-legacy-webview.dark .connection-transport-layer-option--selected:hover {
  background-color: rgba(255, 255, 255, 0.14) !important;
}

.connection-db-picker-option {
  color: var(--foreground);
}

.connection-config-step :is([data-slot="input"], [data-slot="select-trigger"], [data-slot="tabs-list"], [data-slot="tabs-trigger"], textarea) {
  border-radius: var(--dbx-radius-fixed-4, 4px);
}

.connection-dialog-content[data-wide="true"] .grid.grid-cols-4 {
  grid-template-columns: minmax(5.5rem, 0.7fr) repeat(3, minmax(0, 1fr));
}

.connection-dialog-content[data-wide="true"] .connection-form-body {
  width: min(100%, 36rem);
  margin-inline: auto;
}
</style>
