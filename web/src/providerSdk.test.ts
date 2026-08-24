import { assertEquals, assertThrows } from "jsr:@std/assert";
import {
  compareProviderVersions,
  evaluateExpression,
  initialProviderState,
  projectAgentPluginInventory,
  type ProviderCatalogEntry,
  providerCompatibilityProblem,
  type ProviderContractInventory,
  type ProviderHostContext,
  type ProviderManifest,
  type ProviderUiManifest,
  transitionProvider,
  validateMachineProviderInventory,
  validateProviderCatalog,
  validateProviderContractInventory,
  validateProviderManifest,
  validateProviderUiManifest,
} from "@cowboy/provider-ui";
import {
  exactProviderEntry,
  joinProviderInstallations,
  latestCompatibleProviderEntries,
  providerAuthenticationExecutorEntry,
  providerEntryForIdentity,
  providerPresentationEntry,
} from "./providerCatalogRegistry.ts";

function manifest(): ProviderManifest {
  const text = {
    component: "text",
    variant: "body",
    value: { source: "literal", value: "ok" },
  } as const;
  return {
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
    ui: {
      schema_version: 1,
      assets: ["logo", "icon", "loading"].map((id) => ({
        id,
        role: id as "logo" | "icon" | "loading",
        media_type: "image/svg+xml",
        digest: `sha256:${"1".repeat(64)}`,
        accessible_label: id,
        content: { kind: "vector_path", view_box: "0 0 1 1", path: "M0 0" },
      })),
      surfaces: {
        card: text,
        setup: text,
        settings: text,
        information: text,
        empty: text,
        loading: text,
        error: text,
        session: text,
      },
    },
    logic: {
      schema_version: 1,
      state: [{ id: "open", value_type: "bool", initial: false }],
      messages: [
        { id: "toggle", payload: {} },
        { id: "done", payload: {} },
        { id: "failed", payload: { detail: "string" } },
      ],
      reducers: [{
        message: "toggle",
        assignments: [{
          field: "open",
          value: { source: "literal", value: true },
        }],
        effect: "toggle",
      }],
      effects: [{
        id: "toggle",
        capability: "install_on_machine",
        request: {},
        success_message: "done",
        failure_message: "failed",
      }],
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
    runtime: {
      driver_schema: 1,
      protocol: "test",
      entrypoint: "test",
      behavior: {
        schema_version: 1,
        permission: "portable_v1",
        session: "portable_v1",
        turn_end: "portable_v1",
        configuration: "portable_v1",
        default_preferences: {},
        error_rules: [],
      },
      arguments: [],
      environment: {},
      sidecars: [],
      remove_environment: [],
      remove_environment_prefixes: [],
      dependencies: [{
        id: "fixture-runtime",
        version: "1.0.0",
        source: "https://example.invalid/fixture-runtime-1.0.0.tgz",
        integrity: `sha256:${"a".repeat(64)}`,
        private: true,
      }],
      platforms: [{
        os: "linux",
        architecture: "x86_64",
        payload_digest: `sha256:${"b".repeat(64)}`,
        launch_command: "fixture-runtime",
        private_components: [{
          kind: "provider_cli",
          slot: "fixture",
          dependency: "fixture-runtime",
          command: "fixture-runtime",
        }],
      }],
      required_capabilities: ["provider.runtime.v1"],
    },
    authentication: {
      schema_version: 1,
      required: false,
      portable_schema: "none-v1",
      projection_schema: "none-v1",
      refresh: "service",
      methods: [],
      credential_files: [],
      environment_projection: {},
    },
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

function uiManifest(full = manifest()): ProviderUiManifest {
  const { runtime: _runtime, authentication, ...ui } = full;
  return {
    ...ui,
    authentication: {
      schema_version: authentication.schema_version,
      required: authentication.required,
      presentation:
        authentication.required && authentication.methods.length > 0 &&
          authentication.methods.every(({ flow }) => flow === "secret_input")
          ? "api_key"
          : "account",
      methods: authentication.methods.map(({ id, label, flow }) => ({
        id,
        label,
        flow,
      })),
    },
  };
}

Deno.test("Provider SDK validates and executes linked typed logic", () => {
  const fixture = manifest();
  validateProviderManifest(fixture);
  const result = transitionProvider(fixture, initialProviderState(fixture), {
    message: "toggle",
    payload: {},
  });
  assertEquals(result.state.open, true);
  assertEquals(result.effect?.capability, "install_on_machine");
});

Deno.test("Machine Plugin inventory projects only Agent capability entries", () => {
  const shared = {
    generation_digest: `sha256:${"4".repeat(64)}`,
    contract_fingerprint: `sha256:${"5".repeat(64)}`,
    state: "active",
    active_session_leases: 0,
    replica_state: "absent",
    materialization_state: "not_installed",
  } as const;
  const inventory = [
    {
      ...shared,
      plugin_id: "codex",
      plugin_version: "3.0.0",
      plugin_kind: "agent_provider",
    },
    {
      ...shared,
      plugin_id: "zed",
      plugin_version: "1.0.0",
      plugin_kind: "code_intelligence",
    },
  ];
  assertEquals(projectAgentPluginInventory(inventory), [{
    ...shared,
    provider_id: "codex",
    provider_version: "3.0.0",
  }]);
});

Deno.test("Provider UI accepts only closed Service authentication presentations", () => {
  const account = uiManifest();
  account.authentication.presentation = "account";
  validateProviderUiManifest(account);

  const apiKey = uiManifest();
  apiKey.authentication.required = true;
  apiKey.authentication.methods = [{
    id: "api-key",
    label: "API key",
    flow: "secret_input",
  }];
  apiKey.authentication.presentation = "api_key";
  validateProviderUiManifest(apiKey);

  const mismatched = uiManifest();
  mismatched.authentication.presentation = "api_key";
  assertThrows(
    () => validateProviderUiManifest(mismatched),
    Error,
    "does not match its typed methods",
  );

  const unknown = uiManifest() as unknown as Record<string, unknown>;
  (unknown.authentication as Record<string, unknown>).presentation = "login";
  assertThrows(
    () => validateProviderUiManifest(unknown),
    Error,
    "Invalid Provider UI authentication contract",
  );
});

Deno.test("Provider UI schema 2 accepts bounded gradient and activity IR", () => {
  const fixture = manifest();
  fixture.sdk_version = "3.0.0";
  fixture.ui.schema_version = 2;
  const icon = fixture.ui.assets.find((asset) => asset.id === "icon")!;
  if (icon.content.kind !== "vector_path") throw new Error("missing vector");
  icon.content.gradient = {
    x1_percent: 0,
    y1_percent: 0,
    x2_percent: 100,
    y2_percent: 100,
    stops: [
      { offset_percent: 0, color: "#4F6BED" },
      { offset_percent: 100, color: "#168B78" },
    ],
  };
  fixture.ui.surfaces.loading = {
    component: "activity",
    indicator: { kind: "terminal_prompt", interval_ms: 4_200 },
    label: {
      kind: "phrase_cycle",
      phrases: ["Inspecting", "Testing"],
      interval_ms: 4_200,
      suffix: "…",
      effect: "shimmer",
    },
    accessible_label: "Example is working",
  };
  validateProviderManifest(fixture);
});

Deno.test("Provider UI rejects v2 nodes in v1 and unknown future interfaces", () => {
  const fixture = manifest();
  fixture.ui.surfaces.loading = {
    component: "activity",
    indicator: { kind: "progress_ring" },
    label: {
      kind: "text",
      value: { source: "literal", value: "Thinking…" },
      effect: "none",
    },
    accessible_label: "Example is working",
  };
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "requires UI schema 2",
  );
  fixture.ui.schema_version = 3;
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "Invalid Provider manifest envelope",
  );

  const unknown = manifest();
  unknown.sdk_version = "3.0.0";
  unknown.ui.schema_version = 2;
  unknown.ui.surfaces.loading = {
    component: "activity",
    indicator: { kind: "glyph_cycle", frames: ["·", "✢"], interval_ms: 200 },
    label: {
      kind: "text",
      value: { source: "literal", value: "Thinking…" },
      effect: "none",
    },
    accessible_label: "Example is working",
  };
  const unknownIndicator = unknown.ui.surfaces.loading
    .indicator as unknown as Record<string, unknown>;
  unknownIndicator.speed_curve = "provider_magic";
  assertThrows(
    () => validateProviderManifest(unknown),
    Error,
    "Invalid Provider glyph activity",
  );
});

Deno.test("Provider UI rejects unbounded activity motion before rendering", () => {
  const fixture = manifest();
  fixture.sdk_version = "3.0.0";
  fixture.ui.schema_version = 2;
  fixture.ui.surfaces.loading = {
    component: "activity",
    indicator: { kind: "glyph_cycle", frames: ["·", "✢"], interval_ms: 1 },
    label: {
      kind: "text",
      value: { source: "literal", value: "Thinking…" },
      effect: "none",
    },
    accessible_label: "Example is working",
  };
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "Invalid Provider glyph activity",
  );
});

