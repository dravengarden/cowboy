import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { Connection } from "../../execution-connection.mjs";
import { Facade, LIMITS, Retained, ToolError } from "./facade.mjs";
import { decodeText, HomeReads, imageType, splice } from "./facade-tools.mjs";
import {
  buildProjection,
  guidanceDirectories,
  PROJECTION_LIMITS,
  parseGit,
  READINESS_PROBE,
} from "./projection.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

const hasBash = (() => {
  try {
    return execFileSync("sh", ["-c", "command -v bash"]).toString().trim() || false;
  } catch {
    return false;
  }
})();

const hasFish = (() => {
  try {
    return execFileSync("sh", ["-c", "command -v fish"]).toString().trim() || false;
  } catch {
    return false;
  }
})();

const hasRg = (() => {
  try {
    execFileSync("rg", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
})();

async function fixture(t, options = {}, facadeOptions = {}) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-facade-")));
  const root = join(base, "workspace");
  mkdirSync(join(root, "src"), { recursive: true });
  mkdirSync(join(base, "outside"));
  writeFileSync(join(base, "outside", "secret.txt"), "OUTSIDE_SECRET\n");
  writeFileSync(join(root, "hello.txt"), "hello target\n");
  symlinkSync(join(base, "outside"), join(root, "escape"));
  const socket = join(base, "exec.sock");
  const endpoint = await fakeEndpoint(socket, root, options);
  const connection = await Connection.open(
    {
      schema: 1,
      endpoint: `unix:${socket}`,
      bearer_token: "a".repeat(64),
      binding: { schema: 1, environment: { id: "env" }, workspace: { cwd: root } },
    },
    { clientName: "cowboy-antigravity-test", requestTimeoutMs: 20000 },
  );
  const uncertain = [];
  const results = [];
  const progress = [];
  const facade = await Facade.create(connection, {
    onUncertain: (session, method) => uncertain.push([session, method]),
    onResult: (call, result) => results.push([call, result]),
    onProgress: (call, text) => progress.push([call, text]),
    ...facadeOptions,
  });
  t.after(() => {
    connection.close();
    endpoint.close();
    rmSync(base, { recursive: true, force: true });
  });
  facade.beginTurn("s1");
  const call = async (name, args) => {
    const result = await facade.call("s1", name, args);
    return { text: result.content[0].text, error: result.isError === true };
  };
  return { base, root, facade, call, endpoint, uncertain, results, progress };
}

test("every answer is reported for presentation, a running command's output as it comes", async (t) => {
  const { facade, call, results, progress } = await fixture(t);
  assert.match((await call("ReadFile", { path: "hello.txt" })).text, /hello target/);
  assert.equal(results.length, 1);
  const [reported, result] = results[0];
  assert.deepEqual(reported, { sessionId: "s1", name: "ReadFile", args: { path: "hello.txt" } });
  assert.match(result.content[0].text, /hello target/);
  // A failing presentation never fails the tool.
  facade.onResult = () => {
    throw new Error("presentation");
  };
  assert.equal((await call("ListDirectory", {})).error, false);
  facade.onResult = (reportedCall, answer) => results.push([reportedCall, answer]);
  const ran = await call("RunCommand", {
    command: "printf one; sleep 2.5; printf two; sleep 0.2",
  });
  assert.match(ran.text, /Exit code: 0\none\s*two/);
  assert.ok(progress.length >= 1, "the output so far was shown while it ran");
  const [running, text] = progress[0];
  assert.equal(running, results.at(-1)[0], "progress and result share the call");
  assert.match(text, /one/);
  assert.doesNotMatch(text, /two/);
});

test("UpdatePlan validates the plan and never reaches the target", async (t) => {
  const { call } = await fixture(t);
  const plan = [
    { step: "Read the code", status: "completed" },
    { step: "Fix the bug", status: "in_progress" },
  ];
  assert.deepEqual(await call("UpdatePlan", { plan }), { text: "Plan updated.", error: false });
  for (const bad of [{}, { plan: [] }, { plan: [{ step: "x", status: "done" }] }]) {
    assert.equal((await call("UpdatePlan", bad)).error, true, JSON.stringify(bad));
  }
});

test("tools are unavailable outside a turn and after the turn ends", async (t) => {
  const { facade } = await fixture(t);
  const outside = await facade.call("other", "ReadFile", { path: "hello.txt" });
  assert.equal(outside.isError, true);
  assert.match(outside.content[0].text, /No Cowboy turn/);
  await facade.endTurn("s1");
  assert.equal((await facade.call("s1", "ReadFile", { path: "hello.txt" })).isError, true);
  assert.deepEqual(
    facade.tools().map((tool) => tool.name),
    [
      "ReadFile",
      "ListDirectory",
      "SearchFiles",
      "WriteFile",
      "EditFile",
      "RunCommand",
      "StopCommand",
      "UpdatePlan",
      "Subagent",
      "StopSubagent",
    ],
  );
});

test("ReadFile pages at 256 KiB and reads anywhere the user can read", async (t) => {
  const { root, base, call } = await fixture(t);
  assert.deepEqual(await call("ReadFile", { path: "hello.txt" }), {
    text: "hello target\n",
    error: false,
  });
  writeFileSync(join(root, "big.txt"), "x".repeat(300 * 1024));
  const first = await call("ReadFile", { path: "big.txt" });
  assert.equal(first.text.indexOf("\n[cowboy:"), LIMITS.readBytes);
  assert.match(first.text, /continue with offset=262144\]$/);
  const second = await call("ReadFile", { path: "big.txt", offset: 262144 });
  assert.equal(second.text.split("\n")[0].length, 300 * 1024 - 262144);
  assert.equal(
    (await call("ReadFile", { path: "big.txt", length: LIMITS.readBytes + 1 })).error,
    true,
  );
  // Reads, like Claude's and Codex's, are not confined to the workspace.
  for (const path of [
    "../outside/secret.txt",
    "escape/secret.txt",
    join(base, "outside", "secret.txt"),
  ]) {
    assert.deepEqual(await call("ReadFile", { path }), { text: "OUTSIDE_SECRET\n", error: false });
  }
  assert.match((await call("ReadFile", { path: "missing.txt" })).text, /No such file/);
  assert.equal((await call("ReadFile", { path: "a\0b" })).error, true);
});

const PNG = Buffer.concat([
  Buffer.from("89504e470d0a1a0a", "hex"),
  Buffer.from("0000000d49484452000000010000000108060000001f15c489", "hex"),
  Buffer.from([0]),
]);

