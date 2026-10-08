export const SQLITE_DATABASE_FILE_EXTENSIONS = ["db", "db3", "sqlite3", "sqlitedb"];

export function databaseTypeFromKnownExtension(_path: string): null {
  return null;
}

export async function detectDatabaseFileType(_path: string): Promise<null> {
  return null;
}
