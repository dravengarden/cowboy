import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { agentLabel } from "./agentTools.ts";
import { providerLabel } from "./managedCalls.ts";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry.ts";
import { providerDisplayName } from "./providerDisplayName.ts";
import { providerUiManifestFixture } from "./providerUiContract.fixture.ts";

function entry(id: string, name: string, version = "1.0.0") {
  const manifest = providerUiManifestFixture();
  manifest.id = id;
  manifest.version = version;
  manifest.display.name = name;
  const artifact = `sha256:${"a".repeat(64)}`;
  return {
    provider_id: id,
    provider_version: version,
    package_digest: artifact,
    artifact_digest: artifact,
    authentication_scope: "none-v1",
    release_state: "ready",
    publisher: manifest.publisher,
    contract_fingerprint: artifact,
    supported_platforms: [{ os: "linux", architecture: "x86_64" }],
    manifest,
  };
}

test("a Provider is named by its signed Plugin when the Catalog carries it", () => {
  const entries = [
    entry("antigravity", "Antigravity"),
    entry("grok", "Grok Build"),
    entry("grok", "Grok Old", "0.9.0"),
  ];
  assertEquals(providerDisplayName("antigravity", entries), "Antigravity");
  // The newest release names it.
  assertEquals(providerDisplayName("grok", entries), "Grok Build");
  assertEquals(providerDisplayName("unknown", entries), undefined);
  assertEquals(providerDisplayName("antigravity", []), undefined);
});

test("labels use the Plugin display name, keep Claude and Codex short and fall back to the id", async () => {
  const previous = globalThis.fetch;
  resetProviderCatalog();
  try {
    // Before the Catalog loads: today's text.
    assertEquals(providerLabel("antigravity"), "antigravity");
    assertEquals(providerLabel("grok"), "grok");
    assertEquals(agentLabel("claude-deepseek"), "Claude Deepseek");

    globalThis.fetch = () =>
      Promise.resolve(Response.json({
        providers: [
          entry("antigravity", "Antigravity"),
          entry("claude-code", "Claude Code"),
          entry("claude-deepseek", "Claude Code · DeepSeek"),
          entry("codex", "Codex"),
        ],
        authentications: [],
        authentication_executors: [],
        platform: { hosts: [] },
      }));
    await loadProviderCatalog();

    assertEquals(providerLabel("antigravity"), "Antigravity");
    assertEquals(providerLabel("claude-deepseek"), "Claude Code · DeepSeek");
    assertEquals(agentLabel("claude-deepseek"), "Claude Code · DeepSeek");
    // The two short names users already know do not change.
    assertEquals(providerLabel("claude-code"), "Claude");
    assertEquals(providerLabel("codex"), "Codex");
    assertEquals(agentLabel("claude-code"), "Claude");
    assertEquals(agentLabel("codex"), "Codex");
    assertEquals(agentLabel("auto"), "Auto");
    // Not in the Catalog: today's text.
    assertEquals(providerLabel("other-agent"), "other-agent");
    assertEquals(agentLabel("other-agent"), "Other Agent");
  } finally {
    globalThis.fetch = previous;
    resetProviderCatalog();
  }
});
