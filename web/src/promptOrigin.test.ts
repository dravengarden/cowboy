import { assertEquals } from "jsr:@std/assert";
import {
  HUMAN_COMPOSER_ORIGIN,
  isHumanPrompt,
  isInternalRuntimePrompt,
  resolvePromptOrigin,
  runtimePromptPresentation,
} from "./promptOrigin";

Deno.test("internal runtime prompts are not human text", () => {
  assertEquals(
    isInternalRuntimePrompt(
      "<system-reminder>Background task completed.</system-reminder>",
    ),
    true,
  );
  assertEquals(
    isInternalRuntimePrompt("<system-reminder>Background task still running"),
    true,
  );
  assertEquals(
    isInternalRuntimePrompt("Please do not leak <system-reminder> tags"),
    false,
  );
});

Deno.test("resolvePromptOrigin prefers the persisted source object", () => {
  assertEquals(
    resolvePromptOrigin(
      {
        autoResumed: true,
        promptOrigin: { actor: "agent", source: "runtime", provider: "grok" },
      },
      "hello",
    ),
    { actor: "agent", source: "runtime", provider: "grok" },
  );
  assertEquals(
    resolvePromptOrigin({ autoResumed: true }, "resume this"),
    { actor: "cowboy", source: "auto-resume" },
  );
  assertEquals(
    resolvePromptOrigin({}, "<system-reminder>done</system-reminder>"),
    { actor: "agent", source: "runtime" },
  );
  assertEquals(resolvePromptOrigin({}, "Ship it"), HUMAN_COMPOSER_ORIGIN);
  assertEquals(isHumanPrompt(HUMAN_COMPOSER_ORIGIN), true);
  assertEquals(isHumanPrompt({ actor: "agent", source: "runtime" }), false);
});

Deno.test("runtime presentation hides reminder markup from the title", () => {
  const presented = runtimePromptPresentation(
    '<system-reminder>Background task "find" completed (exit code: 0).\nCommand: find /tmp</system-reminder>',
    { actor: "agent", source: "runtime", provider: "grok" },
  );
  assertEquals(presented.title, "Background task completed");
  assertEquals(presented.raw?.includes("Command: find /tmp"), true);
  assertEquals(presented.raw?.includes("<system-reminder>"), false);
});
