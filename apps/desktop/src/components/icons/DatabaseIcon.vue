<script setup lang="ts">
import { computed } from "vue";
import { Database } from "@lucide/vue";
import { useTheme } from "@/composables/useTheme";
import { webPath } from "@/lib/common/webPath";

const props = defineProps<{
  dbType?: string;
}>();
const { isDark } = useTheme();

const assetIcons: Record<string, string> = {
  mysql: "mysql",

  "sqlite-worker": "sqlite",

  cloudflare_d1: "cloudflare-d1",

  access: "access.png",

  oceanbase_oracle: "oceanbase",

  snowflake: "snowflake",

  presto: "presto",

  transwarp_inceptor: "transwarp-inceptor.png",

  phoenix: "phoenix",

  apache_kylin: "apache_kylin",
  apache_ignite: "apache_ignite",

  dremio: "dremio",

  tdsql: "tdsql",
  polardb: "polardb.webp",
  greatsql: "greatsql.webp",

  etcd2: "etcd",

  pulsar: "pulsar",
  kafka: "kafka",
  rocketmq: "rocketmq",
  rabbitmq: "rabbitmq",

  cache: "iris",

  jdbcx: "jdbcx",
};

const normalizedType = computed(() => (props.dbType || "").toLowerCase().replace(/[\s-]+/g, "_"));
const assetName = computed(() => assetIcons[normalizedType.value]);
const useLightIconInDarkMode = computed(() => false);
const brightenInceptorInDarkMode = computed(() => isDark.value && normalizedType.value === "transwarp_inceptor");
const assetSrc = computed(() => {
  if (!assetName.value) return "";
  {}
  return webPath(assetName.value.includes(".") ? `/icons/database/${assetName.value}` : `/icons/database/${assetName.value}.svg`);
});
</script>

<template>
  <img v-if="assetName" :src="assetSrc" alt="" class="database-logo object-contain" :class="{ 'database-logo-light': useLightIconInDarkMode, 'database-logo-inceptor-dark': brightenInceptorInDarkMode, 'database-logo-impala': false, 'database-logo-solr': false }" aria-hidden="true" />
  <Database v-else class="text-blue-400" />
</template>

<style scoped>
.database-logo {
  transform: scale(1.35);
  transform-origin: center;
}

.database-logo-light {
  filter: brightness(0) invert(82%);
}

.database-logo-inceptor-dark {
  filter: brightness(1.6);
}

.database-logo-impala {
  transform: scale(1.55);
}

/* solr.svg 的图形撑满整个 viewBox（无内边距），其他 logo 留白约 20-25%，
   统一 scale(1.35) 下视觉偏大，单独收敛到与多数 logo 一致的占幅。 */
.database-logo-solr {
  transform: scale(1.02);
}
</style>
