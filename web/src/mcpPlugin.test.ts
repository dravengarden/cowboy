import { test } from "bun:test";
import {
  genericPluginCompatibilityProblem,
  type PluginContractInventory,
  validatePluginCompatibilityRequirements,
} from "@cowboy/provider-ui";
import { assert, assertEquals, assertThrows } from "../../tools/test-assert.ts";

const requirements = {
  plugin_kind: "mcp_server" as const,
  plugin_sdk_version: "1.13.0",
  manifest_schema: 1,
  package_schema: 1,
  release_schema: 6,
  payload_schema: 1,
};

const inventory: PluginContractInventory = {
  plugin_sdk_version: "1.13.0",
  min_manifest_schema: 1,
  max_manifest_schema: 1,
  min_package_schema: 1,
  max_package_schema: 1,
  min_release_schema: 1,
  max_release_schema: 6,
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

test("MCP install compatibility requires the signed SDK and release reader floor", () => {
  assertEquals(validatePluginCompatibilityRequirements(requirements), requirements);
  const platform = { os: "linux" as const, architecture: "x86_64" as const };
  const entry = {
    plugin_id: "fixture-mcp", plugin_version: "1.0.0", release_state: "ready",
    supported_platforms: [platform], compatibility_requirements: requirements,
  };
  const target = { platform: platform.os, architecture: platform.architecture, plugin_contracts: inventory };
  assertEquals(genericPluginCompatibilityProblem(entry, target), undefined);
  for (const old of [{ plugin_sdk_version: "1.12.4" }, { max_release_schema: 5 }]) {
    assert(genericPluginCompatibilityProblem(entry, { ...target, plugin_contracts: { ...inventory, ...old } }));
  }
  assertThrows(() => validatePluginCompatibilityRequirements({ ...requirements, plugin_kind: "memory_backend" }));
  assertThrows(() => validatePluginCompatibilityRequirements({ ...requirements, host_bundle_schema: 1, host_schema: 2 }));
});