test("ReadFile returns a whole image as an MCP image block", async (t) => {
  const { root, facade } = await fixture(t);
  writeFileSync(join(root, "shot.png"), PNG);
  const read = await facade.call("s1", "ReadFile", { path: "shot.png" });
  assert.equal(read.isError, undefined);
  assert.deepEqual(read.content, [
    { type: "text", text: `Image ${join(root, "shot.png")} (image/png, ${PNG.length} bytes)` },
    { type: "image", data: PNG.toString("base64"), mimeType: "image/png" },
  ]);
  // Bytes that are not the named type, a page past the start and an image
  // over 5 MiB stay binary text.
  writeFileSync(join(root, "fake.png"), Buffer.from([0, 1, 2, 3]));
  const fake = await facade.call("s1", "ReadFile", { path: "fake.png" });
  assert.deepEqual(fake.content, [{ type: "text", text: "[binary content, 4 bytes]" }]);
  const paged = await facade.call("s1", "ReadFile", { path: "shot.png", offset: 8 });
  assert.equal(paged.content.length, 1);
  assert.match(paged.content[0].text, /^\[binary content/);
  writeFileSync(join(root, "big.png"), Buffer.concat([PNG, Buffer.alloc(LIMITS.imageBytes)]));
  const big = await facade.call("s1", "ReadFile", { path: "big.png" });
  assert.equal(big.content.length, 1);
  assert.match(big.content[0].text, /^\[binary content, 262144 bytes\]/);
  // An explicit length keeps the byte range; a null one is invalid.
  const nulled = await facade.call("s1", "ReadFile", { path: "shot.png", length: null });
  assert.equal(nulled.isError, true);
  assert.match(nulled.content[0].text, /length must be between/);
  const ranged = await facade.call("s1", "ReadFile", { path: "shot.png", length: 4 });
  assert.deepEqual(ranged.content.length, 1);
  // The type follows the name asked for, not a symlink target's.
  symlinkSync(join(root, "shot.png"), join(root, "shot-link.png"));
  symlinkSync(join(root, "shot.png"), join(root, "shot.bin"));
  writeFileSync(join(root, "target.dat"), PNG);
  symlinkSync(join(root, "target.dat"), join(root, "named.png"));
  assert.equal((await facade.call("s1", "ReadFile", { path: "shot.bin" })).content.length, 1);
  const named = await facade.call("s1", "ReadFile", { path: "named.png" });
  assert.equal(named.content[1]?.mimeType, "image/png");
  assert.equal(facade.session("s1").observed.has(join(root, "target.dat")), true);
  // An image is readable before an edit like any file it read.
  assert.equal(facade.session("s1").observed.has(join(root, "shot.png")), true);
});

test("image types follow both the name and the bytes", () => {
  assert.equal(imageType("/a/b.PNG", PNG), "image/png");
  assert.equal(imageType("/a/b.jpg", Buffer.from([0xff, 0xd8, 0xff, 0xe0])), "image/jpeg");
  assert.equal(imageType("/a/b.jpeg", Buffer.from([0xff, 0xd8, 0xff, 0xdb])), "image/jpeg");
  assert.equal(imageType("/a/b.gif", Buffer.from("GIF89a....")), "image/gif");
  assert.equal(imageType("/a/b.webp", Buffer.from("RIFF\0\0\0\0WEBPVP8 ")), "image/webp");
  assert.equal(imageType("/a/b.gif", PNG), undefined);
  assert.equal(imageType("/a/b.svg", Buffer.from("<svg/>")), undefined);
  assert.equal(imageType("/a/png", PNG), undefined);
});

test("ListDirectory is bounded to 2000 entries", async (t) => {
  const { base, root, call } = await fixture(t);
  mkdirSync(join(root, "many"));
  for (let index = 0; index < 2005; index++) writeFileSync(join(root, "many", `f${index}`), "");
  const listing = await call("ListDirectory", { path: "many" });
  assert.equal(listing.text.split("\n").length, 1 + 2000 + 1);
  assert.match(listing.text, /truncated; 5 more entries/);
  assert.match((await call("ListDirectory", {})).text, /\nsrc\//);
  assert.equal(
    (await call("ListDirectory", { path: "escape" })).text,
    `${join(base, "outside")}:\nsecret.txt`,
  );
});

test("SearchFiles runs the target's rg with match and byte bounds", { skip: !hasRg }, async (t) => {
  const { base, root, call } = await fixture(t);
  writeFileSync(join(root, "src", "a.txt"), "needle one\nhay\n");
  assert.match((await call("SearchFiles", { pattern: "needle" })).text, /src\/a\.txt:1:needle one/);
  assert.equal((await call("SearchFiles", { pattern: "absent-token" })).text, "No matches.");
  writeFileSync(join(root, "src", "many.txt"), "needle\n".repeat(800));
  const many = await call("SearchFiles", { pattern: "needle", path: "src" });
  assert.ok(many.text.split("\n").length <= LIMITS.searchMatches + 1);
  assert.match(many.text, /results truncated/);
  assert.equal(
    (await call("SearchFiles", { pattern: "OUTSIDE", path: "../outside" })).text,
    `${join(base, "outside", "secret.txt")}:1:OUTSIDE_SECRET`,
  );
});

test("SearchFiles with only a glob finds files by name", { skip: !hasRg }, async (t) => {
  const { root, call } = await fixture(t);
  mkdirSync(join(root, "img", "deep"), { recursive: true });
  writeFileSync(join(root, "img", "deep", "logo.PNG"), "");
  writeFileSync(join(root, "img", "icon.png"), "");
  writeFileSync(join(root, "img", "notes.txt"), "png");
  const found = (await call("SearchFiles", { glob: "**/*.png" })).text.split("\n").sort();
  assert.deepEqual(found, ["img/icon.png"]);
  const folded = (await call("SearchFiles", { glob: "**/*.png", ignore_case: true })).text
    .split("\n")
    .sort();
  assert.deepEqual(folded, ["img/deep/logo.PNG", "img/icon.png"]);
  assert.equal((await call("SearchFiles", { glob: "**/*.gif" })).text, "No matches.");
  assert.match((await call("SearchFiles", {})).text, /Give pattern, glob or both/);
  assert.match(
    (await call("SearchFiles", { glob: "*.png", path: "img/notes.txt" })).text,
    /give a directory as path/,
  );
});

test("WriteFile and EditFile are conditional on what was read", async (t) => {
  const { base, root, call } = await fixture(t);
  assert.match(
    (await call("WriteFile", { path: "new/dir/a.txt", content: "one\n" })).text,
    /Created/,
  );
  assert.equal(readFileSync(join(root, "new/dir/a.txt"), "utf8"), "one\n");
  const unread = await call("WriteFile", { path: "hello.txt", content: "x" });
  assert.match(unread.text, /read it with ReadFile/);
  await call("ReadFile", { path: "hello.txt" });
  writeFileSync(join(root, "hello.txt"), "changed elsewhere\n");
  assert.match(
    (await call("WriteFile", { path: "hello.txt", content: "x" })).text,
    /changed since/,
  );
  await call("ReadFile", { path: "hello.txt" });
  execFileSync("chmod", ["0751", join(root, "hello.txt")]);
  assert.match(
    (await call("WriteFile", { path: "hello.txt", content: "mine\n" })).text,
    /Replaced/,
  );
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "mine\n");
  assert.equal(
    execFileSync("stat", ["-c", "%a", join(root, "hello.txt")])
      .toString()
      .trim(),
    "751",
  );
  writeFileSync(join(root, "e.txt"), "a b a\n");
  assert.match(
    (await call("EditFile", { path: "e.txt", old_string: "a", new_string: "c" })).text,
    /Read/,
  );
  await call("ReadFile", { path: "e.txt" });
  assert.match(
    (await call("EditFile", { path: "e.txt", old_string: "a", new_string: "c" })).text,
    /occurs 2 times/,
  );
  await call("EditFile", { path: "e.txt", old_string: "b", new_string: "B" });
  assert.equal(readFileSync(join(root, "e.txt"), "utf8"), "a B a\n");
  await call("EditFile", { path: "e.txt", old_string: "a", new_string: "z", replace_all: true });
  assert.equal(readFileSync(join(root, "e.txt"), "utf8"), "z B z\n");
  // A dangling symlink is never a missing directory to create through.
  symlinkSync(join(base, "outside", "gone"), join(root, "dangling"));
  for (const path of ["dangling/x.txt", "dangling"]) {
    const refused = await call("WriteFile", { path, content: "x" });
    assert.match(refused.text, /symlink whose target does not exist/, path);
  }
  assert.throws(() => readFileSync(join(base, "outside", "gone", "x.txt")));
  assert.throws(() => readFileSync(join(base, "outside", "gone")));
  // Writes stay inside the workspace root, also for a file read outside it.
  // (A parent that cannot be listed: see the dangling-symlink probe test.)
  const secret = join(base, "outside", "secret.txt");
  await call("ReadFile", { path: secret });
  for (const path of ["escape/x.txt", "../x.txt", secret, "escape/secret.txt"]) {
    const refused = await call("WriteFile", { path, content: "x" });
    assert.match(refused.text, /outside the workspace root/, path);
  }
  const edit = await call("EditFile", { path: secret, old_string: "OUTSIDE", new_string: "x" });
  assert.match(edit.text, /outside the workspace root/);
  assert.equal(readFileSync(secret, "utf8"), "OUTSIDE_SECRET\n");
  const big = await call("WriteFile", { path: "big", content: "x".repeat(LIMITS.writeBytes + 1) });
  assert.match(big.text, /larger than 3 MiB/);
});

test("RunCommand bounds output and time; StopCommand stops a running command", async (t) => {
  const { call } = await fixture(t);
  const ok = await call("RunCommand", { command: "printf 'out\\n'; printf 'err\\n' >&2; exit 3" });
  assert.match(ok.text, /^Exit code: 3\n/);
  assert.match(ok.text, /out/);
  assert.match(ok.text, /err/);
  const loud = await call("RunCommand", { command: "head -c 600000 /dev/zero | tr '\\0' x" });
  assert.match(loud.text, /bytes of output omitted/);
  assert.ok(loud.text.length < LIMITS.commandOutput + 200);
  const slow = await call("RunCommand", { command: "sleep 30", timeout_seconds: 1 });
  assert.match(slow.text, /Timed out/);
  const background = await call("RunCommand", {
    command: "echo started; sleep 30",
    wait_seconds: 1,
  });
  const id = /command_id: ([^)]+)\)/.exec(background.text)?.[1];
  assert.ok(id, background.text);
  assert.match(background.text, /started/);
  assert.match((await call("StopCommand", { command_id: id })).text, /Stopped/);
  assert.equal((await call("StopCommand", { command_id: id })).error, true);
  assert.equal((await call("RunCommand", { command: "x", timeout_seconds: 601 })).error, true);
});

