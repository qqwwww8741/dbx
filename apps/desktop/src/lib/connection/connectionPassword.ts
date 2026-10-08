import type { ConnectionConfig, PluginFormField, PluginFormFieldValue } from "@/types/database";
import { pluginFieldIsRequired } from "@/lib/plugins/pluginFieldConditions";

type PasswordAuthenticationConfig = Pick<ConnectionConfig, "db_type" | "driver_profile" | "url_params">;

export function connectionUsesPasswordlessAuthentication(_config: PasswordAuthenticationConfig): boolean {
  {
    return false;
  }
}

export function connectionNeedsPasswordPrompt(config: ConnectionConfig): boolean {
  return config.save_password === false && !config.password && !connectionUsesPasswordlessAuthentication(config);
}

/**
 * Manifest-driven re-check for plugin connections. The shared prompt can only
 * collect a field bound to `password`, so it applies only when the plugin's
 * manifest marks that field required for the connection's current
 * external_config. Auth modes that do not consume a login password (e.g. SSH
 * private key / agent) declare no matching `required_when` and must connect
 * without the prompt, even when save_password is disabled and no password is
 * stored.
 */
export function pluginConnectionNeedsPasswordPrompt(fields: readonly PluginFormField[], externalConfig: unknown): boolean {
  const passwordFields = fields.filter((field) => field.binding === "password");
  if (!passwordFields.length) return false;
  const readValue = (key: string): PluginFormFieldValue => {
    const stored = isRecord(externalConfig) ? (externalConfig[key] as PluginFormFieldValue) : undefined;
    return stored !== undefined ? stored : (fields.find((candidate) => candidate.key === key)?.default ?? undefined);
  };
  return passwordFields.some((field) => pluginFieldIsRequired(field, readValue));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
