import { assertEquals } from "jsr:@std/assert";
import type { RenderItem } from "../derive";
import {
  completePageBeforeItem,
  deriveQuestionPages,
  groupQuestionPages,
  pageContainingItemKey,
  questionTitle,
} from "./questionPages";

function user(key: string, text: string, autoResumed = false): RenderItem {
  return {
    key,
    kind: "message",
    role: "user",
    chunks: [{ type: "text", text }],
    autoResumed,
  };
}

function assistant(key: string, text: string): RenderItem {
  return {
    key,
    kind: "message",
    role: "assistant",
    chunks: [{ type: "text", text }],
  };
}

Deno.test("question pages split on human prompts and retain intervening events", () => {
  const pages = deriveQuestionPages([
    user("1", "What is ACP?"),
    { key: "2", kind: "thought", sections: ["Checking"] },
    assistant("3", "ACP is a protocol."),
    user("4", "How does caching work?"),
    { key: "5", kind: "tool", id: "t", title: "Search", toolKind: "search", toolName: "", status: "completed" },
    assistant("6", "It reuses an exact prefix."),
  ]);

  assertEquals(pages.map((page) => page.id), ["1", "4"]);
  assertEquals(pages[0]?.itemKeys, ["1", "2", "3"]);
  assertEquals(pages[1]?.itemKeys, ["4", "5", "6"]);
});

Deno.test("auto-resume user echoes stay in the preceding page", () => {
  const pages = deriveQuestionPages([
    user("1", "Continue the deployment"),
    assistant("2", "Starting."),
    user("3", "Resume interrupted turn", true),
    assistant("4", "Finished."),
  ]);

  assertEquals(pages.length, 1);
  assertEquals(pages[0]?.itemKeys, ["1", "2", "3", "4"]);
});

Deno.test("context management commands do not create question pages", () => {
  const pages = deriveQuestionPages([
    user("1", "A real question"),
    assistant("2", "A real answer"),
    user("3", "/compact"),
    {
      key: "4",
      kind: "tool",
      id: "compact",
      title: "Context compacted",
      toolKind: "other",
      toolName: "",
      status: "completed",
    },
    user("5", "The next real question"),
    assistant("6", "The next answer"),
  ]);

  assertEquals(pages.map((page) => page.id), ["1", "5"]);
  assertEquals(pages[0]?.itemKeys, ["1", "2", "3", "4"]);
});

Deno.test("question title is compact and strips common markdown wrappers", () => {
  assertEquals(
    questionTitle(user("1", "## [Prompt caching](https://example.test)\nDetails"), 1),
    "Prompt caching Details",
  );
});

Deno.test("explicit follow-ups fold into their target page without moving history", () => {
  const base = deriveQuestionPages([
    user("1", "Root question"),
    assistant("2", "Root answer"),
    user("3", "Another topic"),
    assistant("4", "Another answer"),
    user("5", "Follow-up to root"),
    assistant("6", "Follow-up answer"),
  ]);
  const pages = groupQuestionPages(base, { "5": "1" });

  assertEquals(pages.map((page) => page.id), ["1", "3"]);
  assertEquals(pages[0]?.questionCount, 2);
  assertEquals(pages[0]?.itemKeys, ["1", "2", "5", "6"]);
});

Deno.test("a canonical transcript row resolves to its owning question page", () => {
  const pages = deriveQuestionPages([
    user("question-1", "First"),
    assistant("answer-1", "Answer one"),
    user("question-2", "Second"),
    assistant("answer-2", "Answer two"),
  ]);

  assertEquals(pageContainingItemKey(pages, "answer-1")?.id, "question-1");
  assertEquals(pageContainingItemKey(pages, "question-2")?.id, "question-2");
  assertEquals(pageContainingItemKey(pages, "missing"), undefined);
});

Deno.test("a partial history tail keeps leading answer rows addressable", () => {
  const pages = deriveQuestionPages([
    assistant("answer-tail", "The root prompt is on an older history page"),
    user("next-question", "Next"),
    assistant("next-answer", "Next answer"),
  ]);

  assertEquals(pages[0]?.title, "Earlier question");
  assertEquals(pages[0]?.itemKeys, ["answer-tail"]);
  assertEquals(pageContainingItemKey(pages, "answer-tail")?.id, "answer-tail");
});

Deno.test("previous navigation waits for the real user question boundary", () => {
  const partial = deriveQuestionPages([
    assistant("answer-tail", "The question is on the next history batch"),
    user("current-question", "Current"),
    assistant("current-answer", "Current answer"),
  ]);
  assertEquals(
    completePageBeforeItem(partial, "current-answer"),
    undefined,
  );

  const complete = deriveQuestionPages([
    user("previous-question", "Previous"),
    assistant("answer-tail", "The question is now loaded"),
    user("current-question", "Current"),
    assistant("current-answer", "Current answer"),
  ]);
  assertEquals(
    completePageBeforeItem(complete, "current-answer")?.id,
    "previous-question",
  );
});
