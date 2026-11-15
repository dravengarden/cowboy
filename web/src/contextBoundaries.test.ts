import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert";
import { contextBoundaryItem } from "./contextBoundaries";
import { contextRolloverCaption, type ContextRolloverView } from "./ContextRolloverSettings";
import { derive } from "./derive";
import type { Envelope } from "./protocol";

test("a renewal keeps earlier rows and adds a fresh-context notice", () => {
  const timeline = [
    {
      kind: "update",
      seq: 1,
      update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: "old ask" } },
    },
    {
      kind: "update",
      seq: 2,
      update: {
        sessionUpdate: "context_rolled_over",
        at: 5,
        idleMs: 7_200_000,
        contextUsed: 250_000,
      },
    },
    {
      kind: "update",
      seq: 3,
      update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: "new ask" } },
    },
  ] as unknown as Envelope[];
  const items = derive(timeline);
  const notice = items.find((item) => item.kind === "notice");
  assert(notice && notice.kind === "notice");
  assertEquals(notice.title, "Fresh context");
  assert(notice.description?.includes("2.0 h idle"));
  assert(notice.description?.includes("250k tokens"));
  assertEquals(items.filter((item) => item.kind === "message").length, 2);
});

test("context boundaries keep fork and clear rows", () => {
  assertEquals(
    contextBoundaryItem({ sessionUpdate: "context_cleared", at: 9 }, "k")?.kind,
    "cleared",
  );
  assertEquals(contextBoundaryItem({ sessionUpdate: "cowboy_fork_origin" }, "k"), null);
  assertEquals(
    contextBoundaryItem(
      { sessionUpdate: "cowboy_fork_origin", sourceSessionId: "s", messageId: "m" },
      "k",
    ),
    { kind: "forked", key: "k", sourceSessionId: "s", sourceTitle: "", atMessage: true },
  );
});

test("the renewal caption explains the threshold, the off state and a due renewal", () => {
  const view: ContextRolloverView = {
    policy: { enabled: true, session_override: null, idle_minutes: 60, min_context_tokens: 80_000 },
    context_used: 1,
    idle_ms: 0,
    due: false,
  };
  assert(
    contextRolloverCaption(view).startsWith("After 1 h idle, when the context holds 80k+ tokens"),
  );
  assert(
    contextRolloverCaption({ ...view, policy: { ...view.policy, enabled: false } }).startsWith(
      "Off:",
    ),
  );
  assert(
    contextRolloverCaption({ ...view, due: true }).includes("next message starts a fresh context"),
  );
});
