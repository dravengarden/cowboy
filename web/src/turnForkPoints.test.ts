import { expect, test } from "bun:test";
import { COMPACTING_NOTICE, derive, forkableAnswerKeys } from "./derive";
import { buildSettingsForkAttempt } from "./ForkPointPicker";
import type { Envelope } from "./protocol";
import { requestNewFrom, takeNewFromRequestDetail } from "./sessionSettingsOpen";
import {
  completedTurnForkPointMap,
  extractCompletedTurnForkPoints,
  formatTurnForkLabel,
  latestContextClearSeq,
} from "./turnForkPoints";

let seq = 0;
function update(
  sessionUpdate: string,
  text: string,
  fields: Record<string, unknown> = {},
): Envelope {
  seq += 1;
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: {
      sessionUpdate,
      content: { type: "text", text },
      ...fields,
    },
  } as Envelope;
}

function clearedEnvelope(): Envelope {
  seq += 1;
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: { sessionUpdate: "context_cleared", at: seq * 1000 },
  } as Envelope;
}

test("running turn intermediate commentary blocks are never offered as fork points", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk", "Fix scroll anchoring"),
    update("agent_message_chunk", "Checking Transcript.tsx", { messageId: "t1-a1" }),
    update("tool_call", "", { toolCallId: "tc1", title: "Read", status: "completed" }),
    update("agent_message_chunk", "Scroll anchoring is fixed.", { messageId: "t1-a2" }),
    update("user_message_chunk", "Now run the e2e self-test"),
    update("agent_message_chunk", "Filter state is recorded.", { messageId: "t2-a1" }),
    update("tool_call", "", { toolCallId: "tc2", title: "Run", status: "completed" }),
    update("agent_message_chunk", "Re-run the scratch dry runs.", { messageId: "t2-a2" }),
    update("tool_call", "", { toolCallId: "tc3", title: "Run", status: "completed" }),
    update("agent_message_chunk", "Extending the self-test now.", { messageId: "t2-a3" }),
    update("tool_call", "", { toolCallId: "tc4", title: "Terminal", status: "pending" }),
  ]);

  // While Turn 2 is busy: ONLY Turn 1's final answer (t1-a2) is forkable.
  // None of Turn 1's commentary (t1-a1) or Turn 2's three commentary bubbles qualify.
  const whileBusy = extractCompletedTurnForkPoints(items, true);
  expect(whileBusy).toHaveLength(1);
  expect(whileBusy[0]?.messageId).toBe("t1-a2");
  expect(whileBusy[0]?.turnNumber).toBe(1);
  expect(whileBusy[0]?.promptPreview).toBe("Fix scroll anchoring");
  expect(whileBusy[0]?.answerPreview).toBe("Scroll anchoring is fixed.");
  expect(whileBusy[0]?.beforeSeq).toBe(5);

  // Once Turn 2 finishes (idle): Turn 2's final answer (t2-a3) becomes forkable,
  // still excluding intermediate bubbles t2-a1 and t2-a2.
  const whenIdle = extractCompletedTurnForkPoints(items, false);
  expect(whenIdle.map((turn) => turn.messageId)).toEqual(["t1-a2", "t2-a3"]);
  expect(whenIdle[1]?.turnNumber).toBe(2);
  expect(formatTurnForkLabel(whenIdle[1]!)).toBe("Turn 2 · Now run the e2e self-test");
});

test("steered prompts stay in the same turn and cleared boundaries discard older turns", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk", "Old turn before clear"),
    update("agent_message_chunk", "Old answer", { messageId: "old-1" }),
    clearedEnvelope(),
    update("user_message_chunk", "Fresh start"),
    update("agent_message_chunk", "Initial thought", { messageId: "new-1" }),
    update("user_message_chunk", "Also check dark mode", {
      _meta: { cowboyDelivery: "current_turn" },
    }),
    update("agent_message_chunk", "Verified light and dark.", { messageId: "new-2" }),
  ]);

  expect(latestContextClearSeq(items)).toBe(3);
  const turns = extractCompletedTurnForkPoints(items, false);
  expect(turns).toHaveLength(1);
  expect(turns[0]?.turnNumber).toBe(1);
  expect(turns[0]?.messageId).toBe("new-2");
  expect(turns[0]?.promptPreview).toBe("Fresh start");

  // derive.ts::forkableAnswerKeys also drops pre-clear turns.
  expect([...forkableAnswerKeys(items, false).values()]).toEqual(["new-2"]);
});

