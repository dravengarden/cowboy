import assert from "node:assert/strict";
import {
  chmod,
  mkdtemp,
  readdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { MatrixClient, matrixConfiguration } from "./index.mjs";

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "matrix-client-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const requests = [];
  let available = true;
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const value = JSON.parse(Buffer.concat(chunks));
    requests.push({ path: request.url, ...value });
    if (
      request.headers.authorization !== "Bearer " + "a".repeat(64) || !available
    ) {
      response.writeHead(503).end("{}");
      return;
    }
    const result = request.url === "/v1/context"
      ? { text: "[Matrix memory]\nCurrent revision only." }
      : { events: ["observation:e0"] };
    response.writeHead(200, { "Content-Type": "application/json" }).end(
      JSON.stringify(result),
    );
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() =>
    new Promise((resolve) => {
      server.closeAllConnections();
      server.close(resolve);
    })
  );
  const config = {
    schema: 1,
    provider: "codex",
    endpoint: "http://127.0.0.1:" + server.address().port,
    token: "a".repeat(64),
    state_dir: join(root, "queue"),
    projects: [
      { workspace: "project-123", machine: "hawk", project: "cowboy" },
      {
        workspace: "different-project-id",
        machine: "falcon",
        project: "cowboy",
      },
      { path: "/local/cowboy", machine: "local", project: "cowboy" },
    ],
  };
  const descriptor = {
    binding: {
      id: "session-123",
      workspace: { id: "project-123", cwd: "/task/worktree-7" },
      environment: { machine_id: "hawk" },
    },
  };
  return {
    root,
    requests,
    config,
    descriptor,
    setAvailable: (value) => available = value,
  };
}

test("host configuration refuses wrong Provider, insecure transport and symlinks", async (t) => {
  const f = await fixture(t);
  const path = join(f.root, "config.json");
  await writeFile(path, JSON.stringify(f.config), { mode: 0o600 });
  assert.equal(
    (await matrixConfiguration("codex", { path })).provider,
    "codex",
  );
  await assert.rejects(matrixConfiguration("claude", { path }), /Invalid/);
  const link = join(f.root, "link");
  await symlink(path, link);
  await assert.rejects(
    matrixConfiguration("codex", { path: link }),
    /private regular/,
  );
  await chmod(path, 0o644);
  await assert.rejects(
    matrixConfiguration("codex", { path }),
    /private regular/,
  );
  await chmod(path, 0o600);
  await writeFile(
    path,
    JSON.stringify({ ...f.config, endpoint: "http://remote.invalid" }),
  );
  await assert.rejects(matrixConfiguration("codex", { path }), /Invalid/);
  assert.equal(
    await matrixConfiguration("codex", { path: join(f.root, "absent") }),
    null,
  );
});

test("logical project identity spans Machines and ignores task directory names", async (t) => {
  const f = await fixture(t);
  const first = await MatrixClient.open(f.config, f.descriptor);
  const second = await MatrixClient.open(f.config, {
    binding: {
      id: "other-session",
      workspace: { id: "different-project-id" },
      environment: { machine_id: "falcon" },
    },
  });
  assert.deepEqual(first.binding, {
    project: "cowboy",
    machine: "hawk",
    session: "session-123",
  });
  assert.equal(second.binding.project, first.binding.project);
  assert.notEqual(second.binding.machine, first.binding.machine);
  await assert.rejects(
    MatrixClient.open(f.config, undefined, { cwd: "/other/cowboy" }),
    /mapping/,
  );
  await assert.rejects(
    MatrixClient.open({
      ...f.config,
      projects: [...f.config.projects, f.config.projects[0]],
    }, f.descriptor),
    /ambiguous/,
  );
  await first.close();
  await second.close();
  await first.delivery;
  await second.delivery;
});

test("outage retains completed turns for restart delivery without stale recall", async (t) => {
  const f = await fixture(t);
  const client = await MatrixClient.open(f.config, f.descriptor);
  assert.match(
    await client.begin("Use the release receipt."),
    /Current revision/,
  );
  f.setAvailable(false);
  assert.match(
    await client.begin(
      "Now forget the receipt. bearer abcdefghijklmnopqrstuvwxyz",
    ),
    /unavailable/,
  );
  assert.doesNotMatch(client.context, /Current revision/);
  client.add("tool", "Verification passed.");
  client.add("assistant", "Recorded the result.");
  await client.close();
  await client.delivery;
  const names = (await readdir(client.directory)).filter((name) =>
    name.endsWith(".json")
  );
  assert.ok(names.length >= 2);
  for (const name of names) {
    assert.doesNotMatch(
      await readFile(join(client.directory, name), "utf8"),
      /abcdefghijklmnopqrstuvwxyz/,
    );
  }
  f.setAvailable(true);
  const restarted = await MatrixClient.open(f.config, f.descriptor);
  await restarted.flush();
  assert.equal(
    (await readdir(client.directory)).filter((name) => name.endsWith(".json"))
      .length,
    0,
  );
  assert.ok(
    f.requests.some((request) =>
      request.payload.events?.some((event) => event.role === "tool")
    ),
  );
  assert.ok(f.requests.some((request) => request.payload.learn === false));
  await restarted.close();
  await restarted.delivery;
});
