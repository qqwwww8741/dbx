import type { UpdateInfo } from "@/lib/backend/api";
import type { AgentDriverInfo, McpServerStatus } from "@/lib/backend/tauri";
import type { JdbcPluginStatus } from "@/types/database";
import type { MarketplacePluginListing } from "@/lib/plugins/pluginMarketplace";

export function isUpdatePreviewMockEnabled(): boolean {
  return import.meta.env.DEV && import.meta.env.VITE_DBX_MOCK_UPDATES === "1";
}

export function previewAppUpdateInfo(currentVersion: string): UpdateInfo {
  const current = currentVersion || "0.6.15";
  const showAppUpdate = import.meta.env.VITE_DBX_MOCK_UPDATE_SCENARIO !== "components";
  return {
    current_version: current,
    latest_version: showAppUpdate ? "0.6.16" : current,
    update_available: showAppUpdate,
    portable_mode: false,
    manual_update_only: false,
    release_name: showAppUpdate ? "DBX v0.6.16 Preview" : `DBX v${current}`,
    release_url: "https://github.com/t8y2/dbx/releases",
    release_notes: showAppUpdate
      ? `## 更新预览

### 新功能
- 更新中心集中展示 DBX、驱动、JDBC、MCP 与插件更新
- 支持按更新类别查看版本信息和更新内容

### 修复
- 优化自动更新开关默认值
- 修复独立入口红点与聚合更新状态不一致的问题

### 体验优化
- 更新检查在后台静默执行
- 组件更新仅在无阻塞任务时安装`
      : "",
  };
}

export function previewDriverUpdates(): AgentDriverInfo[] {
  return [];
}

export function previewJdbcUpdate(): JdbcPluginStatus {
  return {
    installed: true,
    version: "0.1.0",
    protocol_version: 1,
    compatible: true,
    latest_version: "0.2.0",
    latest_protocol_version: 2,
    update_available: true,
    path: "/preview/jdbc-agent",
  };
}

export function previewMcpUpdate(): McpServerStatus {
  return {
    installed: true,
    installation_source: "npm",
    npm_available: true,
    npm_installed: true,
    node_path: "/preview/bin/node",
    node_version: "v22.18.0",
    current_version: "0.4.88",
    latest_version: "0.4.89",
    update_available: true,
    bin_path: "/preview/bin/dbx-mcp",
    native_bin_path: null,
    script_path: "/preview/lib/dbx-mcp/index.js",
    data_dir: "/preview/data/mcp",
    install_command: "npm install -g @dbx-app/mcp-server",
    update_command: "npm update -g @dbx-app/mcp-server",
    uninstall_command: "npm uninstall -g @dbx-app/mcp-server",
    error: null,
  };
}

export function previewPluginUpdates(): MarketplacePluginListing[] {
  return [
    {
      key: "official:io.dbx.ssh",
      repository: { id: "official", name: "DBX Official", kind: "official", enabled: true, managed: true },
      plugin: {
        id: "io.dbx.ssh",
        name: "SSH Tunnel",
        description: "通过 SSH 隧道安全访问数据库。",
        publisher: "DBX",
        verified: true,
        tags: ["ssh", "tunnel"],
        permissions: ["network"],
        latestVersion: "0.4.78",
        versions: [],
      },
      name: "SSH Tunnel",
      description: "通过 SSH 隧道安全访问数据库。",
      target: "universal",
      artifact: { target: "universal", url: "https://example.invalid/ssh.dbxp", sha256: "preview", signingKeyId: "preview" },
      installed: { manifest: { id: "io.dbx.ssh", name: "SSH Tunnel", version: "0.4.77", drivers: [] }, compatibility: { compatible: true } },
      verified: true,
      status: "update",
    },
    {
      key: "official:io.dbx.data-tools",
      repository: { id: "official", name: "DBX Official", kind: "official", enabled: true, managed: true },
      plugin: {
        id: "io.dbx.data-tools",
        name: "Data Tools",
        description: "数据生成、转换和校验工具集。",
        publisher: "DBX",
        verified: true,
        tags: ["data", "tools"],
        permissions: ["database"],
        latestVersion: "3.3.0",
        versions: [],
      },
      name: "Data Tools",
      description: "数据生成、转换和校验工具集。",
      target: "universal",
      artifact: { target: "universal", url: "https://example.invalid/data-tools.dbxp", sha256: "preview", signingKeyId: "preview" },
      installed: { manifest: { id: "io.dbx.data-tools", name: "Data Tools", version: "3.2.0", drivers: [] }, compatibility: { compatible: true } },
      verified: true,
      status: "update",
    },
  ];
}
