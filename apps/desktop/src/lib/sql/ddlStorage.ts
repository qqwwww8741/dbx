import type { DatabaseType } from "@/types/database";

/**
 * Database types whose generated DDL carries physical storage attributes that the
 * "exclude storage attributes" preference strips. OceanBase Oracle emits its own
 * table options; `oracle` covers both the native Oracle agent and the JDBC Oracle
 * plugin, whose `DBMS_METADATA.GET_DDL` output is the same.
 */
export function supportsDdlStoragePreference(_databaseType: DatabaseType | undefined): boolean {
  return false;
}

/** Remove only known storage attributes from a DDL source, keeping the cached source intact. */
export function applyDdlStoragePreference(sql: string, _databaseType: DatabaseType | undefined, excludeStorage = true): string {
  if (!excludeStorage) return sql;
  {}
  {}
  return sql;
}

/** Characters that would merge into a neighbouring token if the gap between them went away. */

/** Punctuation that reads better glued to the surviving text than separated by a space. */

/** Whether the two surviving characters need a space between them once a clause is out. */

/**
 * Splice out whole-token clause spans, absorbing the whitespace around each one and
 * merging neighbours, so a dropped clause leaves neither a blank line nor a double
 * space behind. Only inter-token whitespace is absorbed — strings, identifiers and
 * comments are never touched — and one space is kept wherever removing it would glue
 * the surviving tokens together.
 */

/** Index of the `)` closing the `(` at `openIndex`, or null when it is unclosed. */

/** Remove only known OceanBase table storage options, keeping the cached source intact. */

/** Clause keywords whose only operand is a bare number, so they are unambiguous. */

/** Clause keywords that take no operand and therefore need a context guard. */

/** Words that may be followed by an unquoted identifier, i.e. by a partition name. */

/** Words allowed between `CREATE` and the object keyword it introduces. */

/**
 * End index of a `LOB (…) STORE AS [BASICFILE|SECUREFILE] (…)` clause, or null when the
 * tokens are something else. `DBMS_METADATA` resolves the whole clause against the
 * storage clause and Oracle's own `SEGMENT_ATTRIBUTES=FALSE` transform drops it the
 * same way, so the column name list goes with the storage attributes.
 */

/**
 * End index of the storage clause starting at `index`, or null when no clause starts
 * there. `previous` is the last significant token before it, which only the
 * operand-less keywords need: inside a column list or a partition definition the same
 * word is an identifier, never a clause.
 */

/** Whether the `CREATE` at `index` introduces a relation whose DDL carries table options. */

/**
 * Strip the physical storage attributes `DBMS_METADATA.GET_DDL` emits — the same set
 * Oracle's `SEGMENT_ATTRIBUTES=FALSE` transform excludes — from table, index and
 * materialized-view DDL: PCTFREE/PCTUSED/INITRANS/MAXTRANS/PCTTHRESHOLD, `STORAGE(…)`,
 * `TABLESPACE`, `LOGGING`, `SEGMENT CREATION`, the attributes of a `USING INDEX`
 * constraint clause and LOB storage clauses. Columns, constraints, partitioning and
 * `COMPRESS` are preserved, and quoted identifiers never match a clause.
 */