test("RunCommand runs the user's shell as a login shell; internal helpers do not", async (t) => {
  const { root, call, endpoint, facade } = await fixture(t);
  const ran = await call("RunCommand", { command: 'echo "login $#"' });
  assert.match(ran.text, /Exit code: 0\nlogin 0/);
  await call("WriteFile", { path: "helper.txt", content: "x\n" });
  const starts = endpoint.calls
    .filter((entry) => entry.method === "process/start")
    .map((entry) => entry.params.argv);
  const guard = (b, dir) =>
    `${b} cd -- '${dir}' || exit 1; ${b} test "$(${b} pwd -P; ${b} printf x)" = '${dir}\nx' || exit 1\n`;
  assert.deepEqual(starts[0], [
    facade.shell,
    "-l",
    "-c",
    `${guard("\\command", root)}echo "login $#"`,
  ]);
  const helper = starts.find((argv) => argv[2]?.includes("apply"));
  assert.deepEqual(helper.slice(0, 2), ["/bin/sh", "-c"]);
  // Quotes in the directory stay one word; bash and zsh use `builtin`; fish
  // has its own syntax; an unknown shell and a managed call stay plain.
  facade.root = "/w/it's";
  for (const shell of ["/bin/bash", "/usr/bin/zsh"]) {
    facade.shell = shell;
    assert.equal(facade.commandArgv("ls")[3], `${guard("\\builtin", "/w/it'\\''s")}ls`);
  }
  facade.shell = "/usr/bin/fish";
  facade.root = "/w/it's \\";
  assert.deepEqual(facade.commandArgv("ls"), [
    "/usr/bin/fish",
    "-l",
    "-c",
    "builtin cd '/w/it\\'s \\\\'; or exit 1; builtin test (begin; builtin pwd -P; builtin printf x; end | builtin string collect) = '/w/it\\'s \\\\\nx'; or exit 1\nls",
  ]);
  facade.shell = "/usr/bin/nu";
  assert.deepEqual(facade.commandArgv("ls"), ["/usr/bin/nu", "-c", "ls"]);
  facade.shell = "/bin/bash";
  facade.managed = true;
  assert.deepEqual(facade.commandArgv("ls"), ["/bin/bash", "-c", "ls"]);
});

test("a login profile that changes directory does not move the command", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-shell-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  // A "bash" whose startup leaves the directory, as a profile's `cd` does.
  mkdirSync(join(base, "bin"));
  writeFileSync(join(base, "bin", "bash"), `#!/bin/sh\ncd ${base}\nexec /bin/sh "$@"\n`, {
    mode: 0o755,
  });
  const { root, call } = await fixture(t, { shell: join(base, "bin", "bash") });
  assert.equal((await call("RunCommand", { command: "pwd" })).text, `Exit code: 0\n${root}\n`);
  const status = await call("RunCommand", { command: "exit 7" });
  assert.match(status.text, /^Exit code: 7/);
});

test("a profile's cd function cannot move the command", { skip: !hasBash }, async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-shell-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  mkdirSync(join(base, "bin"));
  // A "bash" whose environment defines `cd` (as a profile may) to go elsewhere.
  writeFileSync(
    join(base, "bin", "bash"),
    `#!${hasBash}\ncd() { builtin cd ${base}; }\nexport -f cd\nexec ${hasBash} "$@"\n`,
    { mode: 0o755 },
  );
  const { root, call } = await fixture(t, { shell: join(base, "bin", "bash") });
  assert.equal((await call("RunCommand", { command: "pwd" })).text, `Exit code: 0\n${root}\n`);
});

test("fish runs the login command in its own syntax", { skip: !hasFish }, async (t) => {
  const { root, call } = await fixture(t, { shell: hasFish });
  assert.equal(
    (await call("RunCommand", { command: "pwd; echo (count $argv)" })).text,
    `Exit code: 0\n${root}\n0\n`,
  );
});

test("a quiet command stops polling and ends on the executor's exit notification", async (t) => {
  const { call, endpoint } = await fixture(t);
  const started = Date.now();
  const quiet = await call("RunCommand", { command: "sleep 5; echo done" });
  const elapsed = Date.now() - started;
  assert.match(quiet.text, /^Exit code: 0\n/);
  assert.match(quiet.text, /done/);
  const reads = endpoint.calls.filter((entry) => entry.method === "process/read");
  // Three one-second reads, then a read that does not wait before the
  // notification wait (instead of a one-second read each second).
  assert.deepEqual(
    reads.slice(0, LIMITS.quietReads + 1).map((entry) => entry.params.waitMs > 900),
    [true, true, true, false],
  );
  assert.ok(reads.length <= LIMITS.quietReads + 4, `${reads.length} reads`);
  // The exit is seen at once, not at the end of a quiet wait.
  assert.ok(elapsed < 5000 + 1500, `${elapsed} ms`);
});

