import type { Event } from "./protocol";

// Field-level fixtures complement the Rust/TypeScript discriminant test. The
// compiler checks field names, nesting and nullability here; Rust serializes
// the same shapes in protocol_contract.rs.
const EVENTS = [
  { kind: "update", update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "hello" } } },
  { kind: "permission_request", request_id: "p1", tool_call: { title: "Shell" }, options: [] },
  { kind: "permission_resolved", request_id: "p1", option_id: null },
  { kind: "lifecycle", status: "running", detail: null },
  { kind: "turn_end", stop_reason: "end_turn" },
] satisfies Event[];

Deno.test("event contract fixtures retain every field", () => {
  if (EVENTS.length !== 5) throw new Error("event fixture count drifted");
  if (EVENTS[1].request_id !== "p1") throw new Error("permission field drifted");
});
