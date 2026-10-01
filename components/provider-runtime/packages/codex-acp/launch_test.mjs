import assert from "node:assert/strict";
import test from "node:test";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import {
  bindExecutionRequest,
  readExecutionDescriptor,
  splitConfigurationArguments,
} from "./launch.mjs";

test("the private CLI process receives the configured argv without a shell", () => {
  const echo = (process.env.PATH ?? "").split(delimiter)
    .map((directory) => join(directory, "echo")).find(existsSync);
  assert.ok(echo, "the pinned test shell must supply echo");
  const output = execFileSync(process.execPath, [
    fileURLToPath(new URL("./launch.mjs", import.meta.url)),
    "--cowboy-private-cli",
    "app-server",
  ], {
    env: {
      COWBOY_PRIVATE_CODEX_EXECUTABLE: echo,
      COWBOY_PRIVATE_CODEX_ARGUMENTS: JSON.stringify([
        "-c",
        "model_context_window=680000",
      ]),
    },
    encoding: "utf8",
  });
  assert.equal(output, "-c model_context_window=680000 app-server\n");
});

test("private launch forwards exact Codex config without interpreting TOML or shell", () => {
  const configuration = [
    "-c",
    "approval_policy=never",
    "-c",
    'model_providers.private.base_url="http://127.0.0.1:4321/v1"',
  ];
  assert.deepEqual(splitConfigurationArguments(configuration), {
    configuration,
    arguments: [],
  });
});

test("probe and authentication arguments still belong to the upstream adapter", () => {
  for (const args of [["--version"], ["--help"], ["login", "--device-auth"]]) {
    assert.deepEqual(splitConfigurationArguments(args), {
      configuration: [],
      arguments: args,
    });
  }
});

test("invalid or incomplete configuration never reaches an executable", () => {
  for (
    const args of [["-c"], ["--config", "--help"], ["-c", "../escape=true"]]
  ) {
    assert.throws(() => splitConfigurationArguments(args));
  }
});

function descriptor() {
  return {
    schema: 1,
    endpoint: "ws://127.0.0.1:43210/",
    bearer_token: "a".repeat(64),
    binding: {
      schema: 1,
      environment: { protocol: 1, id: "environment-one" },
      workspace: { cwd: "/target/worktree" },
    },
  };
}

test("native new and resumed turns get the same exact environment without rewriting user input", () => {
  for (const method of ["thread/start", "turn/start"]) {
    const original = {
      id: 42,
      method,
      params: { cwd: "/runtime/entry", input: [{ text: "quotes '$() 中文" }] },
    };
    const actual = bindExecutionRequest(original, descriptor());
    assert.deepEqual(actual.params.environments, [{
      environmentId: "environment-one",
      cwd: "/target/worktree",
      runtimeWorkspaceRoots: ["/target/worktree"],
    }]);
    assert.deepEqual(actual.params.input, original.params.input);
    assert.equal(original.params.environments, undefined);
    assert.throws(() =>
      bindExecutionRequest(
        { method, params: { environments: [] } },
        descriptor(),
      )
    );
  }
  const resume = { method: "thread/resume", params: { threadId: "saved" } };
  assert.deepEqual(bindExecutionRequest(resume, descriptor()), resume);
});

test("execution descriptor rejects public files, symlinks, arbitrary hosts and unsupported identities", async () => {
  const directory = mkdtempSync(join(tmpdir(), "cowboy-launch-"));
  const path = join(directory, "descriptor.json");
  try {
    writeFileSync(path, JSON.stringify(descriptor()), { mode: 0o600 });
    assert.deepEqual(await readExecutionDescriptor(path), descriptor());
    chmodSync(path, 0o644);
    await assert.rejects(readExecutionDescriptor(path));
    chmodSync(path, 0o600);
    symlinkSync(path, join(directory, "alias"));
    await assert.rejects(readExecutionDescriptor(join(directory, "alias")));
    for (
      const patch of [
        { endpoint: "ws://remote.example/" },
        { endpoint: "ws://127.0.0.1/?secret=x" },
        { bearer_token: "short" },
        { schema: 2 },
      ]
    ) {
      writeFileSync(path, JSON.stringify({ ...descriptor(), ...patch }));
      await assert.rejects(readExecutionDescriptor(path));
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
