import { assertEquals } from "jsr:@std/assert";
import {
  hasOpenTool,
  hasUnresolvedPermission,
  isCurrentTurnStreamingItem,
  isTurnActivityUpdate,
  QUIET_BADGE_MIN,
  quietMinutes,
  shouldShowQuietBadge,
  WAITING_ELAPSED_VISIBLE_SECONDS,
  waitingActivityLabel,
} from "./turnWaiting.ts";
import { derive } from "./derive.ts";
import type { Envelope } from "./protocol.ts";

Deno.test("Sending never revives a completed assistant bubble before the user echo", () => {
  const completed: Envelope[] = [
    {
      session_id: "s",
      seq: 1,
      kind: "update",
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "Done" },
      },
    },
    { session_id: "s", seq: 2, kind: "turn_end", stop_reason: "end_turn" },
  ];
  const previous = derive(completed).at(-1);
  assertEquals(previous?.kind, "message");
  assertEquals(isCurrentTurnStreamingItem(true, previous, completed), false);
  const timeline: Envelope[] = [...completed, {
    session_id: "s",
    seq: 3,
    kind: "lifecycle",
    status: "busy",
    detail: null,
  }];
  assertEquals(isCurrentTurnStreamingItem(true, previous, timeline), false);
  for (
    const content of [{ type: "text", text: "Next" }, {
      type: "image",
      data: "AA==",
      mimeType: "image/png",
    }]
  ) {
    const echoed: Envelope[] = [...timeline, {
      session_id: "s",
      seq: 4,
      kind: "update",
      update: { sessionUpdate: "user_message_chunk", content },
    }];
    assertEquals(isCurrentTurnStreamingItem(true, previous, echoed), false);
    assertEquals(
      isCurrentTurnStreamingItem(true, derive(echoed).at(-1), echoed),
      false,
    );
    const responding: Envelope[] = [...echoed, {
      session_id: "s",
      seq: 5,
      kind: "update",
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "New reply" },
      },
    }];
    const reply = derive(responding).at(-1);
    assertEquals(isCurrentTurnStreamingItem(true, reply, responding), true);
    assertEquals(isCurrentTurnStreamingItem(false, reply, responding), false);
    for (const stop_reason of ["end_turn", "cancelled", "error: failed"]) {
      assertEquals(
        isCurrentTurnStreamingItem(true, reply, [...responding, {
          session_id: "s",
          seq: 6,
          kind: "turn_end",
          stop_reason,
        }]),
        false,
      );
    }
  }
});

Deno.test("thoughts and retained history respect canonical turn boundaries", () => {
  const thought = { key: "10", kind: "thought" };
  const busy: Envelope = {
    session_id: "s",
    seq: 9,
    kind: "lifecycle",
    status: "busy",
    detail: null,
  };
  assertEquals(isCurrentTurnStreamingItem(true, thought, [busy]), true);
  assertEquals(
    isCurrentTurnStreamingItem(true, thought, [{ ...busy, seq: 11 }]),
    false,
  );
  assertEquals(
    isCurrentTurnStreamingItem(true, thought, [{
      session_id: "s",
      seq: 11,
      kind: "update",
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "Next" },
      },
    }]),
    false,
  );
  assertEquals(isCurrentTurnStreamingItem(true, thought, []), true);
  assertEquals(isCurrentTurnStreamingItem(true, undefined, []), false);
});

Deno.test("agent wait activity names the provider immediately", () => {
  assertEquals(waitingActivityLabel("Grok", 0), "Waiting for Grok…");
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS - 1),
    "Waiting for Grok…",
  );
});

Deno.test("agent wait activity exposes elapsed seconds after a short silence", () => {
  assertEquals(
    waitingActivityLabel("Grok", WAITING_ELAPSED_VISIBLE_SECONDS),
    "Waiting for Grok · 5s",
  );
  assertEquals(waitingActivityLabel("Codex", 53), "Waiting for Codex · 53s");
});

Deno.test("Codex terminal output is turn activity, usage snapshots are not", () => {
  assertEquals(isTurnActivityUpdate("tool_call_update"), true);
  assertEquals(isTurnActivityUpdate("agent_thought_chunk"), true);
  assertEquals(isTurnActivityUpdate("usage_update"), false);
  assertEquals(isTurnActivityUpdate("session_info_update"), false);
  assertEquals(isTurnActivityUpdate("available_commands_update"), false);
});

Deno.test("hasOpenTool detects in-progress tools", () => {
  assertEquals(
    hasOpenTool([{ kind: "thought" }, { kind: "tool", status: "in_progress" }]),
    true,
  );
  assertEquals(
    hasOpenTool([{ kind: "tool", status: "completed" }, { kind: "thought" }]),
    false,
  );
});

Deno.test("a silent pending tool is still quiet; a permission is a human wait", () => {
  assertEquals(
    shouldShowQuietBadge(true, QUIET_BADGE_MIN, false),
    true,
  );
  assertEquals(
    hasUnresolvedPermission([{ kind: "permission", resolved: false }]),
    true,
  );
  assertEquals(
    shouldShowQuietBadge(true, QUIET_BADGE_MIN, true),
    false,
  );
});

Deno.test("quiet badge ignores idle time from before the current turn", () => {
  assertEquals(quietMinutes(10 * 60_000, 0), 10);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN, false), true);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN, true), false);
  assertEquals(shouldShowQuietBadge(false, 56, false), false);
  assertEquals(shouldShowQuietBadge(true, QUIET_BADGE_MIN - 1, false), false);
});

Deno.test("live store treats dropped Codex terminal deltas as turn activity", async () => {
  const storeSource = await Deno.readTextFile(
    new URL("./store.ts", import.meta.url),
  );
  const transcriptSource = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  assertEquals(storeSource.includes("isTurnActivityUpdate"), true);
  assertEquals(storeSource.includes("markSessionTurnActivity"), true);
  assertEquals(storeSource.includes("sessionTurnActivityAt"), true);
  assertEquals(
    transcriptSource.includes("sessionTurnActivityAt(sessionId)"),
    true,
  );
  assertEquals(
    transcriptSource.includes("hasUnresolvedPermission(items)"),
    true,
  );
  assertEquals(
    transcriptSource.includes(
      "shouldShowQuietBadge(working, quietMin, waitingOnHuman)",
    ),
    true,
  );
});
