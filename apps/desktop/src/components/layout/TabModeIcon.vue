<script setup lang="ts">
import { Activity, AlertTriangle, Braces, Clock, Code2, Database, Eye, FileCode, Gauge, Link2, ListTree, Package, PencilRuler, ScrollText, Search, Table, TableProperties, UsersRound, Zap } from "@lucide/vue";
import DatabaseIcon from "@/components/icons/DatabaseIcon.vue";
import PluginIcon from "@/components/plugins/PluginIcon.vue";
import { isEventObjectBrowserTab, tabDatabaseIconType } from "@/lib/tabs/tabPresentation";
import type { QueryTab } from "@/types/database";

// Mirrors EditorGroupTabBar's per-mode tab icon chain so surfaces outside the
// group bar (e.g. the special-page strip's return tabs) render identical icons.
defineProps<{ tab: QueryTab }>();
</script>

<template>
  <AlertTriangle v-if="tab.externalSqlFileMissing" />
  <Eye v-else-if="tab.objectSource?.objectType === 'VIEW' || tab.objectSource?.objectType === 'MATERIALIZED_VIEW' || tab.tableMeta?.tableType?.toUpperCase() === 'VIEW' || tab.tableMeta?.tableType?.toUpperCase() === 'MATERIALIZED_VIEW'" />

  <Table v-else-if="tab.mode === 'data'" />

  <Gauge v-else-if="tab.mode === 'mysql-dashboard'" />

  <Database v-else-if="tab.mode === 'databases'" />
  <Clock v-else-if="isEventObjectBrowserTab(tab)" />
  <TableProperties v-else-if="tab.mode === 'objects'" />
  <UsersRound v-else-if="tab.mode === 'users'" />
  <PencilRuler v-else-if="tab.mode === 'structure'" />
  <Search v-else-if="tab.mode === 'database-search'" />
  <ScrollText v-else-if="tab.objectSource?.objectType === 'PROCEDURE'" />
  <Braces v-else-if="tab.objectSource?.objectType === 'FUNCTION'" />
  <Zap v-else-if="tab.objectSource?.objectType === 'TRIGGER'" />
  <Clock v-else-if="tab.objectSource?.objectType === 'EVENT' || tab.objectSource?.objectType === 'JOB'" />
  <ListTree v-else-if="tab.objectSource?.objectType === 'SEQUENCE'" />
  <Link2 v-else-if="tab.objectSource?.objectType === 'SYNONYM'" />
  <Package v-else-if="tab.objectSource?.objectType === 'PACKAGE'" />
  <FileCode v-else-if="tab.objectSource?.objectType === 'PACKAGE_BODY'" />
  <Braces v-else-if="tab.objectSource?.objectType === 'TYPE'" />
  <FileCode v-else-if="tab.objectSource?.objectType === 'TYPE_BODY'" />

  <Activity v-else-if="tab.mode === 'processlist'" />

  <PluginIcon v-else-if="tab.mode === 'plugin-filesystem' && tab.pluginFilesystem" :plugin-id="tab.pluginFilesystem.pluginId" :contribution-id="tab.pluginFilesystem.providerId" />
  <DatabaseIcon v-else-if="tab.mode === 'query'" :db-type="tabDatabaseIconType(tab)" />
  <Code2 v-else />
</template>
