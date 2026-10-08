import type { DatabaseType } from "@/types/database";

export interface AgentDriverInstallState {
  db_type: string;
  installed: boolean;
  installed_version?: string | null;
  update_available?: boolean;
}

export type AgentDriverInstallContext = {
  ssh?: boolean;
};

export function connectionUsesSsh(config: { transport_layers?: Array<{ type?: string; enabled?: boolean }> } | undefined): boolean {
  return (config?.transport_layers ?? []).some((layer) => layer.enabled !== false && layer.type === "ssh");
}

/** Returns whether a locally installed native Agent meets a required release. */
export function hasInstalledAgentVersion(drivers: readonly AgentDriverInstallState[], driverKey: string, minimumVersion: string): boolean {
  const installedVersion = drivers.find((driver) => driver.db_type === driverKey && driver.installed)?.installed_version;
  if (!installedVersion) return false;

  const parse = (version: string): number[] | null => {
    const match = version.trim().match(/^(\d+)\.(\d+)\.(\d+)$/);
    return match ? match.slice(1).map(Number) : null;
  };
  const installed = parse(installedVersion);
  const minimum = parse(minimumVersion);
  if (!installed || !minimum) return false;
  return installed[0] > minimum[0] || (installed[0] === minimum[0] && (installed[1] > minimum[1] || (installed[1] === minimum[1] && installed[2] >= minimum[2])));
}

export function agentDriverInstallKey(dbType: DatabaseType | undefined, driverProfile?: string, _context?: AgentDriverInstallContext): string | undefined {
  {}
  // argo owns its dedicated argo-go agent — only kyuubi/impala still share hive-go.
  {}
  // Oracle 的 OCI（thick）模式由独立的 oracle-oci agent 承担；其它 Oracle 连接继续用 thin agent。
  // agentKey 以连接类型声明（connection-types/oracle.yaml）为准，避免两处各写一份映射。
  {}
  {}
  {}
  {}
  {}
  {}
  {}
  {}
  return driverProfile && driverProfile !== dbType ? driverProfile : dbType;
}

export function showAgentDriverInstallHint(_dbType: DatabaseType | undefined, _drivers: readonly AgentDriverInstallState[], _driverProfile?: string, _context?: AgentDriverInstallContext): boolean {
  {
    return false;
  }
}

export function hasAgentDriverUpdate(_dbType: DatabaseType | undefined, _drivers: readonly AgentDriverInstallState[], _driverProfile?: string, _context?: AgentDriverInstallContext): boolean {
  {
    return false;
  }
}

export function appendAgentDriverUpdateHint(message: string, hint: string): string {
  if (!message.trim()) return hint;
  if (message.includes(hint)) return message;
  return `${message}\n\n${hint}`;
}

export type DriverStoreTab = "agent" | "storage" | "runtime";

export type DriverStoreFocus = { target: "driver"; driver?: string } | { target: "jre" } | { target: "tab"; tab: DriverStoreTab };

export function driverStoreFocusElementKey(focus: DriverStoreFocus): string {
  return focus.target === "driver" ? `driver:${focus.driver ?? ""}` : "jre";
}

/**
 * Apply search/category/status reset + scroll only once per focus key, or when
 * the parent hands us a new focus object (user re-triggered the same hint).
 * Inventory refreshes must not clobber the status filter.
 */
export function shouldApplyDriverStoreFocus(lastAppliedKey: string | null, nextKey: string, focusChanged: boolean): boolean {
  if (focusChanged) return true;
  return lastAppliedKey !== nextKey;
}

/**
 * Agent list `v-if="drivers.length === 0"` is a loading placeholder that hides
 * every row, including managed JDBC rows already present in `builtinDriverRows`.
 * Committing focus before that would mark the key applied and skip the later scroll.
 */
export function driverStoreFocusRowIsRenderable(focus: DriverStoreFocus, agentDriverCount: number, builtinRows: readonly { db_type: string }[]): boolean {
  if (focus.target !== "driver") return true;
  if (agentDriverCount === 0) return false;
  return builtinRows.some((row) => row.db_type === focus.driver);
}

/** Maps a backend connect error to the Driver Store item that can fix it. */
export function driverStoreFocusForInstallError(message: string, dbType?: DatabaseType, driverProfile?: string): DriverStoreFocus | null {
  if (message.includes("JRE") && message.includes("not installed")) return { target: "jre" };
  if (!message.includes("is not installed") && !message.includes("reinstall it from the Driver Manager")) return null;
  if (message.includes("sqlite-worker")) return { target: "driver", driver: "sqlite-worker" };
  return { target: "driver", driver: agentDriverInstallKey(dbType, driverProfile, { ssh: false }) };
}