Deno.test("Provider host schema 2 accepts only bounded Transcript variants", () => {
  for (
    const variant of ["timeline", "workcell", "signal", "terminal"] as const
  ) {
    const fixture = manifest();
    fixture.sdk_version = "3.0.0";
    fixture.host = {
      ...fixture.host,
      schema_version: 2,
      transcript: {
        schema_version: 1,
        thought: {
          variant,
          density: variant === "terminal" ? "compact" : "comfortable",
          active_label: "Thinking",
          current_surface: variant === "timeline" ? "plain" : "soft",
        },
      },
    };
    validateProviderManifest(fixture);
  }

  const legacy = manifest() as unknown as Record<string, unknown>;
  const legacyHost = legacy.host as Record<string, unknown>;
  legacyHost.transcript = {
    schema_version: 1,
    thought: {
      variant: "timeline",
      density: "comfortable",
      current_surface: "plain",
    },
  };
  assertThrows(
    () => validateProviderManifest(legacy),
    Error,
    "schema 1 cannot declare Transcript presentation",
  );

  const missing = manifest() as unknown as Record<string, unknown>;
  (missing.host as Record<string, unknown>).schema_version = 2;
  assertThrows(
    () => validateProviderManifest(missing),
    Error,
    "Invalid Provider Transcript presentation contract",
  );

  const unknown = manifest() as unknown as Record<string, unknown>;
  unknown.sdk_version = "3.0.0";
  const unknownHost = unknown.host as Record<string, unknown>;
  unknownHost.schema_version = 2;
  unknownHost.transcript = {
    schema_version: 1,
    thought: {
      variant: "provider_canvas",
      density: "comfortable",
      current_surface: "plain",
    },
  };
  assertThrows(
    () => validateProviderManifest(unknown),
    Error,
    "Invalid Provider Transcript thought presentation",
  );

  const stringSchema = manifest() as unknown as Record<string, unknown>;
  (stringSchema.host as Record<string, unknown>).schema_version = "1";
  assertThrows(
    () => validateProviderManifest(stringSchema),
    Error,
    "Invalid Provider manifest envelope",
  );
});

