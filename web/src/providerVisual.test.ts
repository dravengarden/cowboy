import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import { providerVisual } from "./providerVisual.ts";

Deno.test("DeepSeek runtimes share a provider primary but retain agent identity", () => {
  const claude = providerVisual("claude-deepseek", "dark");
  const codex = providerVisual("codex-deepseek", "dark");
  const reasonix = providerVisual("reasonix-deepseek", "dark");

  assertEquals(claude.primary, codex.primary);
  assertEquals(codex.primary, reasonix.primary);
  assertNotEquals(claude.secondary, codex.secondary);
  assertNotEquals(codex.secondary, reasonix.secondary);
  assertNotEquals(
    claude.primary,
    providerVisual("claude-code", "dark").primary,
  );
});