test("output ends a quiet wait and resumes one-second reads", async (t) => {
  const { call, endpoint } = await fixture(t);
  const ticking = await call("RunCommand", { command: "sleep 4; echo tick; sleep 2; echo tock" });
  assert.match(ticking.text, /^Exit code: 0\n/);
  assert.match(ticking.text, /tick\ntock/);
  const reads = endpoint.calls.filter((entry) => entry.method === "process/read");
  const waits = reads.map((entry) => entry.params.waitMs > 900);
  // Quiet, quiet, quiet, then notification-driven reads until "tick", then
  // one-second reads again while the command still runs.
  assert.deepEqual(waits.slice(0, 4), [true, true, true, false]);
  assert.ok(waits.slice(4).includes(true), JSON.stringify(waits));
  assert.ok(reads.length <= 10, `${reads.length} reads`);
});

test("a quiet reader starts no read past its wait when notifications are lost", async (t) => {
  const { call, endpoint } = await fixture(t, { notifications: false });
  const started = Date.now();
  const running = await call("RunCommand", { command: "sleep 30", wait_seconds: 5 });
  const until = started + 5000;
  assert.match(running.text, /^Still running \(command_id: /);
  assert.ok(Date.now() - started < 6500, `${Date.now() - started} ms`);
  const reads = endpoint.calls.filter((entry) => entry.method === "process/read");
  assert.equal(reads.filter((entry) => entry.at >= until).length, 0);
});

test("a stop confirmed by another reader ends a quiet reader's wait", async (t) => {
  const { call, facade } = await fixture(t, { notifications: false });
  const started = Date.now();
  const running = call("RunCommand", { command: "sleep 60", wait_seconds: 30 });
  await new Promise((done) => setTimeout(done, 4500));
  const [id] = facade.session("s1").commands.keys();
  assert.match((await call("StopCommand", { command_id: id })).text, /^Stopped /);
  const text = (await running).text;
  assert.match(text, /^Exit code: /);
  // Without the wake it would sleep to the 15 s quiet bound.
  assert.ok(Date.now() - started < 9000, `${Date.now() - started} ms`);
});

test("a connection lost during a quiet wait is an unknown outcome", async (t) => {
  const { call, endpoint, uncertain } = await fixture(t, { notifications: false });
  const running = call("RunCommand", { command: "sleep 60", wait_seconds: 30 });
  await new Promise((done) => setTimeout(done, 4500));
  endpoint.close();
  await running;
  assert.deepEqual(uncertain, [["s1", "process/read"]]);
});

test("an exit whose notification was lost is still seen before the deadline", async (t) => {
  const { call } = await fixture(t, { notifications: false });
  const done = await call("RunCommand", { command: "sleep 4; echo fin", timeout_seconds: 7 });
  assert.match(done.text, /^Exit code: 0\n/);
  assert.match(done.text, /fin/);
});

test("output a stop's drain reads is in its answer", async (t) => {
  const { call } = await fixture(t);
  const first = await call("RunCommand", {
    command: "echo early; sleep 2; echo late; sleep 60",
    wait_seconds: 1,
  });
  const id = /command_id: ([^)]+)\)/.exec(first.text)?.[1];
  assert.ok(id, first.text);
  await new Promise((resolve) => setTimeout(resolve, 3000));
  const stopped = await call("StopCommand", { command_id: id });
  assert.match(stopped.text, /^Stopped /);
  assert.match(stopped.text, /late/);
});

test("ReadFile asks for a file's metadata once", async (t) => {
  const { call, endpoint } = await fixture(t);
  assert.match((await call("ReadFile", { path: "hello.txt" })).text, /hello target/);
  const methods = endpoint.calls.map((entry) => entry.method);
  assert.equal(methods.filter((method) => method === "fs/getMetadata").length, 1);
  assert.equal(methods.filter((method) => method === "fs/readFile").length, 1);
});

test("projection reads guidance and skills concurrently, within a bound, in order", async (t) => {
  const { root, facade, endpoint } = await fixture(t, {
    delay: (method) => (method.startsWith("fs/") ? 50 : 0),
  });
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  mkdirSync(join(root, "src", "deep"), { recursive: true });
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  writeFileSync(join(root, "src", "AGENTS.md"), "SRC_GUIDANCE\n");
  writeFileSync(join(root, "src", "deep", "GEMINI.md"), "DEEP_GUIDANCE\n");
  for (const name of ["alpha", "beta", "gamma", "delta"]) {
    mkdirSync(join(root, ".agents", "skills", name), { recursive: true });
    writeFileSync(
      join(root, ".agents", "skills", name, "SKILL.md"),
      `---\nname: ${name}\ndescription: the ${name} skill\n---\n`,
    );
  }
  const projection = await buildProjection(facade, join(root, "src", "deep"));
  const order = ["ROOT_GUIDANCE", "SRC_GUIDANCE", "DEEP_GUIDANCE"].map((text) =>
    projection.indexOf(text),
  );
  assert.ok(order.every((index, at) => index > 0 && (at === 0 || index > order[at - 1])));
  for (const name of ["alpha", "beta", "gamma", "delta"]) {
    assert.match(projection, new RegExp(`- ${name}: the ${name} skill`));
  }
  assert.ok(endpoint.maxInflight() > 1, "reads overlap");
  assert.ok(endpoint.maxInflight() <= 6, `${endpoint.maxInflight()} in flight`);
});

test("projection reads no guidance once its budget is spent", async (t) => {
  const { root, facade, endpoint } = await fixture(t);
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  const deep = join(root, "a", "b", "c", "d", "e");
  mkdirSync(deep, { recursive: true });
  for (const directory of [
    root,
    join(root, "a"),
    join(root, "a", "b"),
    join(root, "a", "b", "c"),
  ]) {
    writeFileSync(join(directory, "AGENTS.md"), "g".repeat(PROJECTION_LIMITS.guidanceFile + 10));
  }
  writeFileSync(join(deep, "AGENTS.md"), "LATE_GUIDANCE\n");
  const projection = await buildProjection(facade, deep);
  assert.ok(!projection.includes("LATE_GUIDANCE"));
  assert.match(projection, /e\/AGENTS\.md ---\n\[truncated: guidance limit reached\]/);
  const late = endpoint.calls.filter((entry) =>
    String(entry.params?.path ?? "").endsWith("/e/AGENTS.md"),
  );
  assert.equal(late.length, 0);
});

test("a lost mutation reply reports an unknown outcome and ends the turn", async (t) => {
  let lose = false;
  // The publication's helper start loses its reply.
  const { root, call, uncertain } = await fixture(t, {
    lose: (method) => lose && method === "process/start",
  });
  lose = true;
  const result = await call("WriteFile", { path: "lost.txt", content: "maybe\n" });
  assert.equal(result.error, true);
  assert.match(result.text, /^outcome unknown/);
  assert.deepEqual(uncertain, [["s1", "process/start"]]);
  lose = false;
  const after = await call("ReadFile", { path: "hello.txt" });
  assert.match(after.text, /unknown outcome/);
  // The fake never ran the lost start; nothing was published or retried.
  assert.throws(() => readFileSync(join(root, "lost.txt")));
});

test("a lost process start is an unknown outcome too", async (t) => {
  const { call, uncertain } = await fixture(t, { lose: (method) => method === "process/start" });
  assert.match((await call("RunCommand", { command: "true" })).text, /^outcome unknown/);
  assert.deepEqual(uncertain, [["s1", "process/start"]]);
});