Deno.test("Provider SDK rejects assignment type drift before rendering", () => {
  const fixture = manifest();
  fixture.logic.reducers[0]!.assignments[0]!.value = {
    source: "literal",
    value: "not-a-boolean",
  };
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "assignment type mismatch",
  );
});

Deno.test("Provider SDK host expressions are closed and deterministic", () => {
  const host: ProviderHostContext = {
    provider_version: "1.0.0",
    installation_state: "active",
    authentication_state: "ready",
    distribution_state: "current",
    machine_name: "hawk",
    error_detail: "",
    installed: true,
    auth_ready: true,
    auth_required: true,
    machine_online: true,
    upgrade_available: false,
  };
  assertEquals(
    evaluateExpression(
      {
        op: "all",
        values: [
          { op: "host_equals", field: "installed", value: true },
          {
            op: "not",
            value: {
              op: "host_equals",
              field: "upgrade_available",
              value: true,
            },
          },
        ],
      },
      {},
      host,
    ),
    true,
  );
});

Deno.test("Provider SDK validates closed signed tool presentation links", () => {
  const fixture = manifest();
  fixture.host.tool_presentations = [{
    tool_name: "FutureTodo",
    renderer: "todo_list_v1",
  }];
  validateProviderManifest(fixture);
  fixture.host.tool_presentations.push({
    tool_name: "FutureTodo",
    renderer: "todo_list_v1",
  });
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "Duplicate Provider tool presentation",
  );
});

