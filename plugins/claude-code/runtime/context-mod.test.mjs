import assert from "node:assert/strict";
import test from "node:test";
import {
  nativeLocalRead,
  rememberDenial,
  targetCompactionResult,
  targetImageResult,
  withheldResult,
} from "./context-mod.js";

test("target image result retains media and target path without the native cache locator", () => {
  const image = { type: "image", source: { type: "base64", data: "fixture" } };
  const target = {
    type: "text",
    text: "Image source in workspace: /target/a.png",
  };
  const cached = {
    type: "text",
    text:
      "[Image: source: /runtime/projects/session/tool-results/mcp-cowboy_execution-blob-1.png]",
  };
  const event = {
    origin: { kind: "tool", tool: "ReadFile" },
    message: {
      role: "user",
      content: [
        {
          type: "tool_result",
          tool_use_id: "original",
          content: [image, cached, target],
        },
      ],
    },
  };
  const result = targetImageResult(event);
  assert.deepEqual(result.message.content[0], {
    type: "tool_result",
    tool_use_id: "original",
    content: [image, target],
  });
  assert.equal(event.message.content[0].content.length, 3);
  const other = { ...event, origin: { kind: "tool", tool: "another_server" } };
  assert.equal(targetImageResult(other), other);
  const textOnly = {
    ...event,
    message: {
      content: [
        { type: "tool_result", tool_use_id: "text", content: [cached] },
      ],
    },
  };
  assert.deepEqual(targetImageResult(textOnly), textOnly);
});

test("native compaction keeps its summary without suggesting a runtime transcript read", () => {
  const summary =
    "Kept target summary.\n\nIf you need specific details from before compaction (like exact code snippets, error messages, or content you generated), read the full transcript at: /runtime/session.jsonl\nContinue the conversation.";
  const event = {
    door: "compaction",
    origin: { kind: "engine" },
    message: {
      content: [
        { type: "text", text: summary },
      ],
    },
  };
  assert.equal(
    targetCompactionResult(event).message.content[0].text,
    "Kept target summary.\n\nContinue the conversation.",
  );
  const user = { ...event, door: "prompt" };
  assert.equal(targetCompactionResult(user), user);
});

test("native's own Read of a local copy names the target file", async () => {
  const copy = "/runtime/state/local-reads/abc.png";
  const answer = {
    localRead: copy,
    localTarget: "/target/a.png",
    instructions: [],
  };
  const event = { tool: "Read", file_path: "a.png", tool_use_id: "t" };
  const read = [];
  const next = (value) => {
    read.push(value.file_path);
    return Promise.resolve({
      result: { type: "image", file: { base64: "AA==", type: "image/png" } },
      newMessages: [{ note: true }],
    });
  };
  assert.deepEqual(await nativeLocalRead({}, event, next, answer), {
    instructions: [],
    result: { type: "image", file: { base64: "AA==", type: "image/png" } },
    newMessages: [{ note: true }],
  });
  assert.deepEqual(read, [copy]);
  const failing = () =>
    Promise.resolve({
      result: `Error: Image file is empty: ${copy}`,
      isError: true,
    });
  assert.deepEqual(await nativeLocalRead({}, event, failing, answer), {
    deny: "Image file is empty: /target/a.png",
  });
  for (const target of ["/target/price$&.png", "/target/price$'.png"]) {
    assert.deepEqual(
      await nativeLocalRead({}, event, failing, {
        ...answer,
        localTarget: target,
      }),
      { deny: `Image file is empty: ${target}` },
    );
  }
  const throwing = () => Promise.reject(new Error(`bad ${copy}`));
  assert.deepEqual(await nativeLocalRead({}, event, throwing, answer), {
    deny: "bad /target/a.png",
  });
});

test("a denial of this module after the tool ran keeps the failure text alone", () => {
  const reasons = new Map();
  const failure =
    'Shell command failed for pattern "!`exit 3`": [stderr]\npartial';
  const row = (id, content, tool = "Skill") => ({
    door: "tool-result",
    origin: { kind: "tool", tool },
    message: {
      role: "user",
      content: [{
        type: "tool_result",
        tool_use_id: id,
        content,
        is_error: true,
      }],
    },
  });
  const noted = (tool, reason, state = "ran") => [{
    type: "text",
    text: `${tool} ${state}, and a plugin withheld its result: ${reason}`,
  }];
  rememberDenial({ tool_use_id: "a" }, { deny: failure }, reasons);
  assert.equal(
    withheldResult(row("a", noted("Skill", failure)), reasons).message
      .content[0].content,
    failure,
  );
  assert.equal(reasons.size, 0);
  rememberDenial({ tool_use_id: "b" }, { deny: "gone" }, reasons);
  assert.equal(
    withheldResult(row("b", noted("Read", "gone", "started"), "Read"), reasons)
      .message.content[0].content,
    "gone",
  );
  // Another plugin's denial, or one for another call, is not this module's.
  const other = row("c", noted("Skill", failure));
  assert.equal(withheldResult(other, reasons), other);
  rememberDenial({ tool_use_id: "d" }, { deny: "mine" }, reasons);
  const foreign = row("d", noted("Skill", "theirs"));
  assert.equal(withheldResult(foreign, reasons), foreign);
  // A denial before the tool ran carries no note and is left as it is.
  rememberDenial({ tool_use_id: "e" }, { deny: "refused" }, reasons);
  const plain = row("e", [{ type: "text", text: "refused" }]);
  assert.equal(withheldResult(plain, reasons), plain);
  assert.equal(reasons.size, 0);
  // Only denials are remembered, and the answer is returned as given.
  const answer = { result: {} };
  assert.equal(rememberDenial({ tool_use_id: "f" }, answer, reasons), answer);
  assert.equal(reasons.size, 0);
});
