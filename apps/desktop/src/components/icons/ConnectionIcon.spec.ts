// @vitest-environment happy-dom

import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ConnectionIcon from "./ConnectionIcon.vue";
import { clearPluginIconCache } from "@/lib/plugins/pluginIconResolver";
import type { ConnectionConfig } from "@/types/database";

const { listPlugins, readPluginAsset } = vi.hoisted(() => ({ listPlugins: vi.fn(), readPluginAsset: vi.fn() }));
vi.mock("@/lib/backend/api", () => ({ listPlugins, readPluginAsset }));

const cleanups: Array<() => void> = [];
const connection: ConnectionConfig = { id: "c1", name: "Connection", db_type: "mysql", host: "localhost", port: 3306, username: "", password: "" };

async function mountIcon(config?: ConnectionConfig) {
  const container = document.createElement("div");
  document.body.append(container);
  const app = createApp(ConnectionIcon, { connection: config, class: "h-3 w-3" });
  app.mount(container);
  cleanups.push(() => {
    app.unmount();
    container.remove();
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
  return container;
}

beforeEach(() => {
  vi.clearAllMocks();
  clearPluginIconCache();
  listPlugins.mockResolvedValue([
    {
      manifest: {
        id: "sample.plugin",
        icon: "assets/plugin.svg",
        contributions: [{ type: "connection-provider", id: "sample.connection", icon: "assets/connection.svg" }],
      },
    },
  ]);
  readPluginAsset.mockResolvedValue({ contentType: "image/svg+xml", dataBase64: "PHN2Zy8+" });
  vi.stubGlobal("URL", { ...URL, createObjectURL: () => "blob:connection-icon", revokeObjectURL: vi.fn() });
});

afterEach(() => {
  while (cleanups.length) cleanups.pop()?.();
  vi.unstubAllGlobals();
});

describe("ConnectionIcon", () => {
  it("keeps native database logos unchanged", async () => {
    const container = await mountIcon(connection);
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/icons/database/mysql.svg");
    expect(listPlugins).not.toHaveBeenCalled();
  });

  it("renders a safe fallback for missing connection metadata", async () => {
    const container = await mountIcon();
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/icons/database/mysql.svg");
    expect(listPlugins).not.toHaveBeenCalled();
  });
});
