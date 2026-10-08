import { ref } from "vue";
import { useConnectionStore } from "@/stores/connectionStore";
import { isSchemaAware as isSchemaAwareType } from "@/lib/database/databaseCapabilities";
import { sortSidebarNames } from "@/lib/database/databaseTree";
import { filterSchemaNamesForConnection } from "@/lib/database/visibleDatabases";
import type { ConnectionConfig } from "@/types/database";

export function hasSchemaOptionsCacheEntry(options: Record<string, string[]>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(options, key);
}

export function schemaOptionsCacheKey(connectionId: string, database: string, showSystemSchemas: boolean): string {
  return `${connectionId}:${database}:${showSystemSchemas ? "show-system" : "hide-system"}`;
}

export function schemaOptionsForConnection(schemaNames: string[], connection: Pick<ConnectionConfig, "db_type" | "driver_profile" | "visible_databases" | "visible_schemas" | "show_system_schemas"> | undefined, database = ""): string[] {
  // Keep numeric schema suffixes in human order (SCHEMA2 before SCHEMA10), matching the database tree.
  return sortSidebarNames(filterSchemaNamesForConnection(schemaNames, connection, database));
}

export function useSchemaOptions() {
  const connectionStore = useConnectionStore();

  const schemaOptions = ref<Record<string, string[]>>({});
  const loadingSchemaOptions = ref<Record<string, boolean>>({});

  function cacheKey(connectionId: string, database: string) {
    return schemaOptionsCacheKey(connectionId, database, connectionStore.getConfig(connectionId)?.show_system_schemas === true);
  }

  function isSchemaAware(connectionId: string): boolean {
    return isSchemaAwareType(connectionStore.getConfig(connectionId)?.db_type);
  }

  async function loadSchemaOptions(_connectionId: string, _database: string) {
    {
      return;
    }
  }

  function getSchemaOptionsForDb(connectionId: string, database: string): string[] {
    return schemaOptions.value[cacheKey(connectionId, database)] ?? [];
  }

  function isLoadingSchemas(connectionId: string, database: string): boolean {
    return !!loadingSchemaOptions.value[cacheKey(connectionId, database)];
  }

  return {
    schemaOptions,
    loadingSchemaOptions,
    loadSchemaOptions,
    getSchemaOptionsForDb,
    isLoadingSchemas,
    isSchemaAware,
  };
}
