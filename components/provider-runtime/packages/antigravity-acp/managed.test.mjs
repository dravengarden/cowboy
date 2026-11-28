import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { decide } from "./hook.mjs";
import {
  hasManagedRound,
  hookManaged,
  launchManagedMode,
  MANAGED_ALLOWED_TOOLS,
  MANAGED_DENIAL_REASON,
  MANAGED_META_KEY,
  managedArguments,
  managedConfigAllowed,
  managedMode,
  managedPrompt,
} from "./managed.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const input = (name, args = {}) => JSON.stringify({ toolCall: { name, args } });
const BINDING = { managed: { parent_session_id: "parent", profile: "read_only_v1" } };
const ANNOUNCED = { schema: 1, profile: "read_only_v1", roundPath: "/m/round.json" };

test("only the exact read-only profile flag is recognised", () => {
  assert.deepEqual(managedArguments(["acp", "--cowboy-managed-profile=read-only-v1"]), {
    managed: true,
    args: ["acp"],
  });
  assert.deepEqual(managedArguments(["acp"]), { managed: false, args: ["acp"] });
  for (const flag of [
    "--cowboy-managed-profile=image-v1",
    "--cowboy-managed-profile",
    "--cowboy-managed-profile=read_only_v1",
  ]) {
    assert.throws(() => managedArguments([flag]), /Unsupported/, flag);
  }
  // The output contract declaration is the Controller's; agy never sees it.
  assert.deepEqual(
    managedArguments([
      "acp",
      "--cowboy-managed-profile=read-only-v1",
      "--cowboy-output-contract=v1",
    ]),
    { managed: true, args: ["acp"] },
  );
  assert.throws(() => managedArguments(["acp", "--cowboy-output-contract=v1"]), /managed/);
  assert.throws(
    () =>
      managedArguments(["--cowboy-managed-profile=read-only-v1", "--cowboy-output-contract=v2"]),
    /Unsupported/,
  );
});

test("managed mode needs the binding and the target to name the same profile", () => {
  assert.equal(managedMode({}, null), false);
  assert.equal(managedMode({ managed: null }, undefined), false);
  assert.equal(managedMode(BINDING, ANNOUNCED), true);
  for (const [binding, announced] of [
    [BINDING, null],
    [{}, ANNOUNCED],
    [{ managed: { ...BINDING.managed, profile: "image_v1" } }, ANNOUNCED],
    [{ managed: { profile: "read_only_v1" } }, ANNOUNCED],
    [BINDING, { ...ANNOUNCED, profile: "image_v1" }],
    [BINDING, { ...ANNOUNCED, schema: 2 }],
    [BINDING, { ...ANNOUNCED, roundPath: "relative" }],
  ]) {
    assert.throws(() => managedMode(binding, announced), /read-only execution environment/);
  }
});

