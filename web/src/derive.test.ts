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

Deno.test("derive turns empty Codex HTML separators into thought sections only", () => {
  const timeline: Envelope[] = [
    {
      session_id: "s1",
      seq: 1,
      kind: "update",
      update: {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: "**Inspecting setup**\n\n<!--" },
      },
    },
    {
      session_id: "s1",
      seq: 2,
      kind: "update",
      update: {
        sessionUpdate: "agent_thought_chunk",
        content: {
          type: "text",
          text: " -->\n\n<!--\n  \n-->\n\n**Keeping meaningful comments**\n\n<!-- explanation -->",
        },
      },
    },
    {
      session_id: "s1",
      seq: 3,
      kind: "update",
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "Example: <!-- -->" },
      },
    },
  ];

  const [thought, message] = derive(timeline);
  if (thought?.kind !== "thought") throw new Error("expected a thought");
  if (thought.sections.length !== 3) {
    throw new Error(`expected three thought sections, got ${thought.sections.length}`);
  }
  if (thought.sections.some((section) => section.includes("<!-- -->") || section.includes("<!--\n"))) {
    throw new Error("empty thought separators should become section boundaries");
  }
  if (!thought.sections[2]?.includes("<!-- explanation -->")) {
    throw new Error("non-empty thought comments should be preserved");
  }
  if (message?.kind !== "message" || message.chunks[0]?.type !== "text") {
    throw new Error("expected an assistant message");
  }
  if (message.chunks[0].text !== "Example: <!-- -->") {
    throw new Error("normal message content should remain untouched");
  }
});