Deno.test("Provider Catalog distinguishes typed unbound packages from installable releases", () => {
  const fixture = uiManifest();
  const catalog = {
    providers: [{
      provider_id: fixture.id,
      provider_version: fixture.version,
      package_digest: `sha256:${"4".repeat(64)}`,
      artifact_digest: null,
      authentication_scope: "none-v1",
      release_state: "unbound",
      release_detail: "runtime is not published",
      publisher: fixture.publisher,
      contract_fingerprint: `sha256:${"5".repeat(64)}`,
      supported_platforms: [{ os: "linux", architecture: "x86_64" }],
      manifest: fixture,
    }],
    authentications: [],
    authentication_executors: [],
  };
  validateProviderCatalog(catalog);
  const leaked = structuredClone(catalog) as unknown as {
    providers: Array<{ manifest: Record<string, unknown> }>;
    authentications: unknown[];
    authentication_executors: unknown[];
  };
  leaked.providers[0]!.manifest.runtime = {};
  assertThrows(
    () => validateProviderCatalog(leaked),
    Error,
    "exposed a private Provider contract",
  );
  catalog.providers[0]!.release_state = "ready";
  assertThrows(
    () => validateProviderCatalog(catalog),
    Error,
    "not content addressed",
  );
});

Deno.test("Provider Catalog preserves presentation for historical SDK releases", () => {
  const fixture = uiManifest();
  fixture.sdk_version = "2.4.0";
  const catalog = {
    providers: [{
      provider_id: fixture.id,
      provider_version: fixture.version,
      package_digest: `sha256:${"4".repeat(64)}`,
      artifact_digest: `sha256:${"5".repeat(64)}`,
      authentication_scope: "none-v1",
      release_state: "ready",
      publisher: fixture.publisher,
      contract_fingerprint: `sha256:${"6".repeat(64)}`,
      supported_platforms: [{ os: "linux", architecture: "x86_64" }],
      manifest: fixture,
    }],
    authentications: [],
    authentication_executors: [],
  };

  validateProviderCatalog(catalog);
  assertThrows(
    () => validateProviderUiManifest(fixture),
    Error,
    "is incompatible",
  );
});

Deno.test("Provider Catalog accepts a first Service authentication in progress", () => {
  const fixture = manifest();
  const status = {
    provider_id: fixture.id,
    auth_generation: 0,
    authentication_state: "authenticating",
    distribution_state: "none",
    auth_contract_fingerprint: fixture.compatibility.auth_contract_fingerprint,
    authentication_scope: fixture.authentication.portable_schema,
    portable_schema: fixture.authentication.portable_schema,
    projection_schema: fixture.authentication.projection_schema,
    updated_at_ms: 1,
  };
  validateProviderCatalog({
    providers: [],
    authentications: [status],
    authentication_executors: [],
  });
  status.authentication_state = "ready";
  assertThrows(
    () =>
      validateProviderCatalog({
        providers: [],
        authentications: [status],
        authentication_executors: [],
      }),
    Error,
    "Invalid Provider authentication status",
  );
});

Deno.test("Provider Catalog validates exact temporary authentication executors", () => {
  const executor = {
    provider_id: "codex",
    provider_version: "1.1.1",
    generation_digest: `sha256:${"a".repeat(64)}`,
  };
  validateProviderCatalog({
    providers: [],
    authentications: [],
    authentication_executors: [executor],
  });
  assertThrows(
    () =>
      validateProviderCatalog({
        providers: [],
        authentications: [],
        authentication_executors: [executor, executor],
      }),
    Error,
    "Duplicate Provider authentication executor",
  );
});

