import { assertEquals } from "jsr:@std/assert";
import { defaultNewSessionProvider } from "./newSessionProvider.ts";

Deno.test("new sessions prefer standard Claude Code when it is available", () => {
  assertEquals(
    defaultNewSessionProvider([
      "claude-deepseek",
      "codex",
      "claude-code",
      "codex-deepseek",
    ]),
    "claude-code",
  );
});

Deno.test("new sessions fall back to standard Codex without Claude Code", () => {
  assertEquals(
    defaultNewSessionProvider([
      "claude-deepseek",
      "codex",
      "codex-deepseek",
    ]),
    "codex",
  );
});

Deno.test("new sessions fall back to Machine inventory order without a standard Provider", () => {
  assertEquals(
    defaultNewSessionProvider(["claude-deepseek", "grok"]),
    "claude-deepseek",
  );
  assertEquals(defaultNewSessionProvider([]), "");
});
