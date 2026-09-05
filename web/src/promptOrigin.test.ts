import { assertEquals } from "jsr:@std/assert";
import {
  HUMAN_COMPOSER_ORIGIN,
  isAgentReviewTaskPrompt,
  isHumanPrompt,
  isInternalRuntimePrompt,
  resolvePromptOrigin,
  runtimePromptPresentation,
} from "./promptOrigin";

const AGENT_REVIEW_FOLLOW_UP =
  `The reviewer found issues. The review_file is at: /tmp/agent-1000/design-review-d58766af.md

Read the review_file. Address ALL issues with Status: open -- including nits.

For each issue, revise /tmp/agent-1000/design-doc-d58766af.md
Then update the review_file:
- Status: open -> addressed
- Add a Response field

You may set wontfix with a technical explanation, or needs-user-input if the user must decide.`;

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
  assertEquals(isAgentReviewTaskPrompt(AGENT_REVIEW_FOLLOW_UP), true);
  assertEquals(isInternalRuntimePrompt(AGENT_REVIEW_FOLLOW_UP), true);
  assertEquals(
    resolvePromptOrigin({}, AGENT_REVIEW_FOLLOW_UP),
    { actor: "agent", source: "review" },
  );
  assertEquals(isAgentReviewTaskPrompt("Read the review_file please"), false);
  assertEquals(
    isAgentReviewTaskPrompt("The reviewer found issues in my PR"),
    false,
  );
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

Deno.test("agent review follow-ups collapse to a note instead of a human bubble", () => {
  const presented = runtimePromptPresentation(
    AGENT_REVIEW_FOLLOW_UP,
    { actor: "agent", source: "review", provider: "future-agent" },
  );
  assertEquals(presented.title, "Addressing review findings");
  assertEquals(presented.raw?.includes("/tmp/agent-1000/"), true);
});
