<script setup lang="ts">
import { useI18n } from "vue-i18n";

import type { DatabaseType } from "@/types/database";

defineProps<{ databaseType?: DatabaseType; disabled?: boolean }>();

useI18n();
</script>

<template></template>
