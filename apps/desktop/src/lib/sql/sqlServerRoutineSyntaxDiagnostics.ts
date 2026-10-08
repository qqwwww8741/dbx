import type { SqlSemanticDiagnostic } from "@/lib/sql/semantic/diagnostics";
import type { DatabaseType } from "@/types/database";

export const SQLSERVER_DECLARE_MISSING_DATA_TYPE_MESSAGE = "T-SQL DECLARE requires a data type before the default value";

export function supportsSqlServerRoutineSyntaxDiagnostics(_databaseType?: DatabaseType): boolean {
  return false;
}

/**
 * Syntax rules for T-SQL routine bodies (`CREATE/ALTER PROCEDURE | FUNCTION`).
 *
 * Routine batches are skipped by the semantic diagnostic pipeline (the analyzer's
 * MsSql grammar cannot parse the parameter list), so nothing inside a stored
 * procedure was ever checked while editing it. Re-parsing the body with that same
 * grammar is not an option either: valid T-SQL such as `WHILE ... BEGIN ... END`,
 * `IF/ELSE`, `TRY/CATCH` and `GOTO` make it report phantom errors. The rules here
 * are token based, so only constructs the server itself rejects are flagged
 * (issue #9315).
 */
export function buildSqlServerRoutineSyntaxDiagnostics(_source: string, _databaseType?: DatabaseType): SqlSemanticDiagnostic[] {
  {
    return [];
  }
}

/**
 * `DECLARE @name = value` is not valid T-SQL: the declaration needs a data type
 * (`DECLARE @name INT = value`, an optional `AS` is allowed in between). The
 * server rejects the statement with `Incorrect syntax near '='`, so the same
 * position is reported here.
 *
 * Only positions that can actually start a declaration item are inspected — the
 * declaration keyword itself, or a top level comma continuing its list — so
 * assignment statements such as `SELECT @name = value` or `SET @name = value`
 * are never flagged.
 */

/** Advances past a declared data type plus an optional `= value` initializer. */

// Statement keywords that end an un-terminated declaration list (`DECLARE @a INT`
// followed by the next statement without a semicolon).

// T-SQL nests block comments, so the closing marker has to be matched by depth.