test("compaction notices are never treated as forkable answers", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk", "Summarize the diff"),
    update("agent_message_chunk", "Summary complete.", { messageId: "ans-1" }),
    update("user_message_chunk", "/compact"),
    update("agent_message_chunk", COMPACTING_NOTICE, { messageId: "cmp-1" }),
    update("agent_message_chunk", "Context compacted.", { messageId: "cmp-2" }),
  ]);

  const turns = extractCompletedTurnForkPoints(items, false);
  expect(turns.map((turn) => turn.messageId)).toEqual(["ans-1"]);
  expect([...forkableAnswerKeys(items, false).values()]).toEqual(["ans-1"]);
});

test("linked-only providers without messageId still produce turn-final beforeSeq points", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk", "First prompt"),
    update("agent_message_chunk", "Intermediate note"),
    update("tool_call", "", { toolCallId: "t1", title: "Search", status: "completed" }),
    update("agent_message_chunk", "Final answer"),
  ]);

  const turns = extractCompletedTurnForkPoints(items, false);
  expect(turns).toHaveLength(1);
  expect(turns[0]?.messageId).toBeUndefined();
  expect(turns[0]?.beforeSeq).toBe(5);
  expect(turns[0]?.answerPreview).toBe("Final answer");
});

test("completedTurnForkPointMap caches per items array and busy state for stable React.memo identity", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk", "Hello"),
    update("agent_message_chunk", "World", { messageId: "m1" }),
  ]);
  const a = completedTurnForkPointMap(items, false);
  const b = completedTurnForkPointMap(items, false);
  const c = completedTurnForkPointMap(items, true);
  expect(a).toBe(b);
  expect(a.size).toBe(1);
  expect(c.size).toBe(0);
});

test("requestNewFrom passes optional ForkPointSelection cleanly to takeNewFromRequestDetail", () => {
  const originalDocument = globalThis.document;
  const dispatched: string[] = [];
  (globalThis as unknown as { document: unknown }).document = {
    dispatchEvent(event: Event): boolean {
      dispatched.push(event.type);
      return true;
    },
  };
  try {
    const selection = {
      itemKey: "42",
      messageId: "msg-42",
      beforeSeq: 43,
      turnNumber: 3,
      promptPreview: "Refactor scroll anchor",
    };
    requestNewFrom("sess-turn", selection);
    expect(dispatched).toEqual(["cowboy:session-new-from", "cowboy:open-session-settings"]);
    expect(takeNewFromRequestDetail("sess-turn")).toEqual({
      matched: true,
      forkPoint: selection,
    });
    // Consumed once.
    expect(takeNewFromRequestDetail("sess-turn")).toEqual({ matched: false });
  } finally {
    (globalThis as unknown as { document: unknown }).document = originalDocument;
  }
});

test("buildSettingsForkAttempt carries messageId for native forks and before_seq for linked forks", () => {
  const nativeAttempt = buildSettingsForkAttempt({
    forkMode: "fork",
    provider: "claude-code",
    account: "default",
    preset: undefined,
    inheritSettings: true,
    overrides: { model: "opus" },
    effectiveIncludeUncommitted: undefined,
    forkPoint: { messageId: "msg-7", beforeSeq: 18 },
  });
  expect(nativeAttempt.mode).toBe("fork");
  if (nativeAttempt.mode === "fork") {
    expect(nativeAttempt.messageId).toBe("msg-7");
    expect(nativeAttempt.config).toEqual({ model: "opus" });
  }

  const linkedAttempt = buildSettingsForkAttempt({
    forkMode: "linked",
    provider: "codex",
    account: "default",
    preset: undefined,
    inheritSettings: true,
    overrides: {},
    effectiveIncludeUncommitted: false,
    forkPoint: { beforeSeq: 18 },
  });
  expect(linkedAttempt.mode).toBe("linked");
  if (linkedAttempt.mode === "linked") {
    expect(linkedAttempt.body.before_seq).toBe(18);
    expect(linkedAttempt.body.include_uncommitted).toBe(false);
  }
});
