import assert from "node:assert/strict";
import test from "node:test";
import {
  MANAGED_PROFILE_FLAG,
  managedArguments,
  managedCommandRefused,
  managedConstraint,
  managedControlRefused,
  managedNativeArguments,
  parseRound,
  structuredMessage,
} from "./managed.mjs";

const binding = {
  managed: { parent_session_id: "parent", profile: "read_only_v1" },
};
const announced = {
  schema: 1,
  profile: "read_only_v1",
  roundPath: "/state/managed/child/round.json",
};

test("only the exact signed flag selects the managed profile", () => {
  assert.deepEqual(managedArguments([]), { managed: false, args: [] });
  assert.deepEqual(managedArguments([MANAGED_PROFILE_FLAG, "--x"]), {
    managed: true,
    args: ["--x"],
  });
  assert.throws(() => managedArguments(["--cowboy-managed-profile=full"]));
});

test("the target keeper must announce the binding's exact profile", () => {
  assert.deepEqual(managedConstraint(binding, announced), {
    roundPath: announced.roundPath,
  });
  for (
    const [value, target] of [
      [{}, announced],
      [binding, null],
      [binding, { ...announced, profile: "full_access" }],
      [binding, { ...announced, roundPath: "relative/round.json" }],
      [{ managed: { ...binding.managed, profile: "other" } }, announced],
    ]
  ) {
    assert.throws(() => managedConstraint(value, target), /read-only/);
  }
});

test("rounds carry an optional object schema", () => {
  const round = {
    schema: 1,
    call_id: "call-1",
    output_schema: { type: "object" },
  };
  assert.deepEqual(parseRound(Buffer.from(JSON.stringify(round))), round);
  assert.equal(
    parseRound(Buffer.from(JSON.stringify({ schema: 1, call_id: "c" })))
      .output_schema,
    undefined,
  );
  for (
    const invalid of [{ schema: 2, call_id: "c" }, {
      schema: 1,
      call_id: "c",
      output_schema: [],
    }]
  ) {
    assert.throws(() => parseRound(Buffer.from(JSON.stringify(invalid))));
  }
  assert.throws(() => parseRound(undefined));
});

test("managed native argv denies prompts, writes and nested agents", () => {
  const argv = managedNativeArguments(
    [
      "--print",
      "--tools",
      "Bash,Read,Write,Edit,Agent",
      "--disallowedTools",
      "EnterWorktree",
      "--permission-mode",
      "bypassPermissions",
      "--allow-dangerously-skip-permissions",
      "--allowedTools",
      "mcp__matrix__x",
      "--json-schema",
      "{}",
      "--model",
      "opus",
    ],
    { type: "object", required: ["verdict"] },
  );
  const value = (flag) => argv[argv.indexOf(flag) + 1];
  assert.equal(argv.filter((item) => item === "--permission-mode").length, 1);
  assert.equal(value("--permission-mode"), "dontAsk");
  assert.equal(
    value("--tools"),
    "Bash,Read,Glob,Grep,TaskCreate,TaskGet,TaskList,TaskUpdate",
  );
  for (
    const tool of [
      "EnterWorktree",
      "Write",
      "Edit",
      "Agent",
      "WebFetch",
      "Skill",
    ]
  ) {
    assert.ok(value("--disallowedTools").split(",").includes(tool), tool);
  }
  assert.equal(
    value("--allowedTools"),
    "Bash,Read,Glob,Grep,TaskCreate,TaskGet,TaskList,TaskUpdate",
  );
  assert.equal(
    value("--json-schema"),
    '{"type":"object","required":["verdict"]}',
  );
  assert.equal(argv.filter((item) => item === "--json-schema").length, 1);
  assert.ok(!argv.includes("--allow-dangerously-skip-permissions"));
  assert.equal(value("--model"), "opus");
  assert.ok(
    !managedNativeArguments(["--print"], undefined).includes("--json-schema"),
  );
});

test("native structured output becomes the turn's final message", () => {
  const last = {
    type: "assistant",
    session_id: "s",
    message: { model: "claude-opus-5-5", usage: { input_tokens: 3 } },
  };
  const message = structuredMessage(
    {
      type: "result",
      subtype: "success",
      session_id: "s",
      structured_output: { verdict: "approve" },
    },
    last,
  );
  assert.equal(message.type, "assistant");
  assert.equal(message.parent_tool_use_id, null);
  assert.equal(message.message.model, "claude-opus-5-5");
  assert.deepEqual(message.message.usage, { input_tokens: 3 });
  assert.deepEqual(message.message.content, [{
    type: "text",
    text: '{"verdict":"approve"}',
  }]);
  assert.equal(
    structuredMessage({ type: "result", subtype: "success" }, last),
    undefined,
  );
  assert.equal(
    structuredMessage({
      type: "result",
      subtype: "error_max_structured_output_retries",
      structured_output: {},
    }, last),
    undefined,
  );
});

test("managed configuration and native commands are refused", () => {
  assert.ok(
    managedControlRefused({
      subtype: "set_permission_mode",
      mode: "acceptEdits",
    }),
  );
  assert.ok(!managedControlRefused({ subtype: "set_model" }));
  assert.ok(managedCommandRefused("  /compact"));
  assert.ok(!managedCommandRefused("Review src/ for /tmp writes"));
});