Deno.test("Service authentication selects the newest exact active executor", () => {
  const olderManifest = uiManifest();
  olderManifest.authentication.required = true;
  olderManifest.authentication.presentation = "account";
  olderManifest.authentication.methods = [{
    id: "account",
    label: "Account",
    flow: "device_code",
  }];
  const older: ProviderCatalogEntry = {
    provider_id: olderManifest.id,
    provider_version: "1.0.0",
    package_digest: `sha256:${"1".repeat(64)}`,
    artifact_digest: `sha256:${"2".repeat(64)}`,
    authentication_scope: "example-auth-v1",
    release_state: "ready",
    publisher: olderManifest.publisher,
    contract_fingerprint: `sha256:${"3".repeat(64)}`,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest: olderManifest,
  };
  const latestManifest = structuredClone(olderManifest);
  latestManifest.version = "1.1.0";
  const latest: ProviderCatalogEntry = {
    ...older,
    provider_version: latestManifest.version,
    package_digest: `sha256:${"4".repeat(64)}`,
    artifact_digest: `sha256:${"5".repeat(64)}`,
    manifest: latestManifest,
  };
  assertEquals(
    providerAuthenticationExecutorEntry(
      {
        providers: [latest, older],
        authentications: [],
        authentication_executors: [{
          provider_id: older.provider_id,
          provider_version: older.provider_version,
          generation_digest: older.artifact_digest!,
        }],
      },
      older.provider_id,
      "account",
    ),
    older,
  );
});

