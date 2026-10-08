import { isSecretConnectionProperty } from "@/lib/connection/jdbcProperties";

/** Shared by copyable connection strings and sidebar previews. */
export function redactConnectionStringSecrets(value: string): string {
  const withoutUserInfo = value.replace(/(:\/\/[^/\s:@?#;]*):([^@\s/?#;]+)@/g, "$1:***@");
  return withoutUserInfo.replace(/([?&;])([^=?&;]+)=([^&;]*)/g, (part, separator: string, key: string) => (isSecretConnectionProperty(key) ? `${separator}${key}=***` : part));
}
