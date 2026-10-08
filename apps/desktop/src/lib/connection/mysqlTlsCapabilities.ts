import type { ConnectionConfig, DatabaseType } from "@/types/database";

export function supportsMysqlTlsOptions(dbType: DatabaseType, _selectedType: string): boolean {
  return dbType === "mysql";
}

export function supportsMysqlTlsTab(dbType: DatabaseType, selectedType: string): boolean {
  return supportsMysqlTlsOptions(dbType, selectedType);
}

type SavedMysqlTlsFields = Pick<ConnectionConfig, "url_params" | "ssl" | "ca_cert_path" | "client_cert_path" | "client_key_path">;

export function savedMysqlTlsFormFields(config: SavedMysqlTlsFields): Required<SavedMysqlTlsFields> {
  return {
    url_params: config.url_params || "",
    ssl: config.ssl || false,
    ca_cert_path: config.ca_cert_path || "",
    client_cert_path: config.client_cert_path || "",
    client_key_path: config.client_key_path || "",
  };
}