test("retained output keeps head and tail", () => {
  const retained = new Retained(10);
  retained.push(Buffer.from("abcdefghijklmnop"));
  assert.equal(retained.take(), "abcde\n[cowboy: 6 bytes of output omitted]\nlmnop");
  assert.equal(retained.take(), "");
});

test("projection fences repository guidance and bounds Git status", async (t) => {
  const { root, facade, base } = await fixture(t);
  const git = (...args) => execFileSync("git", ["-C", root, ...args], { stdio: "pipe" });
  git("init", "-q", "-b", "main");
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  writeFileSync(join(root, "src", "GEMINI.md"), `SUB_GUIDANCE ${"y".repeat(20000)}\n`);
  for (let index = 0; index < 250; index++) writeFileSync(join(root, `u${index}`), "");
  const projection = await buildProjection(facade, join(root, "src"));
  assert.ok(/^<cowboy_routing>/.test(projection));
  assert.ok(/Git branch: main/.test(projection));
  assert.ok(/Git status --short \(200 lines\):/.test(projection));
  assert.ok(/\n\[truncated\]\n/.test(projection));
  const fence = /(COWBOY_UNTRUSTED_[a-f0-9]{32})/.exec(projection)[1];
  const fenced = projection.split(fence);
  assert.equal(fenced.length, 4);
  assert.match(fenced[2], /ROOT_GUIDANCE/);
  assert.match(fenced[2], /SUB_GUIDANCE/);
  assert.ok(fenced[2].length < 64 * 1024);
  // A real AGENTS.md (18-23 KB) keeps its tail.
  assert.match(fenced[2], /y{20000}\n/);
  assert.doesNotMatch(fenced[2], /\[truncated\]/);
  assert.ok(fenced[0].indexOf("ROOT_GUIDANCE") < 0);
  await assert.rejects(
    buildProjection(facade, join(base, "outside")),
    /outside the bound workspace/,
  );
  assert.deepEqual(guidanceDirectories("/r", "/r/a/b"), ["/r", "/r/a", "/r/a/b"]);
  assert.deepEqual(guidanceDirectories(undefined, "/w"), ["/w"]);
  assert.deepEqual(parseGit("top\t/r\nbranch\tdev\nstatus\t M a\n"), {
    top: "/r",
    branch: "dev",
    status: [" M a"],
  });
});

/** A target home beside the workspace, its guidance symlinked into a store. */
function homeFixture(base) {
  const home = join(base, "home");
  const store = join(base, "store");
  mkdirSync(join(home, ".gemini"), { recursive: true });
  mkdirSync(store);
  return { home, store };
}

test("projection reads the user's home guidance first, through its symlinks", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const { home, store } = homeFixture(base);
  writeFileSync(join(store, "AGENTS.md"), "HOME_AGENTS\n");
  symlinkSync(join(store, "AGENTS.md"), join(home, "AGENTS.md"));
  writeFileSync(join(home, ".gemini", "GEMINI.md"), "HOME_GEMINI\n");
  const { root, facade } = await fixture(t, { home });
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const projection = await buildProjection(facade, root);
  const order = [
    `--- ${join(home, "AGENTS.md")} ---\nHOME_AGENTS`,
    `--- ${join(home, ".gemini", "GEMINI.md")} ---\nHOME_GEMINI`,
    `--- ${join(root, "AGENTS.md")} ---\nROOT_GUIDANCE`,
  ].map((text) => projection.indexOf(text));
  assert.ok(
    order.every((index, at) => index > 0 && (at === 0 || index > order[at - 1])),
    order,
  );
  assert.doesNotMatch(projection, new RegExp(join(store, "AGENTS.md")));
  assert.match(projection, /the user's home guidance/);
});

