import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import { providerVisual } from "./providerVisual.ts";

Deno.test("DeepSeek runtimes share a provider primary but retain agent identity", () => {
  const claude = providerVisual("claude-deepseek", "dark");
  const codex = providerVisual("codex-deepseek", "dark");

  assertEquals(claude.primary, codex.primary);
  assertNotEquals(claude.secondary, codex.secondary);
  assertNotEquals(
    claude.primary,
    providerVisual("claude-code", "dark").primary,
  );
});

Deno.test("Grok has a distinct monochrome xAI visual", () => {
  const grok = providerVisual("grok", "dark");
  assertNotEquals(grok.primary, providerVisual("unknown", "dark").primary);
  assertNotEquals(grok.primary, grok.secondary);
});
