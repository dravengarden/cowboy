import { assert } from "jsr:@std/assert";
import {
  desktopTopBarTimelineSlice,
  sameDesktopTopBarTimelineSlice,
} from "./desktopTopBarTimelineSlice";
import type { Envelope } from "../protocol";

const update = (seq: number, sessionUpdate: string, extra = {}): Envelope => ({
  session_id: "s",
  seq,
  kind: "update",
  update: { sessionUpdate, ...extra },
});

Deno.test("desktop top bar ignores ordinary streaming transcript churn", () => {
  const base: Envelope[] = [
    update(1, "available_commands_update", {
      availableCommands: [{ name: "compact", description: "Compact" }],
    }),
  ];
  const before = desktopTopBarTimelineSlice(base);
  const after = desktopTopBarTimelineSlice([
    ...base,
    update(2, "agent_message_chunk", {
      messageId: "answer",
      content: { type: "text", text: "streaming" },
    }),
    update(3, "tool_call", { toolCallId: "tool", title: "Inspect" }),
  ]);
  assert(sameDesktopTopBarTimelineSlice(before, after));
});

Deno.test("desktop top bar reacts to command and compaction signals", () => {
  const initial = desktopTopBarTimelineSlice([
    update(1, "available_commands_update", { availableCommands: [] }),
  ]);
  const commands = desktopTopBarTimelineSlice([
    update(2, "available_commands_update", {
      availableCommands: [{ name: "compact", description: "Compact" }],
    }),
  ]);
  const compacting = desktopTopBarTimelineSlice([
    update(3, "user_message_chunk", {
      content: { type: "text", text: "/compact" },
    }),
  ]);
  assert(!sameDesktopTopBarTimelineSlice(initial, commands));
  assert(!sameDesktopTopBarTimelineSlice(initial, compacting));
});