test("a missing home or one that is the repository adds nothing twice", async (t) => {
  const missingHome = await fixture(t, { home: "/nonexistent/cowboy-home" });
  writeFileSync(join(missingHome.root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const plain = await buildProjection(missingHome.facade, missingHome.root);
  assert.match(plain, /ROOT_GUIDANCE/);
  assert.doesNotMatch(plain, /cowboy-home/);
  // The workspace is the home: its AGENTS.md is shown once, as the repository's.
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const { root, facade } = await fixture(t, { home: join(base, "unused") });
  facade.connection.info.userHomeDir = pathToFileURL(root).href;
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const projection = await buildProjection(facade, root);
  assert.equal(projection.split("ROOT_GUIDANCE").length, 2);
});

test("a dangling symlink is refused by its metadata; an unlistable parent refuses creates", async (t) => {
  // The executor reports a dangling link's own metadata, and listing fails.
  const { base, root, call } = await fixture(t, {
    metadataNoFollow: true,
    fail: (method) =>
      method === "fs/readDirectory" ? { code: -32600, message: "permission denied" } : undefined,
  });
  symlinkSync(join(base, "outside", "gone"), join(root, "dangling"));
  const refused = await call("WriteFile", { path: "dangling/x.txt", content: "x" });
  assert.match(refused.text, /symlink whose target does not exist/);
  assert.throws(() => readFileSync(join(base, "outside", "gone", "x.txt")));
  // Absence cannot be proven without the parent's listing: refused.
  const unproven = await call("WriteFile", { path: "fresh/dir/a.txt", content: "a\n" });
  assert.equal(unproven.error, true);
  assert.match(unproven.text, /cannot be listed to confirm fresh is absent/);
  assert.throws(() => readFileSync(join(root, "fresh", "dir", "a.txt")));
  // Replacing an existing file needs no listing.
  await call("ReadFile", { path: "hello.txt" });
  assert.match((await call("WriteFile", { path: "hello.txt", content: "y\n" })).text, /^Replaced/);
});

test("a dangling link is refused when metadata follows links and listing fails", async (t) => {
  const { base, root, call } = await fixture(t, {
    fail: (method) =>
      method === "fs/readDirectory" ? { code: -32600, message: "permission denied" } : undefined,
  });
  symlinkSync(join(base, "outside", "gone"), join(root, "dangling"));
  const refused = await call("WriteFile", { path: "dangling/x.txt", content: "x" });
  assert.equal(refused.error, true);
  assert.throws(() => readFileSync(join(base, "outside", "gone", "x.txt")));
});

test("the directory guard keeps working with a minimal PATH and an exact path", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-shell-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  mkdirSync(join(base, "bin"));
  // A profile leaving no usable PATH, for a bash and for a plain sh.
  const shells = [];
  for (const [name, real] of [
    ["bash", hasBash],
    ["sh", "/bin/sh"],
  ]) {
    if (!real) continue;
    writeFileSync(
      join(base, "bin", name),
      `#!${real}\nexport PATH=/nonexistent\nexec ${real} "$@"\n`,
      { mode: 0o755 },
    );
    shells.push(join(base, "bin", name));
  }
  if (hasFish) shells.push(hasFish);
  const { root, call, facade } = await fixture(t);
  // A root whose final component ends in a newline still matches.
  const odd = join(root, "odd\n");
  mkdirSync(odd);
  facade.root = odd;
  for (const shell of shells) {
    facade.shell = shell;
    const ran = await call("RunCommand", { command: "echo ran" });
    assert.equal(ran.text, "Exit code: 0\nran\n", shell);
  }
});

test("a home that links into the repository gets no symlink handling", async (t) => {
  const { base, root, facade } = await fixture(t);
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  writeFileSync(join(base, "outside", "AGENTS.md"), "OUTSIDE_GUIDANCE\n");
  mkdirSync(join(root, "sub"));
  symlinkSync(join(base, "outside", "AGENTS.md"), join(root, "sub", "AGENTS.md"));
  // The reported home is outside the checkout but resolves into it.
  symlinkSync(join(root, "sub"), join(base, "linked-home"));
  facade.connection.info.userHomeDir = pathToFileURL(join(base, "linked-home")).href;
  assert.doesNotMatch(await buildProjection(facade, root), /OUTSIDE_GUIDANCE/);
});

test("guidance whose decoded text was clipped is marked truncated", async (t) => {
  const { root, facade } = await fixture(t);
  // Exactly the byte bound, but invalid bytes decode to three bytes each.
  writeFileSync(join(root, "AGENTS.md"), Buffer.alloc(PROJECTION_LIMITS.guidanceFile, 0xff));
  const projection = await buildProjection(facade, root);
  assert.match(projection, /\ufffd\n\[truncated\]\n/);
});

test("a home inside the repository is read by the repository's rules", async (t) => {
  const { base, root, facade } = await fixture(t);
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  writeFileSync(join(base, "outside", "AGENTS.md"), "OUTSIDE_GUIDANCE\n");
  // The checkout is the home, and contains another home: both symlinks
  // would be followed for a home outside it.
  symlinkSync(join(base, "outside", "AGENTS.md"), join(root, "AGENTS.md"));
  mkdirSync(join(root, "nested-home"));
  symlinkSync(join(base, "outside", "AGENTS.md"), join(root, "nested-home", "AGENTS.md"));
  // Even its own regular files contribute nothing.
  mkdirSync(join(root, "nested-home", ".gemini"));
  writeFileSync(join(root, "nested-home", ".gemini", "GEMINI.md"), "NESTED_HOME_GEMINI\n");
  for (const home of [root, join(root, "nested-home")]) {
    facade.connection.info.userHomeDir = pathToFileURL(home).href;
    const projection = await buildProjection(facade, root);
    assert.doesNotMatch(projection, /OUTSIDE_GUIDANCE|NESTED_HOME_GEMINI/, home);
    assert.doesNotMatch(projection, /the user's home guidance/, home);
  }
});

test("a managed projection reads no home guidance", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const { home } = homeFixture(base);
  writeFileSync(join(home, "AGENTS.md"), "HOME_AGENTS\n");
  const { root, facade, endpoint } = await fixture(t, { home });
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const projection = await buildProjection(facade, root, { skills: false, home: false });
  assert.match(projection, /ROOT_GUIDANCE/);
  assert.doesNotMatch(projection, /HOME_AGENTS|home guidance/);
  assert.ok(!endpoint.calls.some((entry) => String(entry.params?.path).includes(home)));
});

test("a home containing the repository loads its own links; links into the repository are skipped", async (t) => {
  const { base, root, facade } = await fixture(t);
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  // The normal case: the repository (base/workspace) is under the home (base).
  facade.connection.info.userHomeDir = pathToFileURL(base).href;
  mkdirSync(join(base, "store"));
  writeFileSync(join(base, "store", "AGENTS.md"), "STORE_GUIDANCE\n");
  symlinkSync(join(base, "store", "AGENTS.md"), join(base, "AGENTS.md"));
  // ~/.gemini is a second link into the repository.
  mkdirSync(join(root, "x"));
  writeFileSync(join(root, "x", "GEMINI.md"), "INJECTED_GEMINI\n");
  symlinkSync(join(root, "x"), join(base, ".gemini"));
  const loaded = await buildProjection(facade, root);
  assert.match(loaded, new RegExp(`--- ${join(base, "AGENTS.md")} ---\nSTORE_GUIDANCE`));
  assert.doesNotMatch(loaded, /INJECTED_GEMINI/);
  assert.match(loaded, /the user's home guidance/);
  // A home file linking straight to a repository file is skipped too; the
  // file is still shown as the repository's.
  rmSync(join(base, "AGENTS.md"));
  writeFileSync(join(root, "src", "AGENTS.md"), "NESTED_GUIDANCE\n");
  symlinkSync(join(root, "src", "AGENTS.md"), join(base, "AGENTS.md"));
  const linked = await buildProjection(facade, join(root, "src"));
  assert.equal(linked.split("NESTED_GUIDANCE").length, 2);
  assert.match(linked, new RegExp(`--- ${join(root, "src", "AGENTS.md")} ---\nNESTED_GUIDANCE`));
  assert.doesNotMatch(linked, /the user's home guidance/);
});

test("home reads stop at their deadline and at the discovery budget", async (t) => {
  // The fake sender never settles and the deadline timer is unreferenced:
  // keep the event loop alive so the test is not cancelled early.
  const keepAlive = setInterval(() => {}, 1000);
  t.after(() => clearInterval(keepAlive));
  const sent = [];
  const never = (method) => {
    sent.push(method);
    return new Promise(() => {});
  };
  const reads = HomeReads.for(never, "/h", "/r", { deadlineMs: 50, budgetMs: 1000 });
  const started = Date.now();
  await assert.rejects(reads.call("fs/getMetadata", {}), /did not answer in time/);
  assert.ok(Date.now() - started < 500);
  // Stalled: nothing more is sent.
  await assert.rejects(reads.call("fs/getMetadata", {}), /did not answer in time/);
  assert.equal(sent.length, 1);
  // Time between home reads (repository reads) does not use the budget.
  const quick = async () => ({ path: "file:///x" });
  const idle = HomeReads.for(quick, "/h", "/r", { deadlineMs: 50, budgetMs: 80 });
  await idle.call("fs/getMetadata", {});
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.deepEqual(await idle.call("fs/getMetadata", {}), { path: "file:///x" });
  // Concurrent home reads share the wall time instead of each adding theirs.
  const pair = () => new Promise((resolve) => setTimeout(() => resolve({ path: "file:///x" }), 45));
  const shared = HomeReads.for(pair, "/h", "/r", { deadlineMs: 100, budgetMs: 80 });
  await Promise.all([shared.call("fs/getMetadata", {}), shared.call("fs/getMetadata", {})]);
  assert.ok(shared.elapsed() < 80, `${shared.elapsed()} ms`);
  // Time spent waiting on home reads does: a slow home exhausts it.
  const slow = () => new Promise((resolve) => setTimeout(() => resolve({ path: "file:///x" }), 45));
  const budgeted = HomeReads.for(slow, "/h", "/r", { deadlineMs: 100, budgetMs: 80 });
  await budgeted.call("fs/getMetadata", {});
  await assert.rejects(budgeted.call("fs/getMetadata", {}), /did not answer in time/);
  await assert.rejects(budgeted.call("fs/getMetadata", {}), /did not answer in time/);
});

test("a stalled home read gives up within its deadline", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const { home } = homeFixture(base);
  writeFileSync(join(home, "AGENTS.md"), "HOME_AGENTS\n");
  const { root, facade } = await fixture(t, {
    home,
    delay: (method, params) => (String(params?.path ?? "").includes(home) ? 30_000 : 0),
  });
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const started = Date.now();
  const projection = await buildProjection(facade, root);
  assert.ok(Date.now() - started < 3000 + 2000, `${Date.now() - started} ms`);
  assert.match(projection, /ROOT_GUIDANCE/);
  assert.doesNotMatch(projection, /HOME_AGENTS/);
  // The shared connection is still usable.
  assert.match(
    (await facade.call("s1", "ReadFile", { path: "hello.txt" })).content[0].text,
    /hello/,
  );
});

test("a component that appears while a write resolves is resolved again", async (t) => {
  let hidden = true;
  const { root, call } = await fixture(t, {
    // The first canonicalize of the new directory misses it, as if it was
    // created just after.
    fail: (method, params) => {
      if (method === "fs/canonicalize" && hidden && String(params.path).endsWith("/appeared")) {
        hidden = false;
        return { code: -32004, message: "No such file or directory (os error 2)" };
      }
      return undefined;
    },
  });
  mkdirSync(join(root, "appeared"));
  const written = await call("WriteFile", { path: "appeared/a.txt", content: "a\n" });
  assert.match(written.text, /^Created/);
  assert.equal(readFileSync(join(root, "appeared", "a.txt"), "utf8"), "a\n");
});

test("guidance is cut at a character boundary within its byte bounds", async (t) => {
  const { root, facade } = await fixture(t);
  const limit = PROJECTION_LIMITS.guidanceFile;
  // Invalid bytes decode to U+FFFD (three bytes each); a two-byte character
  // straddles the cut.
  writeFileSync(join(root, "AGENTS.md"), Buffer.alloc(limit + 10, 0xff));
  writeFileSync(join(root, "GEMINI.md"), `a${"\u00e9".repeat(limit)}`);
  const projection = await buildProjection(facade, root);
  const fence = /(COWBOY_UNTRUSTED_[a-f0-9]{32})/.exec(projection)[1];
  const [, agents, gemini] = projection.split(fence)[2].split(/\n--- [^\n]+ ---\n/);
  for (const text of [agents, gemini]) {
    const body = text.replace(/\n\[truncated\]\n?$/, "");
    assert.ok(Buffer.byteLength(body) <= limit, `${Buffer.byteLength(body)}`);
  }
  assert.doesNotMatch(gemini, /\ufffd/);
  assert.ok(Buffer.byteLength(gemini) > limit - 4);
});

test("home guidance shares the guidance budget", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const { home } = homeFixture(base);
  const big = "h".repeat(PROJECTION_LIMITS.guidanceFile + 10);
  writeFileSync(join(home, "AGENTS.md"), big);
  writeFileSync(join(home, ".gemini", "GEMINI.md"), big);
  const { root, facade } = await fixture(t, { home });
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  writeFileSync(join(root, "AGENTS.md"), "r".repeat(PROJECTION_LIMITS.guidanceFile + 10));
  writeFileSync(join(root, "GEMINI.md"), "LATE_GUIDANCE\n");
  const projection = await buildProjection(facade, root);
  assert.ok(!projection.includes("LATE_GUIDANCE"));
  assert.match(projection, /GEMINI\.md ---\n\[truncated: guidance limit reached\]/);
  const fence = /(COWBOY_UNTRUSTED_[a-f0-9]{32})/.exec(projection)[1];
  const fenced = projection.split(fence)[2];
  assert.ok(fenced.length < PROJECTION_LIMITS.guidanceTotal + 4096, `${fenced.length}`);
  assert.equal(fenced.split("\n[truncated]").length, 4);
});

test("guidance is read through fs/readFile even when the executor has no handles", async (t) => {
  const { facade, root } = await fixture(t, {
    fail: (method) =>
      method === "fs/open" ? { code: -32601, message: "method not found" } : undefined,
  });
  writeFileSync(join(root, "AGENTS.md"), "ROOT_GUIDANCE\n");
  const projection = await buildProjection(facade, root);
  assert.match(projection, /<repository_guidance>/);
  assert.match(projection, /ROOT_GUIDANCE/);
});

test("a worker-reported unknown outcome fences the turn at once", async (t) => {
  const { facade, call, uncertain, endpoint } = await fixture(t, {
    fail: (method) =>
      method === "process/start"
        ? { code: -32000, message: "result unknown", data: { outcome: "unknown" } }
        : undefined,
  });
  const turn = facade.session("s1").turn;
  const written = await call("WriteFile", { path: "new.txt", content: "x" });
  assert.equal(written.error, true);
  assert.match(written.text, /outcome unknown/);
  assert.deepEqual(uncertain, [["s1", "process/start"]]);
  assert.equal(turn.uncertain, "process/start");
  // Recorded by the worker's ledger before the tool answered.
  assert.ok(
    endpoint.calls.some(
      (entry) =>
        entry.method === "cowboy/uncertainEffect" && entry.params.method === "process/start",
    ),
  );
  await assert.rejects(facade.mutate(turn, "fs/createDirectory", {}), ToolError);
});

test("a normally ended turn admits no further mutation from a running tool", async (t) => {
  const { facade } = await fixture(t);
  const turn = facade.session("s1").turn;
  await facade.endTurn("s1");
  assert.equal(turn.ended, true);
  await assert.rejects(facade.mutate(turn, "fs/createDirectory", {}), ToolError);
});

test("a command's deadline holds without polling; an unconfirmed stop keeps it", async (t) => {
  const { facade, call } = await fixture(t);
  const started = await call("RunCommand", {
    command: "sleep 30",
    timeout_seconds: 1,
    wait_seconds: 0,
  });
  assert.match(started.text, /Still running/);
  assert.match(started.text, /Continue it with wait_seconds 1 to wait for the rest of its 1 s/);
  const session = facade.session("s1");
  assert.equal(session.commands.size, 1);
  const deadline = Date.now() + 8000;
  while (session.commands.size && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.equal(session.commands.size, 0, "the expired command was stopped");
  const stubborn = await call("RunCommand", {
    command: "trap '' TERM; echo ready; sleep 8",
    wait_seconds: 1,
  });
  assert.match(stubborn.text, /ready/);
  const id = /command_id: (\S+)\)/.exec(stubborn.text)[1];
  const stopped = await call("StopCommand", { command_id: id });
  assert.match(stopped.text, /not yet confirmed/);
  assert.equal(session.commands.has(id), true);
});

test("EditFile refuses invalid UTF-8 and oversized results before allocating", async (t) => {
  const { root, call } = await fixture(t);
  writeFileSync(join(root, "latin1.txt"), Buffer.from([0x63, 0x61, 0x66, 0xe9, 0x0a]));
  await call("ReadFile", { path: "latin1.txt" });
  const edit = await call("EditFile", { path: "latin1.txt", old_string: "caf", new_string: "x" });
  assert.equal(edit.error, true);
  assert.match(edit.text, /not valid UTF-8/);
  assert.deepEqual(
    readFileSync(join(root, "latin1.txt")),
    Buffer.from([0x63, 0x61, 0x66, 0xe9, 0x0a]),
  );
  writeFileSync(join(root, "many.txt"), "a".repeat(768 * 1024));
  await call("ReadFile", { path: "many.txt" });
  const grown = await call("EditFile", {
    path: "many.txt",
    old_string: "a",
    new_string: "bbbbb",
    replace_all: true,
  });
  assert.equal(grown.error, true);
  assert.match(grown.text, /larger than 3 MiB/);
});

test("publication is the execution helper's conditional apply; the original stays on conflict", async (t) => {
  const { root, call, endpoint } = await fixture(t);
  assert.match((await call("ReadFile", { path: "hello.txt" })).text, /hello target/);
  // Someone else changes the file after it was read: nothing is replaced.
  writeFileSync(join(root, "hello.txt"), "theirs\n");
  const conflict = await call("EditFile", {
    path: "hello.txt",
    old_string: "theirs",
    new_string: "mine",
  });
  assert.equal(conflict.error, true);
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "theirs\n");
  assert.match((await call("ReadFile", { path: "hello.txt" })).text, /theirs/);
  const edited = await call("EditFile", {
    path: "hello.txt",
    old_string: "theirs",
    new_string: "mine",
  });
  assert.equal(edited.error, false, edited.text);
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "mine\n");
  // A new file in a new directory is created by the same primitive.
  const created = await call("WriteFile", { path: "new/dir/file.txt", content: "fresh\n" });
  assert.equal(created.error, false, created.text);
  assert.equal(readFileSync(join(root, "new/dir/file.txt"), "utf8"), "fresh\n");
  // Every publication ran the helper (the stale edit was refused before
  // it); no staged file is left in the tree.
  const applies = endpoint.calls.filter(
    (entry) => entry.method === "process/start" && entry.params.argv[2]?.includes("apply"),
  );
  assert.equal(applies.length, 2);
  assert.deepEqual(execFileSync("find", [root, "-name", "*.tmp"]).toString().trim(), "");
});

