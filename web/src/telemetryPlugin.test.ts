import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import {
  genericPluginCompatibilityProblem,
  type PluginContractInventory,
  projectAgentPluginInventory,
  type TelemetryBackendContract,
  validateTelemetryBackendContract,
} from "@cowboy/provider-ui";

Deno.test("telemetry payloads are closed and never acquire Agent runtime or credentials", () => {
  const value: TelemetryBackendContract = {
    schema_version: 1,
    id: "victoria",
    version: "1.0.0",
    display_name: "Victoria",
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    logs: {
      encoding: "json_lines",
      path: "/insert/jsonline",
      query: { _stream_fields: "component,platform" },
    },
  };
  assertEquals(validateTelemetryBackendContract(value), value);
  for (
    const extra of [
      { runtime: {} },
      { authorization: "private" },
      { endpoints: [] },
      { logs: null },
      {
        supported_platforms: [
          ...value.supported_platforms,
          ...value.supported_platforms,
        ],
      },
    ]
  ) {
    assertThrows(() =>
      validateTelemetryBackendContract({ ...value, ...extra })
    );
  }
  for (const path of ["//attacker/path", "/../escape", "/path?token=secret"]) {
    assertThrows(() =>
      validateTelemetryBackendContract({
        ...value,
        logs: { ...value.logs, path },
      })
    );
  }
  assertThrows(() =>
    validateTelemetryBackendContract({
      ...value,
      logs: { ...value.logs, query: { token: "private" } },
    })
  );
  assertEquals(
    projectAgentPluginInventory([{
      plugin_id: "victoria",
      plugin_version: "1.0.0",
      plugin_kind: "telemetry_backend",
      generation_digest: `sha256:${"ab".repeat(32)}`,
      contract_fingerprint: `sha256:${"cd".repeat(32)}`,
      state: "active",
      active_session_leases: 0,
      replica_state: "absent",
      materialization_state: "not_installed",
    }]),
    [],
  );
});

Deno.test("telemetry installation requires attested SDK 1.7 and its declared platform", () => {
  const inventory: PluginContractInventory = {
    plugin_sdk_version: "1.7.0",
    min_manifest_schema: 1,
    max_manifest_schema: 1,
    min_package_schema: 1,
    max_package_schema: 1,
    min_release_schema: 1,
    max_release_schema: 2,
    min_agent_provider_schema: 2,
    max_agent_provider_schema: 2,
    min_authentication_provider_schema: 1,
    max_authentication_provider_schema: 2,
    min_code_intelligence_schema: 1,
    max_code_intelligence_schema: 2,
    min_host_bundle_schema: 1,
    max_host_bundle_schema: 1,
    min_host_schema: 2,
    max_host_schema: 2,
  };
  const entry = {
    plugin_id: "victoria",
    plugin_version: "1.0.0",
    release_state: "ready",
    supported_platforms: [{
      os: "linux" as const,
      architecture: "x86_64" as const,
    }],
    compatibility_requirements: {
      plugin_kind: "telemetry_backend" as const,
      plugin_sdk_version: "1.7.0",
      manifest_schema: 1,
      package_schema: 1,
      release_schema: 1,
      payload_schema: 1,
    },
  };
  const target = {
    platform: "linux" as const,
    architecture: "x86_64" as const,
    plugin_contracts: inventory,
  };
  assertEquals(genericPluginCompatibilityProblem(entry, target), undefined);
  assert(
    genericPluginCompatibilityProblem(entry, {
      ...target,
      plugin_contracts: { ...inventory, plugin_sdk_version: "1.6.0" },
    }),
  );
  assert(
    genericPluginCompatibilityProblem(entry, { ...target, platform: "macos" }),
  );
  assert(
    genericPluginCompatibilityProblem({
      ...entry,
      compatibility_requirements: {
        ...entry.compatibility_requirements,
        payload_schema: 2,
      },
    }, target),
  );
});
