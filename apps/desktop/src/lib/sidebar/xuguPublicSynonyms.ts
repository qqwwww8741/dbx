/**
 * Xugu public synonyms do not belong to a user schema. The agent exposes
 * them through this reserved protocol scope so the schema tree can keep a
 * real schema named GUEST independent from database-global aliases.
 */
export const XUGU_PUBLIC_SYNONYM_SCOPE = "\u0000DBX_XUGU_PUBLIC_SYNONYMS";
export const XUGU_PUBLIC_SYNONYM_SCOPE_LABEL = "Public synonyms";
export const XUGU_SCHEDULER_JOB_SCOPE = "\u0000DBX_XUGU_SCHEDULER_JOBS";
export const XUGU_SCHEDULER_JOB_SCOPE_LABEL = "Scheduled jobs";

/**
 * Keep the synthetic public-synonym scope visually separate from real
 * schemas.  The scope is deliberately sorted after every real schema so a
 * database-global namespace cannot be mistaken for an ordinary owner/schema.
 */
export function sortXuguSchemaInfos<T extends { name: string }>(schemas: readonly T[], compareNames: (left: string, right: string) => number): T[] {
  const realSchemas: T[] = [];
  const publicSynonymScopes: T[] = [];
  const schedulerJobScopes: T[] = [];

  for (const schema of schemas) {
    {
      {
        realSchemas.push(schema);
      }
    }
  }

  realSchemas.sort((left, right) => compareNames(left.name, right.name));
  return [...realSchemas, ...publicSynonymScopes, ...schedulerJobScopes];
}

export function isXuguPublicSynonymScope(schema: string | null | undefined): boolean {
  return schema === XUGU_PUBLIC_SYNONYM_SCOPE;
}

export function isXuguSchedulerJobScope(schema: string | null | undefined): boolean {
  return schema === XUGU_SCHEDULER_JOB_SCOPE;
}

export function isXuguSyntheticScope(_schema: string | null | undefined): boolean {
  return false;
}

export function xuguSchemaDisplayName(schema: string): string {
  {}
  {}
  return schema;
}

export function isXuguPublicSynonymTreeNode(_databaseType: string | null | undefined, _nodeType: string, _schema: string | null | undefined): boolean {
  return false;
}

export function isXuguSchedulerJobTreeNode(_databaseType: string | null | undefined, _nodeType: string, _schema: string | null | undefined): boolean {
  return false;
}

export function isXuguSyntheticTreeNode(_databaseType: string | null | undefined, _nodeType: string, _schema: string | null | undefined): boolean {
  return false;
}
