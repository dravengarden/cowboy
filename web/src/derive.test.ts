import {
  derive,
  isCompactionCompletionTail,
  isCompactingTail,
  latestCompactionCompletionSeq,
  linkTimeline,
} from "./derive";
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

Deno.test("structured agent questions are not presented as tool approvals", () => {
  const timeline: Envelope[] = [{
    session_id: "s1",
    seq: 1,
    kind: "permission_request",
    request_id: "q1",
    tool_call: {
      title: "Choose how to continue",
      rawInput: {
        question: "How should I continue?",
        options: [{ Label: "Continue" }],
      },
    },
    options: [],
  }];

  const item = derive(timeline)[0];
  if (item?.kind !== "permission" || item.requestKind !== "question") {
    throw new Error("structured question should render as an answer request");
  }
});

Deno.test("derive collapses repeated terminal lifecycle projections", () => {
  const detail = "Gemini personal access retired";
  const items = derive([detail, null, null].map((projectedDetail, index): Envelope => ({
    session_id: "gemini-session",
    seq: index + 1,
    kind: "lifecycle",
    status: "crashed",
    detail: projectedDetail,
  })));
  if (items.length !== 1 || items[0]?.kind !== "lifecycle") {
    throw new Error("one crash projected through multiple layers should render once");
  }
  if (items[0].detail !== detail || items[0].key !== "3") {
    throw new Error("the collapsed crash should retain the latest identity");
  }
});

Deno.test("Codex compact command and completion drive the compact state", () => {
  const requested: Envelope[] = [{
    session_id: "s1",
    seq: 10,
    kind: "update",
    update: {
      sessionUpdate: "user_message_chunk",
      content: { type: "text", text: "/compact" },
    },
  }];
  if (!isCompactingTail(requested)) throw new Error("compact request should be active");

  const completed: Envelope[] = [...requested, {
    session_id: "s1",
    seq: 11,
    kind: "update",
    update: {
      sessionUpdate: "agent_message_chunk",
      content: {
        type: "text",
        text: "*Context compacted to fit the model's context window.*\n\n",
      },
    },
  }];
  if (isCompactingTail(completed)) throw new Error("completed compact should not stay active");
  if (!isCompactionCompletionTail(completed)) {
    throw new Error("Codex completion notice should be recognized");
  }
  if (latestCompactionCompletionSeq(completed) !== 11) {
    throw new Error("completion sequence should identify the fresh compact result");
  }
});

Deno.test("derive preserves unchanged row identities across timeline successors", () => {
  const firstTimeline: Envelope[] = [
    {
      session_id: "s1",
      seq: 1,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "stable" } },
    },
    {
      session_id: "s1",
      seq: 2,
      kind: "update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-1",
        title: "Running",
        status: "pending",
        content: { type: "text", text: "large result placeholder" },
      },
    },
  ];
  const first = derive(firstTimeline);
  const successor = linkTimeline([
    ...firstTimeline,
    {
      session_id: "s1",
      seq: 3,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "new" } },
    },
  ], firstTimeline);
  const second = derive(successor);
  if (first[0] !== second[0] || first[1] !== second[1]) {
    throw new Error("unchanged transcript rows should retain object identity");
  }
  if (second[2]?.kind !== "message") throw new Error("new row should still be derived");
});

Deno.test("derive exposes Codex read locations and formatted raw output", () => {
  const items = derive([{
    session_id: "s1",
    seq: 7,
    kind: "update",
    update: {
      sessionUpdate: "tool_call",
      toolCallId: "read-1",
      kind: "read",
      title: "Read file '/tmp/example.txt'",
      status: "completed",
      locations: [{ path: "/tmp/example.txt" }],
      rawOutput: { exit_code: 0, formatted_output: "example contents" },
    },
  }]);
  const tool = items[0];
  if (tool?.kind !== "tool") throw new Error("expected a tool row");
  if ((tool.rawInput as { path?: string }).path !== "/tmp/example.txt") {
    throw new Error("read location should become the renderer path");
  }
  const content = tool.content as { type?: string; text?: string }[];
  if (content[0]?.type !== "raw_output" || content[0].text !== "example contents") {
    throw new Error("formatted raw output should remain expandable");
  }
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
