<script setup lang="ts">
import { onMounted } from "vue";
import type { DriverStoreFocus, DriverStoreTab } from "@/lib/connection/agentDriverInstallHint";
defineProps<{ updateNotificationsEnabled?: boolean; activeTab?: DriverStoreTab; focusTarget?: DriverStoreFocus | null }>();
const emit = defineEmits<{ "update-count-change": [count: number]; "update:activeTab": [tab: DriverStoreTab] }>();
onMounted(() => emit("update-count-change", 0));
</script>
<template>
  <div class="p-6 space-y-3">
    <h2 class="text-lg font-semibold">MySQL</h2>
    <p class="text-sm text-muted-foreground">MySQL 驱动已内置，无需额外安装。 / The MySQL driver is included.</p>
  </div>
</template>
