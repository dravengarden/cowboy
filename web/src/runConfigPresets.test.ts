import { assertEquals } from "jsr:@std/assert";
import type { ProviderUiManifest } from "@cowboy/provider-ui";
import { providerUiManifestFixture } from "./providerUiContract.fixture";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry";
import type { ConfigOption } from "./protocol";
import {
  activeRunConfigPreset,
  runConfigCurrentTitle,
  runConfigPresetChanges,
  runConfigPresets,
  runConfigSummary,
  supportedRunConfigPresets,
} from "./runConfigPresets";

type DeclaredPreset = ProviderUiManifest["configuration"]["presets"][number];

function selectOption(
  id: string,
  currentValue: string,
  values: readonly string[],
): ConfigOption {
  return {
    id,
    name: id,
    currentValue,
    options: values.map((value) => ({ value, name: value })),
  };
}

const options: ConfigOption[] = [
  selectOption("model", "model-a", ["model-a", "model-b"]),
  selectOption("effort", "max", ["medium", "max"]),
];

const declared: DeclaredPreset[] = [{
  id: "recommended",
  name: "Recommended",
  detail: "Provider-owned recommendation",
  is_default: true,
  values: { model: "model-a", effort: "max" },
}, {
  id: "balanced",
  name: "Balanced",
  detail: "Provider-owned balanced mode",
  is_default: false,
  values: { model: "model-b", effort: "medium" },
}];

Deno.test("custom configuration remains visible without a matching preset", () => {
  const custom = [
    {
      ...options[0],
      name: "Model",
      options: [{ value: "model-a", name: "Model A" }],
    },
    { ...options[1], name: "Reasoning", currentValue: "high" },
  ];
  assertEquals(
    activeRunConfigPreset(supportedRunConfigPresets(declared, options), custom),
    undefined,
  );
  assertEquals(runConfigSummary(custom), "Model: Model A · Reasoning: high");
  assertEquals(
    runConfigCurrentTitle("High · Always approve"),
    "Current · High · Always approve",
  );
  assertEquals(runConfigCurrentTitle(undefined), "Current · Custom");
});

Deno.test("Codex recommends Astra Medium without changing the default", async () => {
  const provider = JSON.parse(
    await Deno.readTextFile(
      new URL("../../plugins/codex/provider.json", import.meta.url),
    ),
  );
  const presets = provider.configuration_presets as DeclaredPreset[];
  assertEquals(presets.find((preset) => preset.id === "astra-medium")?.values, {
    model: "gpt-6-astra",
    reasoning_effort: "medium",
  });
  assertEquals(
    presets.filter((preset) => preset.is_default).map((preset) => preset.id),
    ["sol-medium"],
  );
});

Deno.test("signed Provider presets project without Provider identity branches", () => {
  const presets = supportedRunConfigPresets(declared, options);
  assertEquals(presets.map((preset) => [preset.id, preset.isDefault]), [
    ["recommended", true],
    ["balanced", false],
  ]);
  assertEquals(activeRunConfigPreset(presets, options)?.id, "recommended");
});

Deno.test("presets fail closed when the live Provider surface lacks a value", () => {
  assertEquals(supportedRunConfigPresets(declared, options.slice(0, 1)), []);
  assertEquals(
    supportedRunConfigPresets([{
      ...declared[0],
      values: { model: "unknown" },
    }], options),
    [],
  );
});

Deno.test("preset changes omit values the session already owns", () => {
  const presets = supportedRunConfigPresets(declared, options);
  assertEquals(runConfigPresetChanges(presets[0], options), []);
  assertEquals(runConfigPresetChanges(presets[1], options), [
    { configId: "model", value: "model-b" },
    { configId: "effort", value: "medium" },
  ]);
});

Deno.test("another model's reasoning limits do not hide signed recommendations", () => {
  const live: ConfigOption[] = [
    {
      ...selectOption("model", "spark", ["spark", "astra"]),
      category: "model",
    },
    {
      ...selectOption("effort", "low", ["low", "medium"]),
      category: "thought_level",
    },
  ];
  const astra = { ...declared[0], values: { effort: "max", model: "astra" } };
  const presets = supportedRunConfigPresets([astra], live);
  assertEquals(presets.length, 1);
  assertEquals(runConfigPresetChanges(presets[0], live), [
    { configId: "model", value: "astra" },
    { configId: "effort", value: "max" },
  ]);
  assertEquals(
    supportedRunConfigPresets([{
      ...astra,
      values: { model: "spark", effort: "max" },
    }], live),
    [],
  );
  assertEquals(
    supportedRunConfigPresets([{
      ...astra,
      values: { model: "missing", effort: "max" },
    }], live),
    [],
  );
  assertEquals(supportedRunConfigPresets([astra], live.slice(0, 1)), []);
});

