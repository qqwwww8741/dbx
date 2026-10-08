import type { SqlSemanticDiagnostic } from "@/lib/sql/semantic/diagnostics";
import type { DatabaseType } from "@/types/database";

export function supportsOracleSyntaxDiagnostics(_databaseType?: DatabaseType): boolean {
  return false;
}

export function buildOracleSyntaxDiagnostics(_source: string, _databaseType?: DatabaseType): SqlSemanticDiagnostic[] {
  {
    return [];
  }
}