test("a missing execution helper writes nothing", async (t) => {
  const { root, call } = await fixture(t, { fileHelper: null });
  await call("ReadFile", { path: "hello.txt" });
  const refused = await call("WriteFile", { path: "hello.txt", content: "x" });
  assert.equal(refused.error, true);
  assert.match(refused.text, /execution helper is unavailable/);
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "hello target\n");
});

test("EditFile keeps a UTF-8 byte order mark", async (t) => {
  const { root, call } = await fixture(t);
  writeFileSync(join(root, "bom.txt"), Buffer.from("\ufeffkeep me\n"));
  await call("ReadFile", { path: "bom.txt" });
  const edited = await call("EditFile", {
    path: "bom.txt",
    old_string: "keep",
    new_string: "kept",
  });
  assert.equal(edited.error, false, edited.text);
  const bytes = readFileSync(join(root, "bom.txt"));
  assert.deepEqual([...bytes.subarray(0, 3)], [0xef, 0xbb, 0xbf]);
  assert.equal(bytes.subarray(3).toString(), "kept me\n");
  assert.equal(decodeText(Buffer.from("\ufeffa")), "\ufeffa");
});

test("a splice keeps the shared prefix and suffix", () => {
  const sha = (text) => createHash("sha256").update(text).digest("hex");
  assert.deepEqual(splice(Buffer.from("same"), Buffer.from("same")), []);
  const [one] = splice(Buffer.from("hello world"), Buffer.from("hello there world"));
  assert.deepEqual([one.offset, one.delete], [6, 0]);
  assert.equal(Buffer.from(one.insert, "base64").toString(), "there ");
  assert.equal(sha("x").length, 64);
});