Deno.test("Machine Provider UI resolves the exact installed package", () => {
  const firstManifest = uiManifest();
  const first: ProviderCatalogEntry = {
    provider_id: firstManifest.id,
    provider_version: firstManifest.version,
    package_digest: `sha256:${"2".repeat(64)}`,
    artifact_digest: `sha256:${"3".repeat(64)}`,
    authentication_scope: "none-v1",
    release_state: "ready",
    publisher: firstManifest.publisher,
    contract_fingerprint: `sha256:${"6".repeat(64)}`,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest: firstManifest,
  };
  const secondManifest = uiManifest();
  secondManifest.version = "2.0.0";
  const second: ProviderCatalogEntry = {
    ...first,
    provider_version: secondManifest.version,
    package_digest: `sha256:${"4".repeat(64)}`,
    artifact_digest: `sha256:${"5".repeat(64)}`,
    manifest: secondManifest,
  };
  assertEquals(
    exactProviderEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    first,
  );
  assertEquals(
    providerEntryForIdentity(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    first,
  );
  assertEquals(
    providerEntryForIdentity(
      [second, first],
      first.provider_id,
      first.provider_version,
      `sha256:${"9".repeat(64)}`,
    ),
    undefined,
  );
  assertEquals(
    providerEntryForIdentity([first, second], first.provider_id),
    second,
  );
  assertEquals(
    providerPresentationEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    second,
  );
  second.manifest.ui.schema_version = 2;
  second.release_state = "unbound";
  assertEquals(
    providerPresentationEntry(
      [second],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    second,
  );
  assertEquals(
    providerPresentationEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    first,
  );
  second.release_state = "ready";
  assertEquals(
    providerPresentationEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    second,
  );
  first.manifest.ui.schema_version = 2;
  second.manifest.host = {
    ...second.manifest.host,
    schema_version: 2,
    transcript: {
      schema_version: 1,
      thought: {
        variant: "workcell",
        density: "comfortable",
        active_label: "Thinking",
        current_surface: "soft",
      },
    },
  };
  assertEquals(
    providerPresentationEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    second,
  );
  first.manifest.host = {
    ...first.manifest.host,
    schema_version: 2,
    transcript: {
      schema_version: 1,
      thought: {
        variant: "timeline",
        density: "comfortable",
        current_surface: "plain",
      },
    },
  };
  const { transcript: _transcript, ...legacyHost } = second.manifest.host;
  second.manifest.host = { ...legacyHost, schema_version: 1 };
  assertEquals(
    providerPresentationEntry(
      [second, first],
      first.provider_id,
      first.provider_version,
      first.artifact_digest!,
    ),
    first,
  );
  const installed = {
    provider_id: first.provider_id,
    provider_version: first.provider_version,
    generation_digest: first.artifact_digest!,
    contract_fingerprint: first.contract_fingerprint,
    state: "active",
    active_session_leases: 0,
    replica_state: "absent",
    materialization_state: "not_installed",
  } as const;
  const [joined] = joinProviderInstallations([second, first], [installed]);
  assertEquals(joined?.latestEntry, second);
  assertEquals(joined?.installedEntry, first);

  const [missingExact] = joinProviderInstallations([second], [installed]);
  assertEquals(missingExact?.latestEntry, second);
  assertEquals(missingExact?.installedEntry, undefined);

  const orphan = { ...installed, provider_id: "orphaned-provider" };
  const orphaned = joinProviderInstallations([], [orphan]);
  assertEquals(orphaned[0]?.providerId, "orphaned-provider");
  assertEquals(orphaned[0]?.latestEntry, undefined);
});

Deno.test("Machine Provider capabilities select the newest compatible release", () => {
  const legacyManifest = uiManifest();
  legacyManifest.host.schema_version = 1;
  const legacy: ProviderCatalogEntry = {
    provider_id: legacyManifest.id,
    provider_version: legacyManifest.version,
    package_digest: `sha256:${"1".repeat(64)}`,
    artifact_digest: `sha256:${"2".repeat(64)}`,
    authentication_scope: "none-v1",
    release_state: "ready",
    publisher: legacyManifest.publisher,
    contract_fingerprint: `sha256:${"3".repeat(64)}`,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest: legacyManifest,
  };
  const currentManifest = uiManifest();
  currentManifest.version = "2.0.0";
  currentManifest.host.schema_version = 2;
  currentManifest.host.transcript = {
    schema_version: 1,
    thought: {
      variant: "workcell",
      density: "comfortable",
      active_label: "Thinking",
      current_surface: "soft",
    },
  };
  const current: ProviderCatalogEntry = {
    ...legacy,
    provider_version: currentManifest.version,
    package_digest: `sha256:${"4".repeat(64)}`,
    artifact_digest: `sha256:${"5".repeat(64)}`,
    manifest: currentManifest,
  };
  const legacyMachine: ProviderContractInventory = {
    provider_sdk_version: "3.0.0",
    min_package_schema: 1,
    max_package_schema: 2,
    min_runtime_binding_schema: 1,
    max_runtime_binding_schema: 2,
    min_ui_schema: 1,
    max_ui_schema: 2,
    min_host_schema: 1,
    max_host_schema: 1,
    machine_contract: 4,
  };
  const target = {
    platform: "linux" as const,
    architecture: "x86_64" as const,
    provider_contracts: legacyMachine,
  };
  assertEquals(latestCompatibleProviderEntries([current, legacy], target), [
    legacy,
  ]);
  assertEquals(
    providerCompatibilityProblem(current, target)?.code,
    "host_schema_unsupported",
  );
  const [joined] = joinProviderInstallations([current, legacy], [], target);
  assertEquals(joined?.latestEntry, current);
  assertEquals(joined?.latestCompatibleEntry, legacy);
  assertEquals(joined?.latestCompatibility?.code, "host_schema_unsupported");

  const unpublishedCurrent = {
    ...current,
    release_state: "unbound" as const,
    artifact_digest: null,
  };
  const [readyFallback] = latestCompatibleProviderEntries(
    [unpublishedCurrent, legacy],
    { ...target, provider_contracts: { ...legacyMachine, max_host_schema: 2 } },
  );
  assertEquals(readyFallback, legacy);

  const currentMachine = { ...legacyMachine, max_host_schema: 2 };
  assertEquals(
    latestCompatibleProviderEntries([current, legacy], {
      ...target,
      provider_contracts: currentMachine,
    }),
    [current],
  );
  assertEquals(
    providerCompatibilityProblem(current, {
      platform: "linux",
      architecture: "x86_64",
    })?.code,
    "capability_inventory_unavailable",
  );
});

Deno.test("Machine Provider capability inventories reject unknown fields", () => {
  assertThrows(
    () =>
      validateProviderContractInventory({
        provider_sdk_version: "3.0.0",
        min_package_schema: 1,
        max_package_schema: 2,
        min_runtime_binding_schema: 1,
        max_runtime_binding_schema: 2,
        min_ui_schema: 1,
        max_ui_schema: 2,
        min_host_schema: 1,
        max_host_schema: 2,
        machine_contract: 4,
        future_field: true,
      }),
    Error,
    "Invalid Machine Provider capability inventory",
  );
});

Deno.test("Provider SDK rejects unknown behavior profiles and unsafe retry rules", () => {
  const unknown = manifest() as unknown as Record<string, unknown>;
  const runtime = unknown.runtime as Record<string, unknown>;
  const behavior = runtime.behavior as Record<string, unknown>;
  behavior.permission = "provider_specific_magic";
  assertThrows(
    () => validateProviderManifest(unknown),
    Error,
    "behavior contract",
  );

  const unsafe = manifest();
  unsafe.runtime.behavior.error_rules.push({
    when: { op: "contains", value: "retry" },
    retry_once_without_visible_update: true,
    keep_worker_alive: false,
  });
  assertThrows(
    () => validateProviderManifest(unsafe),
    Error,
    "must keep the worker alive",
  );
});

Deno.test("Provider SDK rejects runtime bindings that do not export their launch command", () => {
  const fixture = manifest();
  fixture.runtime.platforms[0]!.private_components[0]!.command =
    "different-command";
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "launch command",
  );
});

Deno.test("Provider SDK keeps effects inside their typed lifecycle surfaces", () => {
  const fixture = manifest();
  fixture.ui.surfaces.settings = {
    component: "button",
    label: { source: "literal", value: "Wrong scope" },
    style: "primary",
    emit: { message: "toggle", payload: {} },
  };
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "is not allowed on settings surface",
  );
});

