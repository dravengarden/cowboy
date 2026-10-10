import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { defaultNewSessionProvider } from "./newSessionProvider.ts";

test("new sessions prefer standard Claude Code when it is available", () => {
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

test("new sessions fall back to standard Codex without Claude Code", () => {
  assertEquals(
    defaultNewSessionProvider([
      "claude-deepseek",
      "codex",
      "codex-deepseek",
    ]),
    "codex",
  );
});

test("new sessions fall back to Machine inventory order without a standard Provider", () => {
  assertEquals(
    defaultNewSessionProvider(["claude-deepseek", "grok"]),
    "claude-deepseek",
  );
  assertEquals(defaultNewSessionProvider([]), "");
});
