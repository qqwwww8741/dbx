import type { ConnectionConfig, TransportLayerConfig, TunnelProfile } from "@/types/database";

export const CONNECTION_ATTEMPT_TIMEOUT_BUFFER_MS = 2_000;
export const MONGO_LEGACY_FALLBACK_TIMEOUT_BUFFER_MS = 30_000;
export const MONGO_OIDC_BROWSER_AUTH_TIMEOUT_MS = 5 * 60_000;
export const AGENT_DRIVER_MIN_CONNECT_TIMEOUT_SECS = 30;
export const ACCESS_AGENT_MIN_CONNECT_TIMEOUT_SECS = 30;
const DEFAULT_CONNECT_TIMEOUT_SECS = 10;

function positiveSeconds(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

export type TunnelProfileResolver = (profileId: string) => TunnelProfile | undefined;

function resolvedTimeoutLayer(layer: TransportLayerConfig, resolveTunnelProfile?: TunnelProfileResolver): TransportLayerConfig {
  if (!layer.profile_id || !resolveTunnelProfile) return layer;
  const profile = resolveTunnelProfile(layer.profile_id);
  // The backend rejects missing or mismatched profiles; retain the stub here so
  // the UI deadline never masks that lifecycle error with invented settings.
  if (!profile || profile.type !== layer.type) return layer;
  return { ...profile, id: layer.id, enabled: layer.enabled, profile_id: layer.profile_id } as TransportLayerConfig;
}

export function connectionAttemptTimeoutMs(config: Pick<ConnectionConfig, "connect_timeout_secs" | "transport_layers"> & Partial<Pick<ConnectionConfig, "db_type" | "url_params" | "connection_string">>, resolveTunnelProfile?: TunnelProfileResolver): number {
  const baseTimeoutSecs = positiveSeconds(config.connect_timeout_secs, DEFAULT_CONNECT_TIMEOUT_SECS);

  let timeoutSecs = baseTimeoutSecs;
  let hasEnabledTransportLayer = false;
  for (const unresolvedLayer of config.transport_layers ?? []) {
    const layer = resolvedTimeoutLayer(unresolvedLayer, resolveTunnelProfile);
    if (layer.enabled === false) continue;
    hasEnabledTransportLayer = true;
    if (layer.type === "ssh" || layer.type === "http_tunnel") {
      timeoutSecs += positiveSeconds(layer.connect_timeout_secs, DEFAULT_CONNECT_TIMEOUT_SECS);
    }
  }
  // Transport setup, the tunneled endpoint probe, and the database connection
  // happen sequentially in the backend. Keep the UI guard outside their total
  // budget so it cannot cancel a connection attempt that is still progressing.
  if (hasEnabledTransportLayer) timeoutSecs += baseTimeoutSecs;

  const fallbackBuffer = 0;
  const browserAuthBuffer = 0;
  return Math.ceil(timeoutSecs * 1000 + CONNECTION_ATTEMPT_TIMEOUT_BUFFER_MS + fallbackBuffer + browserAuthBuffer);
}

export function connectionAttemptTimeoutMessage(timeoutMs: number): string {
  return `Connection attempt timed out after ${Math.ceil(timeoutMs / 1000)}s. Please check the network or VPN and try again.`;
}

export function connectionAttemptOriginalErrorMessage(timeoutMessage: string, originalMessage: string): string {
  const message = originalMessage.trim();
  if (!message || message === timeoutMessage) return timeoutMessage;
  return `${timeoutMessage}\n\nOriginal database error returned after the UI timeout:\n${message}`;
}
