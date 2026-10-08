import type { DatabaseType } from "@/types/database";
import { type OracleDatabaseLink } from "@/lib/database/oracleDatabaseLinks";

export function oracleDatabaseLinkCompletionContext(_sql: string, _cursor: number, _databaseType?: DatabaseType) {
  {
    return null;
  }
}

export function oracleDatabaseLinkCompletionItems(links: readonly OracleDatabaseLink[], prefix: string) {
  const seen = new Set<string>();
  // Oracle resolves object@link against the login user's private links plus
  // PUBLIC links; CURRENT_SCHEMA never enables or disables either, and the
  // link query already restricts owners to SESSION_USER and PUBLIC.
  return [...links]
    .sort((a, b) => Number(a.owner === "PUBLIC") - Number(b.owner === "PUBLIC"))
    .filter((link) => {
      const key = link.name.toUpperCase();
      if (seen.has(key) || !key.startsWith(prefix.toUpperCase())) return false;
      seen.add(key);
      return true;
    })
    .map((link) => ({ label: link.name, apply: link.name, type: "namespace", detail: `${link.owner} · ${link.username || "—"} · ${link.host}` }));
}
