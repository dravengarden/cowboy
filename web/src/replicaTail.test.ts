import { assert, assertEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import { replicaTailConflicts, trimReplicaTail } from "./replicaTail.ts";

function chunk(seq: number, text = "x"): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } },
  };
}

function tool(seq: number, toolCallId: string): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "tool_call", toolCallId, title: "t" },
  };
}

Deno.test("trimReplicaTail keeps the newest events within both bounds", () => {
  const events = [chunk(1), chunk(2), chunk(3), chunk(4)];
  assertEquals(trimReplicaTail(events, { maxEvents: 2 }).map((e) => e.seq), [3, 4]);
  const big = chunk(2, "y".repeat(500));
  const trimmed = trimReplicaTail([chunk(1), big, chunk(3)], { maxBytes: 400 });
  assertEquals(trimmed.map((e) => e.seq), [3]);
  assertEquals(trimReplicaTail([], {}), []);
  // An oversized newest event yields an empty tail rather than a torn one.
  assertEquals(trimReplicaTail([big], { maxBytes: 100 }), []);
});

Deno.test("replicaTailConflicts detects a restarted transcript epoch", () => {
  const cached = [chunk(10), chunk(11), tool(12, "a")];
  assert(!replicaTailConflicts(cached, [chunk(11), tool(12, "a"), chunk(13)]));
  assert(!replicaTailConflicts(cached, [chunk(40), chunk(41)]));
  assert(!replicaTailConflicts([], [chunk(1)]));
  assert(!replicaTailConflicts(cached, []));
  // Same seq, different event: the conversation was cleared elsewhere.
  assert(replicaTailConflicts(cached, [tool(11, "b"), chunk(12)]));
  // The fresh run ends before the cached one: seqs restarted.
  assert(replicaTailConflicts(cached, [chunk(1), chunk(2)]));
});