const desktopSource = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const storeSource = await Deno.readTextFile(
  new URL("./store.ts", import.meta.url),
);

Deno.test("desktop and mobile expose presets with surface-native interactions", () => {
  assertEquals(desktopSource.includes("data-config-preset={index}"), true);
  assertEquals(desktopSource.includes("presetShortcutLabel"), true);
  assertEquals(composerSource.includes("minHeight: 58"), true);
  assertEquals(composerSource.includes("in={showAgentDetails}"), true);
  assertEquals(composerSource.includes("setCustomizeAgent(false)"), true);
});

Deno.test("mobile current configuration is a disclosure, not a fake card", () => {
  assertEquals(composerSource.includes("runConfigCurrentTitle("), true);
  assertEquals(
    composerSource.includes("Expand current agent configuration"),
    true,
  );
  assertEquals(composerSource.includes('role="status"'), false);
  assertEquals(
    /variant="body2"[^>]*>\s*Customize\s*</.test(composerSource),
    false,
  );
});

Deno.test("mobile preset progress is delayed, acknowledged, and bounded", () => {
  assertEquals(composerSource.includes("presetAction.run"), true);
  assertEquals(composerSource.includes("presetAction.progress"), true);
  assertEquals(composerSource.includes("setSessionConfigOptions"), true);
  assertEquals(
    composerSource.includes("runConfigPresetChanges(pendingPreset, options)"),
    false,
  );
  assertEquals(storeSource.includes("configOptionsMatchChanges("), true);
  assertEquals(storeSource.includes('"Update agent preset"'), true);
  assertEquals(storeSource.includes("canonicalAckListeners"), true);
  assertEquals(storeSource.includes("notifyCanonicalAcks"), true);
  // Only canonical acknowledgements belong to this helper. Later presentation
  // observers intentionally subscribe to the separately held painted state.
  const waitForState = storeSource.match(
    /^function waitForState\([\s\S]*?^\}/m,
  )?.[0] ?? "";
  assertEquals(waitForState.startsWith("function waitForState("), true);
  assertEquals(
    waitForState.includes("canonicalAckListeners.add(check)"),
    true,
  );
  assertEquals(
    waitForState.includes("canonicalAckListeners.delete(check)"),
    true,
  );
  assertEquals(waitForState.includes("listeners.add(check)"), false);
});

Deno.test("a session adopts presets published after its installed generation", async () => {
  // This is the sess-1789450485879 shape: the Machine runs claude-code 3.1.14,
  // which declared no presets, while the presets ship in a later published
  // generation. Pinning presentation to the installed package would leave the
  // session without chips until someone upgrades the Machine.
  const installed = providerUiManifestFixture();
  installed.id = "claude-code";
  installed.version = "3.1.14";
  installed.configuration = { schema_version: 1, presets: [], options: [] };

  const published = providerUiManifestFixture();
  published.id = "claude-code";
  published.version = "3.1.20";
  published.configuration = {
    schema_version: 1,
    presets: [{
      id: "sonnet-medium",
      name: "Sonnet · Medium",
      detail: "Sonnet 5 · Medium effort",
      is_default: false,
      values: { model: "model-b", effort: "medium" },
    }],
    options: [],
  };

  const entry = (manifest: ProviderUiManifest, digest: string) => ({
    provider_id: manifest.id,
    provider_version: manifest.version,
    package_digest: digest,
    artifact_digest: digest,
    authentication_scope: "none-v1",
    release_state: "ready",
    release_detail: "published",
    publisher: manifest.publisher,
    contract_fingerprint: digest,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest,
  });

  const previous = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(
        JSON.stringify({
          providers: [
            entry(installed, `sha256:${"1".repeat(64)}`),
            entry(published, `sha256:${"2".repeat(64)}`),
          ],
          authentications: [],
          authentication_executors: [],
          platform: { hosts: [] },
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )) as typeof globalThis.fetch;
  try {
    resetProviderCatalog();
    await loadProviderCatalog(true);
    const resolved = runConfigPresets(
      "claude-code",
      options,
      "3.1.14",
      `sha256:${"1".repeat(64)}`,
    );
    assertEquals(resolved.map((preset) => preset.id), ["sonnet-medium"]);
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});