test("a managed turn needs its round, text output and no slash command", () => {
  const round = { callId: "c", inputRevision: "r", outputSchema: null };
  const prompt = [{ type: "text", text: "Summarise src/" }];
  assert.match(managedPrompt({ prompt }).error, /managed-call round/);
  for (const blocks of [
    undefined,
    [],
    [{ type: "image", data: "AAAA", mimeType: "image/png" }],
    [...prompt, { type: "resource", resource: { uri: "file:///etc/passwd", text: "x" } }],
    [{ type: "text", text: "x", annotations: {} }],
    [{ type: "text" }],
  ]) {
    assert.match(
      managedPrompt({ prompt: blocks, _meta: { [MANAGED_META_KEY]: round } }).error,
      /text blocks only/,
      JSON.stringify(blocks),
    );
  }
  assert.match(
    managedPrompt({ prompt, _meta: { [MANAGED_META_KEY]: { callId: "" } } }).error,
    /managed-call round/,
  );
  // A structured round is answered through the output contract.
  assert.deepEqual(
    managedPrompt({
      prompt,
      _meta: { [MANAGED_META_KEY]: { ...round, outputSchema: { type: "object" } } },
    }).schema,
    { type: "object" },
  );
  assert.match(
    managedPrompt({
      prompt,
      _meta: { [MANAGED_META_KEY]: { ...round, outputSchema: "object" } },
    }).error,
    /JSON Schema object/,
  );
  for (const text of ["/compact", "  /schedule 5m"]) {
    assert.match(
      managedPrompt({ prompt: [{ type: "text", text }], _meta: { [MANAGED_META_KEY]: round } })
        .error,
      /slash command/,
      text,
    );
  }
  // Admitted without Cowboy's metadata; other metadata stays.
  assert.deepEqual(
    managedPrompt({ sessionId: "s", prompt, _meta: { [MANAGED_META_KEY]: round } }),
    {
      params: { sessionId: "s", prompt },
    },
  );
  const { [MANAGED_META_KEY]: _absent, ...rest } = { [MANAGED_META_KEY]: round, other: 1 };
  assert.deepEqual(managedPrompt({ prompt, _meta: { [MANAGED_META_KEY]: round, other: 1 } }), {
    params: { prompt, _meta: rest },
  });
  // A round without outputSchema is text too.
  assert.ok(managedPrompt({ prompt, _meta: { [MANAGED_META_KEY]: { callId: "c" } } }).params);
  assert.equal(hasManagedRound({ _meta: { [MANAGED_META_KEY]: {} } }), true);
  assert.equal(hasManagedRound({ prompt }), false);
});

test("a managed child changes only its model and preset options", () => {
  assert.equal(managedConfigAllowed("session/set_model", {}), true);
  assert.equal(managedConfigAllowed("session/set_config_option", { configId: "model" }), true);
  assert.equal(
    managedConfigAllowed("session/set_config_option", { configId: "thought_level" }),
    true,
  );
  assert.equal(managedConfigAllowed("session/set_config_option", { configId: "mode" }), false);
  assert.equal(managedConfigAllowed("session/set_mode", { modeId: "default" }), false);
  assert.equal(managedConfigAllowed("session/prompt", {}), true);
});

test("the managed hook allows only the read tools and RunCommand", () => {
  for (const name of MANAGED_ALLOWED_TOOLS) {
    assert.deepEqual(decide(input(name), { managed: true }), { decision: "allow" }, name);
  }
  for (const name of [
    "mcp_cowboy_WriteFile",
    "mcp_cowboy_EditFile",
    "mcp_cowboy_StopCommand",
    "mcp_cowboy_cowboy_execute",
    "mcp_cowboy_cowboy_read",
    "mcp_cowboy_memory_search",
    "ask_question",
    "search_web",
    "read_url_content",
    "view_content_chunk",
    "invoke_subagent",
    "schedule",
    "swarm",
    "read_file",
    "",
  ]) {
    assert.deepEqual(
      decide(input(name, { Url: "https://example.com" }), { managed: true }),
      { decision: "deny", reason: MANAGED_DENIAL_REASON },
      name,
    );
  }
  assert.equal(decide("not json", { managed: true }).decision, "deny");
});

// `bin/agy-hook` of the account-sandbox runtime archive, byte for byte
// (HOOK_ENTRY in components/machine/src/machine_plugins/account_sandbox/
// runtime_archive.rs). The daemon binds `policy/hooks.json` read-only over
// the home; its gate runs `agy-hook` from the PATH the proxy gives `agy`.
const HOOK_ENTRY =
  '#!/bin/sh\nset -eu\ncase "$0" in */*) cowboy_dir=${0%/*} ;; *) cowboy_dir=. ;; esac\nexec "$cowboy_dir/../proxy/runtime/node" "$cowboy_dir/../proxy/app/hook.mjs"\n';

