import type { ConnectionConfig, DatabaseType } from "@/types/database";
export interface ParsedConnectionUrl {
  name?: string;
  dbType: DatabaseType;
  driverProfile: string;
  driverLabel: string;
  host: string;
  port: number;
  username: string;
  password: string;
  database?: string;
  urlParams: string;
  ssl: boolean;
}
export type ConnectionProfile = { type: DatabaseType; profile: string; label: string; defaultPort: number };
const MYSQL_PROFILE: ConnectionProfile = { type: "mysql", profile: "mysql", label: "MySQL", defaultPort: 3306 };
export function connectionProfileForScheme(scheme: string, _preferredProfile?: string): ConnectionProfile | undefined {
  return scheme.toLowerCase() === "mysql" ? MYSQL_PROFILE : undefined;
}
export function parseConnectionUrl(value: string, _preferredProfile?: string): ParsedConnectionUrl {
  const source = value.trim();
  if (!source) throw new Error("Connection URL is empty");
  let url: URL;
  try {
    url = new URL(source.replace(/^jdbc:(?=mysql:\/\/)/i, ""));
  } catch {
    throw new Error("Invalid connection URL");
  }
  if (url.protocol.toLowerCase() !== "mysql:") throw new Error(`Unsupported connection URL scheme: ${url.protocol.replace(/:$/, "")}`);
  if (!url.hostname) throw new Error("MySQL host is required");
  const port = url.port ? Number(url.port) : 3306;
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Invalid MySQL port");
  const params = new URLSearchParams(url.search);
  const name = params.get("name") ?? undefined;
  params.delete("name");
  const username = decodeURIComponent(url.username) || params.get("user") || "";
  const password = decodeURIComponent(url.password) || params.get("password") || "";
  params.delete("user");
  params.delete("password");
  const tls = (params.get("ssl-mode") || params.get("sslmode") || "").toLowerCase();
  const ssl = ["required", "verify_ca", "verify_identity", "verify-ca", "verify-identity"].includes(tls) || params.get("require_ssl") === "true" || params.get("useSSL") === "true";
  return { ...(name ? { name } : {}), dbType: "mysql", driverProfile: "mysql", driverLabel: "MySQL", host: url.hostname.replace(/^\[|\]$/g, ""), port, username, password, database: decodeURIComponent(url.pathname.replace(/^\//, "")) || undefined, urlParams: params.toString(), ssl };
}
export function applyParsedConnectionUrl(config: Omit<ConnectionConfig, "id">, parsed: ParsedConnectionUrl): Omit<ConnectionConfig, "id"> {
  const keepCredentials = !parsed.username && !parsed.password;
  return {
    ...config,
    ...(parsed.name ? { name: parsed.name } : {}),
    db_type: "mysql",
    driver_profile: "mysql",
    driver_label: "MySQL",
    host: parsed.host,
    port: parsed.port,
    username: keepCredentials ? config.username : parsed.username,
    password: keepCredentials ? config.password : parsed.password,
    database: parsed.database,
    url_params: parsed.urlParams,
    ssl: parsed.ssl,
  };
}
