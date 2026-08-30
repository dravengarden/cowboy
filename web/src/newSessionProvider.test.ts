import { assertEquals } from "jsr:@std/assert";
import { defaultNewSessionProvider } from "./newSessionProvider.ts";

Deno.test("new sessions prefer standard Codex when it is available", () => {
  assertEquals(
    defaultNewSessionProvider([
      "claude-deepseek",
      "codex",
      "codex-deepseek",
    ]),
    "codex",
  );
});

Deno.test("new sessions fall back to Machine inventory order without Codex", () => {
  assertEquals(
    defaultNewSessionProvider(["claude-deepseek", "grok"]),
    "claude-deepseek",
  );
  assertEquals(defaultNewSessionProvider([]), "");
});