function packagedHook(t) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-hook-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  for (const directory of ["bin", "proxy/app", "proxy/runtime"]) {
    mkdirSync(join(base, directory), { recursive: true });
  }
  for (const file of ["hook.mjs", "managed.mjs"]) {
    copyFileSync(join(here, file), join(base, "proxy/app", file));
  }
  symlinkSync(process.execPath, join(base, "proxy/runtime/node"));
  writeFileSync(join(base, "bin/agy-hook"), HOOK_ENTRY, { mode: 0o555 });
  return join(base, "bin");
}

test("the packaged hook allows the ordinary policy only for the exact marker", (t) => {
  const gate = JSON.parse(readFileSync(join(here, "hooks.json"), "utf8"))["cowboy-facade-gate"];
  assert.equal(gate.PreToolUse[0].hooks[0].command, "agy-hook");
  const bin = packagedHook(t);
  // As agy would: the command name looked up on its PATH, stdin the call.
  const run = (name, env) => {
    const result = spawnSync("agy-hook", [], {
      input: input(name),
      env: { PATH: bin, ...env },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout).decision;
  };
  assert.equal(run("search_web", { COWBOY_HOOK_PROFILE: "ordinary" }), "allow");
  assert.equal(run("ask_question", { COWBOY_HOOK_PROFILE: "ordinary" }), "allow");
  // No marker (agy passed nothing on), an empty, unknown or managed one.
  for (const env of [
    {},
    { COWBOY_HOOK_PROFILE: "" },
    { COWBOY_HOOK_PROFILE: "Ordinary" },
    { COWBOY_HOOK_PROFILE: "read-only-v1" },
  ]) {
    for (const name of ["search_web", "read_url_content", "ask_question", "mcp_cowboy_WriteFile"]) {
      assert.equal(run(name, env), "deny", `${name} ${JSON.stringify(env)}`);
    }
    assert.equal(run("mcp_cowboy_ReadFile", env), "allow");
  }
  // Even with an empty environment, through its absolute path.
  const bare = spawnSync(join(bin, "agy-hook"), [], {
    input: input("search_web"),
    env: {},
    encoding: "utf8",
  });
  assert.equal(JSON.parse(bare.stdout).decision, "deny");
  assert.equal(hookManaged({}), true);
  assert.equal(hookManaged({ COWBOY_HOOK_PROFILE: "x" }), true);
  assert.equal(hookManaged({ COWBOY_HOOK_PROFILE: "ordinary" }), false);
});

test("before anything starts, the binding and the launch flag must agree", () => {
  assert.equal(launchManagedMode({}, false), false);
  assert.equal(launchManagedMode({ managed: null }, false), false);
  assert.equal(launchManagedMode(BINDING, false), true);
  assert.equal(launchManagedMode(BINDING, true), true);
  assert.throws(() => launchManagedMode({}, true), /requires a managed execution binding/);
  for (const managed of [
    { profile: "read_only_v1" },
    { profile: "read_only_v1", parent_session_id: "" },
    { profile: "read_only_v1", parent_session_id: 7 },
    { profile: "image_v1", parent_session_id: "parent" },
    {},
  ]) {
    assert.throws(
      () => launchManagedMode({ managed }, false),
      /read-only execution environment/,
      JSON.stringify(managed),
    );
  }
});

test("managed continue flag requires the managed profile and refuses unknown versions", () => {
  assert.deepEqual(
    managedArguments([
      "acp",
      "--cowboy-managed-profile=read-only-v1",
      "--cowboy-output-contract=v1",
      "--cowboy-managed-continue=v1",
    ]),
    { managed: true, args: ["acp"] },
  );
  assert.throws(() => managedArguments(["acp", "--cowboy-managed-continue=v1"]), /managed profile/);
  assert.throws(
    () =>
      managedArguments(["--cowboy-managed-profile=read-only-v1", "--cowboy-managed-continue=v2"]),
    /Unsupported Cowboy managed continue/,
  );
});
