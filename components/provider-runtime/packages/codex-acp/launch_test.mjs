import assert from "node:assert/strict";
import test from "node:test";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";
import { delimiter, join } from "node:path";
import { splitConfigurationArguments } from "./launch.mjs";

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
