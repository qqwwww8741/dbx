import type { DatabaseType } from "@/types/database";
import { connectionProfileForScheme, parseConnectionUrl } from "@/lib/connection/connectionUrl";

export interface ConnectionDeepLinkDraft {
  name?: string;
  dbType: DatabaseType;
  driverProfile: string;
  driverLabel: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  database?: string;
  urlParams?: string;
  ssl?: boolean;
  oneTime?: boolean;
}

export interface ConnectionDeepLinkUpdatePatch {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  database?: string;
  urlParams?: string;
  ssl?: boolean;
}

export interface ConnectionDeepLinkUpdate {
  connectionId: string;
  patch: ConnectionDeepLinkUpdatePatch;
}

const CONNECTION_DEEP_LINK_TARGET = "connection/new";
const CONNECTION_UPDATE_PARAMS = new Set(["id", "v", "name", "host", "port", "user", "password", "database", "url_params", "ssl"]);
// Untrusted pages build these URLs; cap them so a deep link cannot prefill
// unbounded strings into a saved connection's fields.
const CONNECTION_UPDATE_MAX_URL_LENGTH = 16384;
const CONNECTION_UPDATE_MAX_PARAM_LENGTH = 4096;

function normalizePath(url: URL): string {
  return [url.hostname, url.pathname.replace(/^\/+/, "")].filter(Boolean).join("/").replace(/\/+$/, "");
}

/** Parse an explicit-field update without applying the defaults used by new connections. */
export function parseConnectionDeepLinkUpdate(value: string): ConnectionDeepLinkUpdate | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "dbx:" || normalizePath(url) !== CONNECTION_DEEP_LINK_TARGET || !url.searchParams.has("id")) return null;
  if (value.length > CONNECTION_UPDATE_MAX_URL_LENGTH) throw new Error("Connection update URL is too long");
  if (url.username || url.password || url.port || url.hash) throw new Error("Invalid connection update URL");

  const params = url.searchParams;
  for (const key of params.keys()) {
    if (!CONNECTION_UPDATE_PARAMS.has(key)) throw new Error("Unsupported connection update parameter");
    if (params.getAll(key).length !== 1) throw new Error("Duplicate connection update parameter");
    if (params.get(key)!.length > CONNECTION_UPDATE_MAX_PARAM_LENGTH) throw new Error(`Connection update ${key} is too long`);
  }
  if (params.has("v") && params.get("v")?.trim() !== "1") throw new Error("Unsupported connection update version");

  const rawConnectionId = params.get("id")!;
  const connectionId = rawConnectionId.trim();
  if (!connectionId || /[\u0000-\u001f\u007f]/.test(rawConnectionId)) throw new Error("A valid connection ID is required for updates");

  const patch: ConnectionDeepLinkUpdatePatch = {};
  for (const key of ["name", "host"] as const) {
    if (!params.has(key)) continue;
    const text = params.get(key)!.trim();
    if (!text) throw new Error(`Connection update ${key} cannot be empty`);
    patch[key] = text;
  }
  if (params.has("port")) {
    const port = params.get("port")!.trim();
    const number = Number(port);
    if (!/^\d+$/.test(port) || !Number.isInteger(number) || number < 1 || number > 65535) throw new Error("Invalid connection update port");
    patch.port = number;
  }
  // Presence matters: an empty value explicitly clears a field, while omission preserves it.
  // Do not trim credentials; spaces can be part of a username or password.
  if (params.has("user")) patch.username = params.get("user")!;
  if (params.has("password")) patch.password = params.get("password")!;
  if (params.has("database")) patch.database = params.get("database")!;
  if (params.has("url_params")) patch.urlParams = params.get("url_params")!;
  if (params.has("ssl")) {
    const ssl = params.get("ssl")!.trim().toLowerCase();
    if (["true", "1", "yes", "on"].includes(ssl)) patch.ssl = true;
    else if (["false", "0", "no", "off"].includes(ssl)) patch.ssl = false;
    else throw new Error("Invalid connection update SSL value");
  }
  return { connectionId, patch };
}

function optionalParam(params: URLSearchParams, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = params.get(key)?.trim();
    if (value) return value;
  }
  return undefined;
}

function optionalPortParam(params: URLSearchParams, ...keys: string[]): number | undefined {
  const value = optionalParam(params, ...keys);
  if (!value) return undefined;
  const numberValue = Number(value);
  if (!Number.isInteger(numberValue) || numberValue < 1 || numberValue > 65535) {
    throw new Error(`Invalid connection port: ${value}`);
  }
  return numberValue;
}

function optionalBooleanParam(params: URLSearchParams, ...keys: string[]): boolean | undefined {
  const value = optionalParam(params, ...keys)?.toLowerCase();
  if (value === undefined) return undefined;
  if (["true", "1", "yes", "on"].includes(value)) return true;
  if (["false", "0", "no", "off"].includes(value)) return false;
  throw new Error(`Invalid boolean value: ${value}`);
}

function validateProtocolVersion(params: URLSearchParams) {
  const version = optionalParam(params, "v");
  if (version !== undefined && version !== "1") {
    throw new Error(`Unsupported connection deep-link version: ${version}`);
  }
}

function draftFromConnectionUrl(value: string, preferredProfile?: string): ConnectionDeepLinkDraft {
  const parsed = parseConnectionUrl(value, preferredProfile);
  return {
    name: parsed.name,
    dbType: parsed.dbType,
    driverProfile: parsed.driverProfile,
    driverLabel: parsed.driverLabel,
    host: parsed.host,
    port: parsed.port,
    username: parsed.username,
    password: parsed.password,
    database: parsed.database,
    urlParams: parsed.urlParams,
    ssl: parsed.ssl,
  };
}

export function parseConnectionDeepLink(value: string): ConnectionDeepLinkDraft | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }

  if (url.protocol !== "dbx:") return null;
  if (normalizePath(url) !== CONNECTION_DEEP_LINK_TARGET) return null;

  const params = url.searchParams;
  if (params.has("id")) throw new Error("Connection update link cannot create a new connection");
  validateProtocolVersion(params);
  const preferredProfile = optionalParam(params, "type")?.toLowerCase();
  const rawConnectionUrl = optionalParam(params, "url");
  const draft: ConnectionDeepLinkDraft = rawConnectionUrl
    ? draftFromConnectionUrl(rawConnectionUrl, preferredProfile)
    : (() => {
        const profile = connectionProfileForScheme(preferredProfile || "mysql");
        if (!profile) throw new Error(`Unsupported connection type: ${preferredProfile}`);
        return {
          dbType: profile.type,
          driverProfile: profile.profile,
          driverLabel: profile.label,
          port: profile.defaultPort,
          ssl: false,
        };
      })();

  const oneTime = optionalBooleanParam(params, "one_time");
  const explicitPort = optionalPortParam(params, "port");
  const explicitSsl = optionalBooleanParam(params, "ssl");
  const host = optionalParam(params, "host") ?? draft.host;
  const port = explicitPort ?? draft.port;
  const username = optionalParam(params, "user") ?? draft.username;
  const password = optionalParam(params, "password") ?? draft.password;
  const ssl = explicitSsl ?? draft.ssl;

  return {
    ...draft,
    name: optionalParam(params, "name") ?? draft.name,
    host,
    port,
    username,
    password,
    database: optionalParam(params, "database") ?? draft.database,
    urlParams: optionalParam(params, "url_params") ?? draft.urlParams,
    ssl,
    ...(oneTime !== undefined ? { oneTime } : {}),
  };
}