Deno.test("Provider SDK enforces semantic release identity and precedence", () => {
  assertEquals(compareProviderVersions("1.0.0-rc.10", "1.0.0-rc.2"), 1);
  assertEquals(compareProviderVersions("1.0.0", "1.0.0-rc.10"), 1);
  const fixture = manifest();
  fixture.id = "..";
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "manifest envelope",
  );
  fixture.id = "example";
  fixture.version = "1.0";
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "manifest envelope",
  );
});

Deno.test("Provider SDK rejects incompatible authoring SDK versions before rendering", () => {
  const newer = manifest();
  newer.sdk_version = "3.1.3";
  assertThrows(() => validateProviderManifest(newer), Error, "is incompatible");

  const current = manifest();
  current.sdk_version = "3.1.2";
  validateProviderManifest(current);

  const oldMajor = manifest();
  oldMajor.sdk_version = "2.99.0";
  assertThrows(
    () => validateProviderManifest(oldMajor),
    Error,
    "is incompatible",
  );
});

Deno.test("Provider SDK links session sidecars, components, and auth projections", () => {
  const fixture = manifest();
  fixture.runtime.behavior.configuration = "openai_gateway_v1";
  fixture.runtime.required_capabilities.push("provider.gateway.v1");
  fixture.runtime.platforms[0]!.private_components.push({
    kind: "provider_gateway",
    slot: "gateway",
    dependency: "fixture-runtime",
    command: "fixture-gateway",
  });
  fixture.runtime.sidecars.push({
    id: "gateway",
    component: { kind: "provider_gateway", slot: "gateway" },
    arguments: [],
    environment: {},
    auth_environment: ["FIXTURE_API_KEY"],
    transport: {
      kind: "loopback_http_v1",
      listen_argument: "--listen",
      health_path: "/healthz",
      timeout_ms: 8_000,
    },
  });
  fixture.runtime.arguments.push({
    source: "sidecar_url",
    sidecar: "gateway",
    prefix: 'base_url="',
    suffix: '/v1"',
  });
  fixture.authentication.environment_projection.FIXTURE_API_KEY = "api_key";
  validateProviderManifest(fixture);

  fixture.runtime.sidecars[0]!.id = "renamed-gateway";
  assertThrows(
    () => validateProviderManifest(fixture),
    Error,
    "unknown sidecar",
  );
});

Deno.test("Machine Provider inventory uses the exact Rust protocol state union", () => {
  const inventory = [{
    provider_id: "gemini",
    provider_version: "1.0.0",
    generation_digest: `sha256:${"4".repeat(64)}`,
    contract_fingerprint: `sha256:${"5".repeat(64)}`,
    state: "incompatible",
    active_session_leases: 0,
    replica_state: "pending",
    materialization_state: "not_installed",
  }];
  assertEquals(validateMachineProviderInventory(inventory), inventory);

  const drifted = structuredClone(inventory) as Array<Record<string, unknown>>;
  drifted[0]!.state = "probing";
  assertThrows(
    () => validateMachineProviderInventory(drifted),
    Error,
    "Invalid Machine Provider inventory entry",
  );
});
