import type { ConnectionConfig } from "@/types/database";
import { redactConnectionStringSecrets } from "@/lib/connection/connectionStringRedaction";
export type ConnectionUrlCopyFormat = "url" | "urlWithPassword" | "jdbcUrl" | "jdbcUrlWithCredentials" | "hostPort";
export const CONNECTION_URL_COPY_WITH_PASSWORD_FORMATS: ReadonlySet<ConnectionUrlCopyFormat> = new Set(["urlWithPassword", "jdbcUrlWithCredentials"]);
export type ConnectionUrlCopyConfig = Pick<ConnectionConfig, "db_type" | "host" | "port" | "username" | "password" | "database" | "url_params" | "ssl">;
export interface ConnectionUrlCopyOptions {
  database?: string;
}
export function connectionSupportsUrlCopy(config: ConnectionUrlCopyConfig | undefined): boolean {
  return config?.db_type === "mysql" && !!config.host?.trim();
}
export function connectionUrlCopyFormats(config: ConnectionUrlCopyConfig | undefined): ConnectionUrlCopyFormat[] {
  if (!config || !connectionSupportsUrlCopy(config)) return [];
  const hasSecret = !!config.password || redactConnectionStringSecrets(`mysql://host?${config.url_params ?? ""}`) !== `mysql://host?${config.url_params ?? ""}`;
  return hasSecret ? ["url", "urlWithPassword", "jdbcUrl", "jdbcUrlWithCredentials", "hostPort"] : ["url", "jdbcUrl", "hostPort"];
}
function encode(value: string): string {
  return encodeURIComponent(value).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
}
export function buildConnectionUrlCopy(config: ConnectionUrlCopyConfig | undefined, format: ConnectionUrlCopyFormat, options?: ConnectionUrlCopyOptions): string | null {
  if (!config || !connectionSupportsUrlCopy(config)) return null;
  const host = config.host.trim();
  const endpoint = `${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${config.port || 3306}`;
  if (format === "hostPort") return endpoint;
  const params = new URLSearchParams(config.url_params?.replace(/^\?/, ""));
  for (const key of [...params.keys()]) if (/^(?:user|username|password|passwd|pwd)$/i.test(key)) params.delete(key);
  if (config.ssl && !params.has("ssl-mode") && !params.has("sslmode") && !params.has("require_ssl") && !params.has("useSSL")) params.set("ssl-mode", "REQUIRED");
  const database = options?.database ?? config.database ?? "";
  if (format === "jdbcUrl" || format === "jdbcUrlWithCredentials") {
    if (format === "jdbcUrlWithCredentials") {
      params.set("user", config.username);
      params.set("password", config.password);
    }
    const query = params.toString();
    const result = `jdbc:mysql://${endpoint}/${encode(database)}${query ? `?${query}` : ""}`;
    return format === "jdbcUrl" ? redactConnectionStringSecrets(result) : result;
  }
  const user = encode(config.username);
  const credentials = format === "urlWithPassword" ? `${user}:${encode(config.password)}` : user;
  const query = params.toString();
  const result = `mysql://${credentials ? `${credentials}@` : ""}${endpoint}/${encode(database)}${query ? `?${query}` : ""}`;
  return format === "url" ? redactConnectionStringSecrets(result) : result;
}
