import { derive } from "./derive";
import type { Envelope } from "./protocol";

Deno.test("derive coalesces text and memoizes immutable timelines", () => {
  const timeline: Envelope[] = [
    {
      session_id: "s1",
      seq: 1,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "hel" } },
    },
    {
      session_id: "s1",
      seq: 2,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "lo" } },
    },
  ];
  const first = derive(timeline);
  const second = derive(timeline);
  if (first !== second) throw new Error("same immutable timeline should reuse derivation");
  const message = first[0];
  if (message?.kind !== "message" || message.chunks[0]?.type !== "text") {
    throw new Error("expected one message");
  }
  if (message.chunks[0].text !== "hello") throw new Error("chunks were not coalesced");
});
