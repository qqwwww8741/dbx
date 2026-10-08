import type { ConnectionConfig, JdbcDriverInfo, JdbcMavenBundleInfo } from "@/types/database";

export const JDBCX_DRIVER_PROFILE = "jdbcx";
export const JDBCX_JDBC_DRIVER_CLASS = "io.github.jdbcx.WrappedDriver";
export const JDBCX_DEFAULT_URL = "jdbcx:";
export const JDBCX_HIGH_PRIVILEGE_EXTENSIONS_JAVA_OPTION = "-Ddbx.jdbcx.allowHighPrivilegeExtensions=true";
const JDBCX_HIGH_PRIVILEGE_EXTENSIONS_JAVA_OPTION_PREFIX = "-Ddbx.jdbcx.allowHighPrivilegeExtensions=";

export type JdbcxRuntimeDriverApi = {
  listJdbcDrivers: () => Promise<JdbcDriverInfo[]>;
  listJdbcMavenBundles: () => Promise<JdbcMavenBundleInfo[]>;
  jdbcPluginStatus: () => Promise<{ installed: boolean; compatible: boolean }>;
  installJdbcPlugin: () => Promise<unknown>;
};

export type JdbcxRuntimeDriverResult = {
  bundles: JdbcMavenBundleInfo[];
  paths: string[];
  runtimeSelectionId: string;
};

export function isJdbcxRuntimePath(path: string): boolean {
  return /(?:^|[/\\])jdbcx-driver(?:-|\.)/i.test(path);
}

export function isJdbcxRuntimeBundle(bundle: JdbcMavenBundleInfo): boolean {
  const [groupId, artifactId] = bundle.coordinate.split(":");
  return groupId === "io.github.jdbcx" && artifactId === "jdbcx-driver";
}

export function jdbcxHighPrivilegeExtensionsEnabled(config: Pick<ConnectionConfig, "agent_java_options">): boolean {
  const option = [...(config.agent_java_options ?? [])]
    .reverse()
    .map((value) => value.trim())
    .find((value) => value.startsWith(JDBCX_HIGH_PRIVILEGE_EXTENSIONS_JAVA_OPTION_PREFIX));
  return option?.slice(option.indexOf("=") + 1).toLowerCase() === "true";
}

export function setJdbcxHighPrivilegeExtensionsEnabled(config: Pick<ConnectionConfig, "agent_java_options">, enabled: boolean): void {
  // Canonicalize this DBX-owned option so legacy whitespace cannot diverge from backend parsing.
  const options = (config.agent_java_options ?? []).filter((option) => !option.trim().startsWith(JDBCX_HIGH_PRIVILEGE_EXTENSIONS_JAVA_OPTION_PREFIX));
  config.agent_java_options = enabled ? [...options, JDBCX_HIGH_PRIVILEGE_EXTENSIONS_JAVA_OPTION] : options;
}

export async function ensureJdbcxRuntimeDrivers(_config: ConnectionConfig, _api: JdbcxRuntimeDriverApi, _onInstalling?: (coordinates: string[]) => void): Promise<JdbcxRuntimeDriverResult | undefined> {
  {
    return undefined;
  }
}
