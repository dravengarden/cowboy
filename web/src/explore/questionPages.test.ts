import { assertEquals } from "jsr:@std/assert";
import type { RenderItem } from "../derive";
import {
  authoritativeTailPageId,
  completePageBeforeItem,
  deriveQuestionPages,
  groupQuestionPages,
  indexedQuestionPagePosition,
  mergeQuestionPageDirectory,
  pageContainingItemKey,
  presentQuestionPageDirectory,
  questionTitle,
} from "./questionPages";

const exploreSurfaceSource = await Deno.readTextFile(
  new URL("./ExploreSurface.tsx", import.meta.url),
);

Deno.test("mobile Page Dock retains disabled previous and next slots", () => {
  assertEquals(exploreSurfaceSource.includes("onlyCompletePage"), false);
  assertEquals(exploreSurfaceSource.includes('aria-label="Next page"'), true);
  assertEquals(
    exploreSurfaceSource.includes('? "Load earlier questions"\n                      : "Previous page"'),
    true,
  );
});

Deno.test("question directory merges live pages and sorts oldest to newest", () => {
  assertEquals(
    mergeQuestionPageDirectory(
      [
        { id: "20", title: "Second", ordinal: 2 },
        { id: "10", title: "First", ordinal: 1 },
      ],
      [
        { id: "20", title: "Hydrated second" },
        { id: "30", title: "Live third" },
      ],
      3,
    ),
    [
      { id: "10", title: "First", ordinal: 1 },
      { id: "20", title: "Second", ordinal: 2 },
      { id: "30", title: "Live third", ordinal: 3 },
    ],
  );
});

Deno.test("desktop question navigator presents newest first without mutating chronology", () => {
  const chronological = [
    { id: "1", ordinal: 1 },
    { id: "2", ordinal: 2 },
    { id: "3", ordinal: 3 },
  ];
  assertEquals(
    presentQuestionPageDirectory(chronological, true).map((page) => page.id),
    ["3", "2", "1"],
  );
  assertEquals(chronological.map((page) => page.id), ["1", "2", "3"]);
});

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
  const longTitle = questionTitle(user("2", "A".repeat(100)), 2);
  assertEquals(longTitle.length, 70);
  assertEquals(longTitle, `${"A".repeat(69)}…`);
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

Deno.test("a completed provisional tail hydrates from the authoritative question root", () => {
  const provisional = deriveQuestionPages([
    assistant("188841", "Only the retained end of a long answer"),
  ])[0]!;
  const index = [
    { id: "180000", ordinal: 98 },
    { id: "186012", ordinal: 99 },
  ];

  assertEquals(
    authoritativeTailPageId(provisional, true, index),
    "186012",
  );
  assertEquals(authoritativeTailPageId(provisional, false, index), null);
  assertEquals(
    authoritativeTailPageId(
      { id: "186012", questionCount: 1 },
      true,
      index,
    ),
    null,
  );
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

Deno.test("authoritative page index survives a sparse loaded content window", () => {
  const index = [
    { id: "28", ordinal: 28 },
    { id: "29", ordinal: 29 },
    { id: "30", ordinal: 30 },
    { id: "31", ordinal: 31 },
    { id: "32", ordinal: 32 },
    { id: "33", ordinal: 33 },
    { id: "34", ordinal: 34 },
    { id: "35", ordinal: 35 },
    { id: "36", ordinal: 36 },
  ];

  // The content cache may currently contain only 28-31 and 36. Its array
  // position cannot be used to infer either the ordinal or the next page.
  assertEquals(indexedQuestionPagePosition(index, "31"), {
    ordinal: 31,
    previousId: "30",
    nextId: "32",
  });
});

Deno.test("authoritative page index falls back from a provisional id to its ordinal", () => {
  const index = [
    { id: "30", ordinal: 1 },
    { id: "31", ordinal: 2 },
  ];

  assertEquals(indexedQuestionPagePosition(index, "provisional", 2), {
    ordinal: 2,
    previousId: "30",
    nextId: undefined,
  });
});
