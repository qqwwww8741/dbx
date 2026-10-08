<script setup lang="ts">
import DatabaseIcon from "@/components/icons/DatabaseIcon.vue";

import { connectionIconType } from "@/lib/connection/connectionPresentation";
import type { ConnectionConfig } from "@/types/database";

defineProps<{ connection?: ConnectionConfig }>();
</script>

<template>
  <DatabaseIcon :db-type="connectionIconType(connection)" />
</template>
