import { assertEquals } from "jsr:@std/assert";
import { derive, linkTimeline } from "./derive";
import { prettifyCrashDetail, rpcErrorKind } from "./crashDetail";
import type { AcpUpdate, Envelope } from "./protocol";

const message =
  "API Error: Connection lost mid-response. The response above may be incomplete.";
const detail =
  `Internal error: ${message}: {\n  "errorKind": "server_error"\n}`;
const update = (seq: number, value: AcpUpdate): Envelope => ({
  session_id: "stream-fixture",
  seq,
  kind: "update",
  update: value,
});
const text = (seq: number, value: string, messageId = "diagnostic"): Envelope =>
  update(seq, {
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text: value },
    messageId,
  });
const end = (seq: number, stop_reason: string): Envelope => ({
  session_id: "stream-fixture",
  seq,
  kind: "turn_end",
  stop_reason,
});
const fixture: Envelope[] = [
  update(1, {
    sessionUpdate: "tool_call",
    toolCallId: "saved",
    title: "Write file",
    status: "completed",
  }),
  text(2, "The file was written.", "answer"),
  update(3, {
    sessionUpdate: "tool_call",
    toolCallId: "partial",
    title: "Terminal",
  }),
  text(4, message),
];

Deno.test("partial stream failure keeps completed work and one actionable diagnostic", () => {
  const timeline = [...fixture, end(5, `error: ${detail}`), {
    session_id: "stream-fixture",
    seq: 6,
    kind: "lifecycle",
    status: "running",
    detail,
  } as Envelope];
  const items = derive(timeline);
  assertEquals(items.map((item) => item.kind), [
    "tool",
    "message",
    "tool",
    "lifecycle",
  ]);
  assertEquals(
    items.filter((item) => item.kind === "tool").map((item) => item.status),
    ["completed", "interrupted"],
  );
  const error = items.at(-1);
  assertEquals(error?.kind === "lifecycle" && error.turnFailure, true);
  assertEquals(error?.kind === "lifecycle" && error.detail, detail);
  assertEquals(error?.kind === "lifecycle" && error.status, "interrupted");
  assertEquals(prettifyCrashDetail(detail), message);
  // Derivation must not rewrite the persisted evidence or prior render state.
  assertEquals(fixture[3]?.kind === "update" && fixture[3].update.content, {
    type: "text",
    text: message,
  });
  const unfinished = derive(fixture)[2];
  assertEquals(unfinished?.kind === "tool" && unfinished.status, "pending");
});

Deno.test("legacy RPC error plus crashed lifecycle folds into the same failure", () => {
  const items = derive([...fixture, end(5, `error: ${detail}`), {
    session_id: "stream-fixture",
    seq: 6,
    kind: "lifecycle",
    status: "crashed",
    detail,
  }]);
  assertEquals(items.filter((item) => item.kind === "lifecycle").length, 1);
  const error = items.at(-1);
  assertEquals(error?.kind === "lifecycle" && error.status, "crashed");
  assertEquals(error?.kind === "lifecycle" && error.turnFailure, true);
});

Deno.test("error coalescing preserves earlier prose and requires a real structured failure", () => {
  const items = derive([
    text(1, "Saved progress. ", "answer"),
    text(2, message),
    end(3, `error: ${detail}`),
  ]);
  assertEquals(items.length, 2);
  assertEquals(items[0]?.kind === "message" && items[0].chunks, [{
    type: "text",
    text: "Saved progress. ",
  }]);
  for (
    const diagnostic of [message, `The error was: ${message}`, `\`${message}\``]
  ) {
    const success = derive([text(1, diagnostic), end(2, "EndTurn")]);
    assertEquals(success[0]?.kind === "message" && success[0].chunks, [{
      type: "text",
      text: diagnostic,
    }]);
  }
  const quoted = derive([
    text(1, `The error was: ${message}`),
    end(2, `error: ${detail}`),
  ]);
  assertEquals(quoted.length, 2);
  const unstructured = derive([text(1, message), end(2, `error: ${message}`)]);
  assertEquals(unstructured.length, 2);
});

Deno.test("cancel and process loss never infer tool success; a late result remains authoritative", () => {
  for (
    const terminal of [end(5, "Cancelled"), {
      session_id: "stream-fixture",
      seq: 5,
      kind: "lifecycle",
      status: "crashed",
      detail: "process exited",
    } as Envelope]
  ) {
    const timeline = [...fixture, terminal];
    const items = derive(timeline);
    assertEquals(items[2]?.kind === "tool" && items[2].status, "interrupted");
    const continued = linkTimeline([
      ...timeline,
      update(6, {
        sessionUpdate: "tool_call_update",
        toolCallId: "partial",
        status: "completed",
        rawOutput: "confirmed",
      }),
    ], timeline);
    const next = derive(continued);
    assertEquals(next[2]?.kind === "tool" && next[2].status, "completed");
    assertEquals(next[0] === items[0], true);
    assertEquals(items[2]?.kind === "tool" && items[2].status, "interrupted");
  }
});

Deno.test("error metadata remains diagnostic data and malformed metadata stays visible", () => {
  assertEquals(rpcErrorKind(detail), "server_error");
  assertEquals(rpcErrorKind(`${message}: {"errorKind": false}`), null);
  assertEquals(rpcErrorKind(`${message}: {"errorKind": "server_error"`), null);
  assertEquals(
    prettifyCrashDetail(`${message}: {broken}`),
    `${message}: {broken}`,
  );
});

Deno.test("bare transport errors cannot settle a pending tool as successful", () => {
  for (const stopReason of ["Error", "error:"]) {
    const timeline: Envelope[] = [
      ...fixture.slice(0, 3),
      end(5, stopReason),
      {
        session_id: "stream-fixture",
        seq: 6,
        kind: "lifecycle",
        status: "crashed",
        detail: "ACP connection closed",
      },
    ];
    const items = derive(timeline);
    assertEquals(items[2]?.kind === "tool" && items[2].status, "interrupted");
    assertEquals(items.filter((item) => item.kind === "lifecycle").length, 1);
    const error = items.at(-1);
    assertEquals(
      error?.kind === "lifecycle" && error.detail,
      "ACP connection closed",
    );
  }
});

Deno.test("model refusal interrupts pending tools without offering transport continuation", () => {
  for (const reason of ["Refusal", "refusal"]) {
    const diagnostic = "API Error: safeguards flagged this message. Details: [reasoning_extraction]";
    const items = derive([
      ...fixture.slice(0, 3), text(4, diagnostic), end(5, reason),
    ]);
    assertEquals(items[0]?.kind === "tool" && items[0].status, "completed");
    assertEquals(items[2]?.kind === "tool" && items[2].status, "interrupted");
    assertEquals(items[3]?.kind === "message" && items[3].chunks, [{ type: "text", text: diagnostic }]);
    assertEquals(items.some((item) => item.kind === "lifecycle" && item.turnFailure), false);
    // Refusal-like prose on a successful turn remains ordinary output.
    const success = derive([...fixture.slice(0, 3), text(4, diagnostic), end(5, "EndTurn")]);
    assertEquals(success[2]?.kind === "tool" && success[2].status, "completed");
  }
});
