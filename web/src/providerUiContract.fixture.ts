/** Hermetic authoring fixture shared by compile, reducer and browser tests.
 * Not imported by the application. No Provider runtime/authentication data.
 */
import { defineProviderUiContract } from "@cowboy/provider-authoring";
import type { ProviderUiManifest, UiAsset } from "@cowboy/provider-ui";

function asset<const I extends UiAsset["role"]>(id: I) {
  return {
    id,
    role: id,
    media_type: "image/svg+xml",
    digest: `sha256:${"1".repeat(64)}`,
    accessible_label: id,
    content: {
      kind: "vector_path" as const,
      view_box: "0 0 1 1",
      path: "M0 0",
    },
  };
}
export const providerUiContractFixture = defineProviderUiContract({
  logic: {
    schema_version: 1,
    state: [
      { id: "busy", value_type: "bool", initial: false },
      { id: "count", value_type: "integer", initial: 0 },
      { id: "detail", value_type: "string", initial: "" },
    ],
    messages: [
      { id: "start", payload: { count: "integer" } },
      { id: "docs", payload: {} },
      { id: "done", payload: {} },
      { id: "failed", payload: { detail: "string" } },
      { id: "reset", payload: {} },
    ],
    effects: [
      {
        id: "install",
        capability: "install_on_machine",
        request: {},
        success_message: "done",
        failure_message: "failed",
      },
      {
        id: "docs",
        capability: "open_external_documentation",
        request: {},
        success_message: "done",
        failure_message: "failed",
      },
    ],
    reducers: [
      {
        message: "start",
        assignments: [
          { field: "busy", value: { source: "literal", value: true } },
          { field: "count", value: { source: "message", field: "count" } },
        ],
      },
      { message: "start", assignments: [], effect: "install" },
      { message: "docs", assignments: [], effect: "docs" },
      {
        message: "done",
        assignments: [
          { field: "busy", value: { source: "literal", value: false } },
        ],
      },
      {
        message: "failed",
        assignments: [
          { field: "busy", value: { source: "literal", value: false } },
          { field: "detail", value: { source: "message", field: "detail" } },
        ],
      },
      {
        message: "reset",
        assignments: [
          { field: "count", value: { source: "state", field: "count" } },
        ],
      },
    ],
  },
  ui: {
    schema_version: 1,
    assets: [asset("logo"), asset("icon"), asset("loading")],
    surfaces: {
      card: { component: "divider" },
      setup: { component: "divider" },
      settings: { component: "divider" },
      information: { component: "divider" },
      loading: { component: "divider" },
      error: { component: "divider" },
      session: { component: "divider" },
      empty: {
        component: "stack",
        direction: "column",
        gap: "sm",
        visible_when: {
          op: "host_equals",
          field: "machine_online",
          value: true,
        },
        children: [
          {
            component: "button",
            style: "primary",
            label: { source: "literal", value: "Install" },
            emit: { message: "start", payload: { count: 1 } },
            enabled_when: { op: "state_equals", field: "busy", value: false },
          },
          {
            component: "button",
            style: "secondary",
            label: { source: "literal", value: "Docs" },
            emit: { message: "docs", payload: {} },
          },
          {
            component: "text",
            variant: "caption",
            value: { source: "state", field: "detail" },
          },
        ],
      },
    },
  },
});

export function providerUiManifestFixture(): ProviderUiManifest {
  return {
    ...structuredClone(providerUiContractFixture),
    id: "example",
    version: "1.0.0",
    publisher: "test",
    sdk_version: "3.0.0",
    display: {
      name: "Example",
      vendor: "Test",
      summary: "Typed fixture",
      accent: "#000000",
      secondary_accent: "#ffffff",
      logo_asset: "logo",
      icon_asset: "icon",
    },
    configuration: {
      schema_version: 1,
      presets: [{
        id: "recommended",
        name: "Recommended",
        detail: "Typed fixture",
        is_default: true,
        values: { model: "fixture" },
      }],
      options: [],
    },
    host: {
      schema_version: 1,
      conversation_compaction: {
        aliases: ["compact"],
        fallback_command: "compact",
      },
      account_usage: { provider: "openai" },
      features: [],
      tool_presentations: [],
    },
    authentication: { schema_version: 1, required: false, methods: [] },
    compatibility: {
      min_controller_contract: 2,
      max_controller_contract: 2,
      min_machine_contract: 4,
      max_machine_contract: 4,
      ui_component_fingerprint: `sha256:${"2".repeat(64)}`,
      auth_contract_fingerprint: `sha256:${"3".repeat(64)}`,
    },
  };
}