test("RunCommand validates wait_seconds before it starts anything", async (t) => {
  const { call, endpoint } = await fixture(t);
  for (const wait_seconds of [-1, 1.5, 100000]) {
    const refused = await call("RunCommand", { command: "true", wait_seconds });
    assert.equal(refused.error, true);
    assert.match(refused.text, /wait_seconds/);
  }
  assert.equal(endpoint.calls.filter((entry) => entry.method === "process/start").length, 0);
});

test("guidance is read only through symlink-free paths, by a readiness probe", async (t) => {
  const { base, root, facade, endpoint } = await fixture(t);
  writeFileSync(join(base, "outside", "AGENTS.md"), "OUTSIDE_GUIDANCE\n");
  symlinkSync(join(base, "outside", "AGENTS.md"), join(root, "AGENTS.md"));
  writeFileSync(join(root, "GEMINI.md"), "inside guidance\n");
  const projection = await buildProjection(facade, root);
  assert.doesNotMatch(projection, /OUTSIDE_GUIDANCE/);
  assert.match(projection, /inside guidance/);
  const starts = endpoint.calls.filter((call) => call.method === "process/start");
  assert.ok(starts.length > 0);
  for (const start of starts) assert.match(start.params.processId, /^cowboy-agy-readiness-/);
});

test("a busy reader never disables a command's deadline", async (t) => {
  const { facade } = await fixture(t);
  const session = facade.session("s1");
  let terminated = 0;
  facade.terminate = async () => {
    terminated++;
    return true;
  };
  const running = { processId: "p1", closed: false, busy: true };
  session.commands.set("p1", running);
  await facade.expire(session, running);
  // Busy: the deadline timer is armed again, not dropped.
  assert.ok(running.timer);
  assert.equal(terminated, 0);
  clearTimeout(running.timer);
  running.busy = false;
  await facade.expire(session, running);
  assert.equal(terminated, 1);
  assert.equal(session.commands.has("p1"), false);
});

test("the largest admitted new file fits one execution invocation after double encoding", () => {
  // Invocation::validate admits at most MAX_FRAME_BYTES / 2 (7 MiB) of JSON.
  const invocationLimit = 7 * 1024 * 1024;
  const bytes = Buffer.alloc(LIMITS.writeBytes, 0xff);
  const request = JSON.stringify({
    schema: 1,
    path: `/${"p".repeat(4096)}`,
    base: "absent",
    expect: "0".repeat(64),
    splices: splice(Buffer.alloc(0), bytes),
    staleTempSeconds: 3600,
  });
  const encoded = Buffer.from(request).toString("base64").length;
  assert.ok(encoded + 64 * 1024 < invocationLimit, `${encoded} bytes leave no framing room`);
});

test("the readiness probe is exactly the script the worker's ledger admits", () => {
  const ledger = readFileSync(
    new URL("../../../../src/worker_execution/ledger.rs", import.meta.url),
    "utf8",
  );
  const admitted = ledger.match(/READINESS_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1];
  assert.equal(admitted, READINESS_PROBE);
  assert.match(READINESS_PROBE, new RegExp(`head -n ${PROJECTION_LIMITS.statusLines + 1} `));
});

test("a managed facade offers no write tools and refuses every other name", async (t) => {
  const { root, facade, call } = await fixture(t, {}, { managed: true });
  facade.extensions = {
    tools: () => [{ name: "cowboy_execute", inputSchema: { type: "object" } }],
    has: () => true,
    call: async () => ({ content: [{ type: "text", text: "EXTENSION_RAN" }] }),
  };
  const tools = facade.tools();
  assert.deepEqual(
    tools.map((tool) => tool.name),
    ["ReadFile", "ListDirectory", "SearchFiles", "RunCommand", "UpdatePlan"],
  );
  assert.doesNotMatch(tools.find((tool) => tool.name === "RunCommand").description, /StopCommand/);
  assert.match((await call("ReadFile", { path: "hello.txt" })).text, /hello target/);
  for (const [name, args] of [
    ["WriteFile", { path: "new.txt", content: "x" }],
    ["EditFile", { path: "hello.txt", old_string: "hello", new_string: "bye" }],
    ["StopCommand", { command_id: "x" }],
    ["cowboy_execute", {}],
  ]) {
    const answer = await call(name, args);
    assert.equal(answer.error, true, name);
    assert.match(answer.text, /not available in a read-only Cowboy managed call/, name);
  }
  assert.equal(readFileSync(join(root, "hello.txt"), "utf8"), "hello target\n");
  assert.match((await call("RunCommand", { command: "echo managed" })).text, /managed/);
});
