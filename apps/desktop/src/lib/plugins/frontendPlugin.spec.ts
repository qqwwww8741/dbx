import { describe, expect, it } from "vitest";
import type { InstalledPlugin, PluginConnectionProviderContribution } from "@/types/database";
import { createFrontendPluginRegistry, initialPluginFormValues, parsePluginConnectionProviderOptionValue, pluginConnectionActionsForDialog, pluginConnectionProviderIcon, pluginConnectionProviderOptionValue } from "./frontendPlugin";

function installedPlugin(id: string, contributions: InstalledPlugin["manifest"]["contributions"] = []): InstalledPlugin {
  return {
    compatibility: { compatible: true },
    manifest: {
      id,
      name: id,
      version: "1.0.0",
      drivers: [],
      contributions,
    },
  };
}

function connectionProvider(overrides: Partial<PluginConnectionProviderContribution> = {}): PluginConnectionProviderContribution {
  return {
    type: "connection-provider",
    id: "connection",
    label: "Example",
    database_type: "example",
    fields: [
      { key: "host", label: "Host", type: "text", required: true },
      { key: "port", label: "Port", type: "number", default: 1234 },
      { key: "tls", label: "TLS", type: "boolean", default: false },
    ],
    ...overrides,
  };
}

describe("FrontendPluginRegistry", () => {
  it("round-trips plugin connection provider picker values", () => {
    const value = pluginConnectionProviderOptionValue("example/plugin", "ssh:main");
    expect(parsePluginConnectionProviderOptionValue(value)).toEqual({ pluginId: "example/plugin", providerId: "ssh:main" });
    expect(parsePluginConnectionProviderOptionValue("mysql")).toBeNull();
  });

  it("indexes declarative connection providers", () => {
    const registry = createFrontendPluginRegistry([installedPlugin("com.example.plugin", [connectionProvider()])]);

    expect(registry.listConnectionProviders()).toHaveLength(1);
    expect(registry.listConnectionProviders()[0]?.contribution.database_type).toBe("example");
  });

  it("indexes workbench, connection provider, and filesystem contributions", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        {
          type: "connection-provider",
          id: "example.connection",
          label: "Example",
          database_type: "example",
          fields: [],
          workbench: "example.main",
          filesystem_provider: "example.files",
        },
        { type: "workbench", id: "example.main", label: "Example Workbench" },
        { type: "filesystem-provider", id: "example.files", label: "Example Files", schemes: ["example"], root_uri: "example:/home", capabilities: ["read", "write"] },
      ]),
    ]);

    expect(registry.listConnectionProviders()).toHaveLength(1);
    expect(registry.listWorkbenches()[0]?.contribution.id).toBe("example.main");
    expect(registry.listFilesystemProviders()[0]?.contribution.schemes).toEqual(["example"]);
    expect(registry.listFilesystemProviders()[0]?.contribution.root_uri).toBe("example:/home");
    expect(registry.listConnectionProviders()[0]?.contribution.filesystem_provider).toBe("example.files");
  });

  it("indexes result-view contributions", () => {
    const registry = createFrontendPluginRegistry([installedPlugin("com.example.plugin", [{ type: "result-view", id: "example.graph", label: "Graph" }])]);

    const views = registry.listResultViews();
    expect(views).toHaveLength(1);
    expect(views[0]?.contribution.id).toBe("example.graph");
    expect(views[0]?.contribution.label).toBe("Graph");
  });

  it("resolves the plugin UI contribution behind a workbench or result-view tab", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        { type: "workbench", id: "example.main", label: "Example Workbench", icon: "assets/main.svg" },
        { type: "result-view", id: "example.graph", label: "Graph", icon: "assets/graph.svg" },
      ]),
    ]);

    const workbench = registry.findUiContribution("com.example.plugin", "example.main");
    expect(workbench?.contribution).toMatchObject({ type: "workbench", id: "example.main", label: "Example Workbench", icon: "assets/main.svg" });

    // The result-view keeps its own id and display metadata: the plugin UI is
    // told which declared contribution the user opened, and it is not a workbench.
    const resultView = registry.findUiContribution("com.example.plugin", "example.graph");
    expect(resultView?.contribution).toMatchObject({ type: "result-view", id: "example.graph", label: "Graph", icon: "assets/graph.svg" });
  });

  it("keeps workbench lookups scoped to workbench contributions", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        { type: "workbench", id: "example.main", label: "Example Workbench" },
        { type: "result-view", id: "example.graph", label: "Graph" },
      ]),
    ]);

    // `findWorkbench` answers `host.openWorkbench` and `connection-provider.workbench`:
    // a result-view id must never satisfy it.
    expect(registry.findWorkbench("com.example.plugin", "example.graph")).toBeUndefined();
    expect(registry.findWorkbench("com.example.plugin", "example.main")?.contribution.id).toBe("example.main");
  });

  it("does not resolve unknown, foreign, or non-UI contributions as plugin UI", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        { type: "result-view", id: "example.graph", label: "Graph" },
        { type: "context-menu", id: "example.inspect", label: "Inspect", menu: "connection" },
        { type: "filesystem-provider", id: "example.files", label: "Files", schemes: ["example"] },
      ]),
      installedPlugin("com.example.other", []),
    ]);

    expect(registry.findUiContribution("com.example.plugin", "example.graph")?.contribution.type).toBe("result-view");
    expect(registry.findUiContribution("com.example.plugin", "example.missing")).toBeUndefined();
    expect(registry.findUiContribution("com.example.other", "example.graph")).toBeUndefined();
    // Context menus render natively and filesystem providers own their tab mode.
    expect(registry.findUiContribution("com.example.plugin", "example.inspect")).toBeUndefined();
    expect(registry.findUiContribution("com.example.plugin", "example.files")).toBeUndefined();
  });

  it("does not resolve plugin UI contributions of incompatible plugins", () => {
    const plugin = installedPlugin("com.example.plugin", [
      { type: "workbench", id: "example.main", label: "Example Workbench" },
      { type: "result-view", id: "example.graph", label: "Graph" },
    ]);
    plugin.compatibility = { compatible: false, errors: ["Unsupported host API"] };

    const registry = createFrontendPluginRegistry([plugin]);

    expect(registry.findUiContribution("com.example.plugin", "example.main")).toBeUndefined();
    expect(registry.findUiContribution("com.example.plugin", "example.graph")).toBeUndefined();
  });

  it("keeps declarative context-menu actions as context-menu metadata, not commands", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        { type: "workbench", id: "example.main", label: "Example Workbench" },
        {
          type: "context-menu",
          id: "example.open",
          label: "Open Example",
          menu: "connection",
          action: { type: "open-workbench", workbench: "example.main" },
        },
      ]),
    ]);

    const item = registry.listContextMenuItems("connection")[0];
    expect(item?.contribution.action).toEqual({ type: "open-workbench", workbench: "example.main" });
    expect(registry.findWorkbench("com.example.plugin", "example.main")?.contribution.id).toBe("example.main");
    expect(registry.findCommand("com.example.plugin", "example.open")).toBeUndefined();
    expect(registry.listCommands()).toHaveLength(0);
  });

  it("indexes context-menu contributions per menu surface", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.plugin", [
        { type: "context-menu", id: "example.inspect", label: "Inspect endpoint", menu: "connection" },
        { type: "context-menu", id: "example.inspect-table", label: "Inspect table", menu: "table" },
      ]),
    ]);

    const connectionItems = registry.listContextMenuItems("connection");
    expect(connectionItems).toHaveLength(1);
    expect(connectionItems[0]?.contribution.id).toBe("example.inspect");
    expect(connectionItems[0]?.contribution.action).toBeUndefined();
    expect(connectionItems[0]?.plugin.manifest.id).toBe("com.example.plugin");

    const tableItems = registry.listContextMenuItems("table");
    expect(tableItems).toHaveLength(1);
    expect(tableItems[0]?.contribution.id).toBe("example.inspect-table");

    const emptyRegistry = createFrontendPluginRegistry([installedPlugin("com.example.empty")]);
    expect(emptyRegistry.listContextMenuItems("table")).toHaveLength(0);
    expect(registry.listContextMenuItems("missing")).toHaveLength(0);
  });

  it("prefers provider display metadata and falls back to plugin metadata", () => {
    const explicit = installedPlugin("com.example.explicit", [
      {
        type: "connection-provider",
        id: "explicit.connection",
        label: "Explicit connection",
        icon: "assets/provider.svg",
        database_type: "explicit",
        fields: [],
      },
    ]);
    explicit.manifest.name = "Explicit plugin";
    explicit.manifest.icon = "assets/plugin.svg";
    const fallback = installedPlugin("com.example.fallback", [
      {
        type: "connection-provider",
        id: "fallback.connection",
        label: "",
        database_type: "fallback",
        fields: [],
      },
    ]);
    fallback.manifest.name = "Fallback plugin";
    fallback.manifest.icon = "assets/fallback.svg";

    const registry = createFrontendPluginRegistry([explicit, fallback]);
    const [explicitEntry, fallbackEntry] = registry.listConnectionProviders();

    expect(explicitEntry?.contribution.label).toBe("Explicit connection");
    expect(pluginConnectionProviderIcon(explicitEntry!)).toBe("assets/provider.svg");
    expect(fallbackEntry?.contribution.label).toBe("Fallback plugin");
    expect(pluginConnectionProviderIcon(fallbackEntry!)).toBe("assets/fallback.svg");
  });

  it("falls back to provider id and the generic icon when metadata is absent", () => {
    const plugin = installedPlugin("com.example.minimal", [
      {
        type: "connection-provider",
        id: "minimal.connection",
        label: "",
        database_type: "minimal",
        fields: [],
      },
    ]);
    plugin.manifest.name = "";

    const entry = createFrontendPluginRegistry([plugin]).listConnectionProviders()[0]!;

    expect(entry.contribution.label).toBe("minimal.connection");
    expect(pluginConnectionProviderIcon(entry)).toBeUndefined();
  });

  it("drops unsafe icon paths before rendering", () => {
    const plugin = installedPlugin("com.example.unsafe", [
      {
        type: "connection-provider",
        id: "unsafe.connection",
        label: "Unsafe",
        icon: "../outside.svg",
        database_type: "unsafe",
        fields: [],
      },
      // Every contribution the host renders through the plugin UI entrypoint
      // resolves its icon asset path the same way.
      { type: "workbench", id: "unsafe.main", label: "Unsafe workbench", icon: "../outside.svg" },
      { type: "result-view", id: "unsafe.graph", label: "Unsafe graph", icon: "/outside.svg" },
    ]);
    plugin.manifest.icon = "\\outside.svg";

    const definition = createFrontendPluginRegistry([plugin]).listPlugins()[0]!;
    const entry = createFrontendPluginRegistry([plugin]).listConnectionProviders()[0]!;

    expect(definition.plugin.manifest.icon).toBeUndefined();
    expect(pluginConnectionProviderIcon(entry)).toBeUndefined();
    expect(createFrontendPluginRegistry([plugin]).listWorkbenches()[0]?.contribution.icon).toBeUndefined();
    expect(createFrontendPluginRegistry([plugin]).listResultViews()[0]?.contribution.icon).toBeUndefined();
  });

  it("localizes plugin metadata, contributions, and form fields", () => {
    const plugin = installedPlugin("com.example.plugin", [connectionProvider({ id: "example.connection", description: "English description" })]);
    plugin.manifest.localizations = {
      "zh-CN": {
        name: "示例插件",
        description: "插件说明",
        contributions: {
          "example.connection": {
            label: "示例连接",
            description: "连接说明",
            fields: {
              host: { label: "主机", placeholder: "请输入主机" },
            },
          },
        },
      },
    };

    const registry = createFrontendPluginRegistry([plugin], "zh-CN");
    const definition = registry.listPlugins()[0]!;
    const contribution = registry.listConnectionProviders()[0]!.contribution;

    expect(definition.plugin.manifest.name).toBe("示例插件");
    expect(definition.plugin.manifest.description).toBe("插件说明");
    expect(contribution.label).toBe("示例连接");
    expect(contribution.description).toBe("连接说明");
    expect(contribution.fields[0]).toMatchObject({ label: "主机", placeholder: "请输入主机" });
    expect(plugin.manifest.name).toBe("com.example.plugin");
  });

  it("normalizes connection actions in manifest order and localizes their labels", () => {
    const plugin = installedPlugin("com.example.actions", [
      {
        type: "connection-provider",
        id: "example.connection",
        label: "Example",
        database_type: "example",
        fields: [],
        capabilities: ["test"],
        actions: [
          {
            id: "discover",
            label: "Discover",
            description: "Discover an endpoint",
            variant: "outline",
            requires_valid_form: false,
          },
          { id: "edit", label: "Edit", variant: "secondary", when: "edit" },
        ],
      },
    ]);
    plugin.manifest.localizations = {
      "zh-CN": {
        contributions: {
          "example.connection": {
            actions: {
              discover: { label: "发现地址", description: "自动发现连接地址" },
            },
          },
        },
      },
    };

    const actions = createFrontendPluginRegistry([plugin], "zh-CN").listConnectionProviders()[0]?.contribution.actions;

    expect(actions?.map((action) => action.id)).toEqual(["discover", "edit"]);
    expect(actions?.[0]).toMatchObject({
      label: "发现地址",
      description: "自动发现连接地址",
      requires_valid_form: false,
    });
    expect(actions?.[1]).toMatchObject({ label: "Edit", variant: "secondary", when: "edit" });
  });

  it("treats omitted and explicitly empty custom action lists equivalently", () => {
    const registry = createFrontendPluginRegistry([
      installedPlugin("com.example.defaults", [{ type: "connection-provider", id: "default.connection", label: "Default", database_type: "default", fields: [] }]),
      installedPlugin("com.example.empty", [{ type: "connection-provider", id: "empty.connection", label: "Empty", database_type: "empty", fields: [], actions: [] }]),
    ]);

    const [defaults, empty] = registry.listConnectionProviders();
    expect(defaults?.contribution.actions).toBeUndefined();
    expect(empty?.contribution.actions).toEqual([]);
    expect(pluginConnectionActionsForDialog(defaults!.contribution, false)).toEqual(pluginConnectionActionsForDialog(empty!.contribution, false));
  });

  it("builds compatible default footer actions and filters create/edit actions", () => {
    const provider: PluginConnectionProviderContribution = {
      type: "connection-provider",
      id: "example.connection",
      label: "Example",
      database_type: "example",
      fields: [],
      capabilities: ["test"],
    };

    expect(pluginConnectionActionsForDialog(provider, false).map((action) => action.kind)).toEqual(["test", "save-and-connect"]);
    expect(pluginConnectionActionsForDialog(provider, true).map((action) => action.kind)).toEqual(["test", "save"]);
    expect(
      pluginConnectionActionsForDialog(
        {
          ...provider,
          actions: [
            { id: "create", label: "Create", when: "create" },
            { id: "edit", label: "Edit", when: "edit" },
            { id: "always", label: "Always", when: "always" },
          ],
        },
        true,
      ).map((action) => action.id),
    ).toEqual(["edit", "always", "test", "save"]);
  });

  it("creates initial values only from declared defaults", () => {
    const values = initialPluginFormValues(connectionProvider());

    expect(values).toEqual({ port: 1234, tls: false });
    expect(values).not.toHaveProperty("host");
  });
});
