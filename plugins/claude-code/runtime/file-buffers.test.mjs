import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  utimes,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { bufferSplice, fileBufferMatcher, WorkspaceTools } from "./tools.mjs";

const fileHelper = fileURLToPath(
  new URL("../../../target/debug/cowboy-execution-host", import.meta.url),
);

// A real target directory behind the executor calls, with Cowboy's helper
// run for every target-local utility.
async function fixture(t, { policy } = {}) {
  const root = await mkdtemp(join(tmpdir(), "cowboy-file-buffers-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const target = join(root, "target");
  const home = join(root, "home");
  const local = join(root, "local");
  await mkdir(target);
  await mkdir(home);
  await mkdir(local, { mode: 0o700 });
  const calls = [];
  const connection = {
    info: { userHomeDir: pathToFileURL(home).href },
    async call(method, params) {
      calls.push(method);
      const path = fileURLToPath(params.path);
      if (method === "fs/getMetadata") {
        const info = await stat(path).catch((error) => {
          error.remote = { code: -32000, message: "No such file" };
          throw error;
        });
        return { isFile: info.isFile(), size: info.size };
      }
      if (method === "fs/readFile") {
        return { dataBase64: (await readFile(path)).toString("base64") };
      }
      if (method === "fs/writeFile") {
        await writeFile(path, Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (method === "fs/createDirectory") {
        await mkdir(path, { recursive: true });
        return {};
      }
      if (method === "fs/remove") {
        await rm(path, { force: true });
        return {};
      }
      throw new Error(`Unexpected ${method}`);
    },
  };
  const binding = {
    workspace: { cwd: target },
    environment: { id: "fixture" },
  };
  const tools = new WorkspaceTools(
    connection,
    binding,
    join(local, "state.json"),
  );
  const policyPath = join(local, "file-buffers.json");
  if (policy) await writeFile(policyPath, JSON.stringify(policy));
  tools.useFileBuffers(policy ? policyPath : undefined);
  await tools.load();
  tools.fileHelper = fileHelper;
  tools.shell = "/bin/sh";
  const applies = [];
  // Ended commands run their script directly; the wrapper's end lines are
  // the collector's concern, covered by the native probe.
  tools.command = async (argv, _timeout, _call, options) => {
    const run = options?.end ? argv.slice(5) : argv;
    const script = run.join(" ");
    const kind = run[1] === "apply" ? "apply" : "shell";
    if (kind === "apply") {
      applies.push(
        /--request-file/.test(script) ? "--request-file" : "--request",
      );
    }
    calls.push(`command:${kind}`);
    const output = execFileSync(run[0], run.slice(1), { encoding: "utf8" });
    return { exitCode: 0, exited: true, closed: true, output };
  };
  const buffers = () =>
    readdir(join(local, "file-buffers")).catch(() => []).then((names) =>
      names.filter((name) => /^[a-f0-9]{64}$/.test(name))
    );
  return {
    tools,
    target,
    home,
    local,
    calls,
    applies,
    buffers,
    policyPath,
    connection,
    binding,
  };
}

const POLICY = {
  schema: 1,
  enabled: true,
  max_file_size: 1024 * 1024,
  excluded: ["*.pem", "**/secret/**"],
  cache_max_bytes: 64 * 1024 * 1024,
  cache_max_age_seconds: 3600,
  target_temp_max_age_seconds: 3600,
};

test("edits after a read send only a hash-checked splice", async (t) => {
  const { tools, target, calls, applies, buffers } = await fixture(t, {
    policy: POLICY,
  });
  const file = join(target, "main.rs");
  const original = "fn main() {\n" + "    // filler\n".repeat(2000) + "}\n";
  await writeFile(file, original);
  await tools.call("read", { file_path: "main.rs" });
  assert.equal((await buffers()).length, 1);
  calls.length = 0;
  for (
    const [old_string, new_string] of [["fn main", "fn start"], [
      "}\n",
      "    run();\n}\n",
    ]]
  ) {
    const result = await tools.call("edit", {
      file_path: "main.rs",
      old_string,
      new_string,
    });
    assert.equal(result.isError, false, result.content[0].text);
  }
  const expected = original.replace("fn main", "fn start").replace(
    /}\n$/,
    "    run();\n}\n",
  );
  assert.equal(await readFile(file, "utf8"), expected);
  // No whole-file transfer in either direction, and both edits inline.
  assert.deepEqual(calls, ["command:apply", "command:apply"]);
  assert.deepEqual(applies, ["--request", "--request"]);
  assert.equal(tools.fileBufferStats.hits, 2);
  assert.ok(tools.fileBufferStats.sentBytes < 2048);
  // The written result is the next edit's base; the old copies remain until
  // the cache evicts them.
  assert.ok((await buffers()).length >= 2);
});

test("a target changed since the buffered read refuses the edit untouched", async (t) => {
  const { tools, target, calls } = await fixture(t, { policy: POLICY });
  const file = join(target, "a.txt");
  await writeFile(file, "one\n");
  await tools.call("read", { file_path: "a.txt" });
  await writeFile(file, "changed by a linter\n");
  calls.length = 0;
  const refused = await tools.call("edit", {
    file_path: "a.txt",
    old_string: "one",
    new_string: "two",
  });
  assert.equal(refused.isError, true);
  assert.match(refused.content[0].text, /modified since read/);
  assert.equal(await readFile(file, "utf8"), "changed by a linter\n");
  assert.deepEqual(calls, ["command:apply"]);
  // Read again, the edit applies to what the target now holds.
  await tools.call("read", { file_path: "a.txt" });
  const edited = await tools.call("edit", {
    file_path: "a.txt",
    old_string: "linter",
    new_string: "formatter",
  });
  assert.equal(edited.isError, false);
  assert.equal(await readFile(file, "utf8"), "changed by a formatter\n");
});

test("excluded, oversized and disabled files keep whole-file transfers", async (t) => {
  const { tools, target, calls, buffers, policyPath } = await fixture(t, {
    policy: { ...POLICY, max_file_size: 4096 },
  });
  await mkdir(join(target, "secret"));
  for (const name of ["key.PEM", "secret/notes.txt", "big.txt"]) {
    await writeFile(
      join(target, name),
      name === "big.txt" ? "x".repeat(8192) : "value\n",
    );
    await tools.call("read", { file_path: name });
    calls.length = 0;
    const result = await tools.call("write", {
      file_path: name,
      content: "replaced\n",
    });
    assert.equal(result.isError, false);
    assert.deepEqual(calls, [
      "fs/getMetadata",
      "fs/readFile",
      "fs/createDirectory",
      "fs/writeFile",
    ], name);
    assert.equal(await readFile(join(target, name), "utf8"), "replaced\n");
  }
  // Neither the secrets nor the oversized file were copied locally; the
  // small replacements of the excluded files were not either.
  const local = await buffers();
  assert.equal(local.length, 1);
  // Disabling applies without a restart, and so does a broken projection.
  for (const next of [{ ...POLICY, enabled: false }, { schema: 2 }]) {
    await writeFile(policyPath, JSON.stringify(next));
    await utimes(policyPath, new Date(), new Date(Date.now() + 1000));
    await writeFile(join(target, "plain.txt"), "a\n");
    await tools.call("read", { file_path: "plain.txt" });
    calls.length = 0;
    await tools.call("edit", {
      file_path: "plain.txt",
      old_string: "a",
      new_string: "b",
    });
    assert.ok(calls.includes("fs/writeFile"), JSON.stringify(next));
    assert.ok(!calls.includes("command:apply"));
  }
});

test("a lost apply reply is resolved by the target's bytes, never reapplied", async (t) => {
  const { tools, target, calls } = await fixture(t, { policy: POLICY });
  const file = join(target, "f.txt");
  await writeFile(file, "base\n");
  await tools.call("read", { file_path: "f.txt" });
  const run = tools.command;
  // The helper applied, but its reply was lost.
  tools.command = async (...args) => {
    await run(...args);
    throw new Error("connection lost");
  };
  calls.length = 0;
  let result = await tools.call("edit", {
    file_path: "f.txt",
    old_string: "base",
    new_string: "next",
  });
  assert.equal(result.isError, false);
  assert.equal(await readFile(file, "utf8"), "next\n");
  assert.ok(!calls.includes("fs/writeFile"));
  // The start was lost before the helper ran: the target still holds the
  // base, so the whole-file write applies it once.
  tools.command = async () => {
    throw new Error("connection lost");
  };
  calls.length = 0;
  result = await tools.call("edit", {
    file_path: "f.txt",
    old_string: "next",
    new_string: "last",
  });
  assert.equal(result.isError, false);
  assert.equal(await readFile(file, "utf8"), "last\n");
  assert.ok(calls.includes("fs/writeFile"));
  // An unknown outcome over a concurrent change is refused.
  await writeFile(file, "theirs\n");
  result = await tools.call("edit", {
    file_path: "f.txt",
    old_string: "last",
    new_string: "mine",
  });
  assert.match(result.content[0].text, /modified since read/);
  assert.equal(await readFile(file, "utf8"), "theirs\n");
});

test("a file deleted since its read is recreated by Write and refused by Edit", async (t) => {
  const { tools, target, calls } = await fixture(t, { policy: POLICY });
  const file = join(target, "gone.txt");
  await writeFile(file, "first\n");
  await tools.call("read", { file_path: "gone.txt" });
  await rm(file);
  let result = await tools.call("edit", {
    file_path: "gone.txt",
    old_string: "first",
    new_string: "second",
  });
  assert.equal(result.isError, true);
  assert.match(result.content[0].text, /File does not exist/);
  assert.equal(await stat(file).catch((error) => error.code), "ENOENT");
  calls.length = 0;
  result = await tools.call("write", {
    file_path: "gone.txt",
    content: "again\n",
  });
  assert.equal(result.isError, false, result.content[0].text);
  assert.equal(await readFile(file, "utf8"), "again\n");
  // The target refused the splice over an absent file; a second
  // compare-and-swap against its absence created it, as natively.
  assert.deepEqual(calls, ["command:apply", "command:apply"]);
  const edited = await tools.call("edit", {
    file_path: "gone.txt",
    old_string: "again",
    new_string: "edited",
  });
  assert.equal(edited.isError, false);
  assert.equal(await readFile(file, "utf8"), "edited\n");
  // The whole directory gone: the helper cannot lock it, and the target's
  // own state settles it the same way.
  const nested = join(target, "deep", "nested.txt");
  await mkdir(join(target, "deep"));
  await writeFile(nested, "n\n");
  await tools.call("read", { file_path: "deep/nested.txt" });
  await rm(join(target, "deep"), { recursive: true });
  result = await tools.call("edit", {
    file_path: "deep/nested.txt",
    old_string: "n",
    new_string: "m",
  });
  assert.match(result.content[0].text, /File does not exist/);
  assert.equal(await stat(join(target, "deep")).catch((e) => e.code), "ENOENT");
  result = await tools.call("write", {
    file_path: "deep/nested.txt",
    content: "m\n",
  });
  assert.equal(result.isError, false, result.content[0].text);
  assert.equal(await readFile(nested, "utf8"), "m\n");
});

test("a Write of identical content still touches the file, as natively", async (t) => {
  const { tools, target, calls } = await fixture(t, { policy: POLICY });
  const file = join(target, "same.txt");
  await writeFile(file, "same\n");
  await tools.call("read", { file_path: "same.txt" });
  const before = (await stat(file)).mtimeMs;
  await new Promise((resolve) => setTimeout(resolve, 20));
  calls.length = 0;
  const result = await tools.call("write", {
    file_path: "same.txt",
    content: "same\n",
  });
  assert.equal(result.isError, false);
  assert.deepEqual(calls, ["command:apply"]);
  assert.ok((await stat(file)).mtimeMs > before);
  assert.equal(await readFile(file, "utf8"), "same\n");
});

test("switching buffers off deletes the copies at the next file operation", async (t) => {
  const { tools, target, buffers, policyPath } = await fixture(t, {
    policy: POLICY,
  });
  await writeFile(join(target, "a.txt"), "a\n");
  await tools.call("read", { file_path: "a.txt" });
  assert.equal((await buffers()).length, 1);
  await writeFile(policyPath, JSON.stringify({ ...POLICY, enabled: false }));
  await utimes(policyPath, new Date(), new Date(Date.now() + 1000));
  tools.fileBufferSweptAt = 0;
  await tools.call("read", { file_path: "a.txt" });
  assert.deepEqual(await buffers(), []);
  // A pattern with too many `**` spans disables buffers rather than running
  // an unbounded search per file.
  await writeFile(
    policyPath,
    JSON.stringify({ ...POLICY, excluded: ["**/".repeat(9) + "x"] }),
  );
  await utimes(policyPath, new Date(), new Date(Date.now() + 2000));
  assert.equal((await tools.fileBufferPolicy()).enabled, false);
});

test("a helper without apply falls back once and stays on whole files", async (t) => {
  const { tools, target, calls } = await fixture(t, { policy: POLICY });
  let refused = 0;
  tools.command = async () => {
    refused++;
    return { exitCode: 2, exited: true, closed: true, output: "usage" };
  };
  const file = join(target, "f.txt");
  await writeFile(file, "a\n");
  await tools.call("read", { file_path: "f.txt" });
  for (const [from, to] of [["a", "b"], ["b", "c"]]) {
    const result = await tools.call("edit", {
      file_path: "f.txt",
      old_string: from,
      new_string: to,
    });
    assert.equal(result.isError, false);
  }
  assert.equal(await readFile(file, "utf8"), "c\n");
  assert.equal(refused, 1);
  assert.ok(calls.filter((call) => call === "fs/writeFile").length === 2);
});

test("a corrupt or evicted copy is read again instead of trusted", async (t) => {
  const { tools, target, local, buffers, calls } = await fixture(t, {
    policy: POLICY,
  });
  const file = join(target, "f.txt");
  await writeFile(file, "hello\n");
  await tools.call("read", { file_path: "f.txt" });
  const [name] = await buffers();
  await writeFile(join(local, "file-buffers", name), "tampered\n");
  calls.length = 0;
  const result = await tools.call("edit", {
    file_path: "f.txt",
    old_string: "hello",
    new_string: "world",
  });
  assert.equal(result.isError, false);
  assert.equal(await readFile(file, "utf8"), "world\n");
  assert.deepEqual(calls, ["fs/getMetadata", "fs/readFile", "command:apply"]);
  assert.ok(!(await buffers()).includes(name));
});

test("new files, parents, large edits and binary bytes go through apply", async (t) => {
  const { tools, target, home, applies } = await fixture(t, {
    policy: POLICY,
  });
  let result = await tools.call("write", {
    file_path: "deep/new/file.txt",
    content: "created\n",
  });
  assert.equal(result.isError, false);
  assert.equal(
    await readFile(join(target, "deep/new/file.txt"), "utf8"),
    "created\n",
  );
  // A large replacement travels in a payload file the helper consumes.
  const large = "y".repeat(200 * 1024);
  result = await tools.call("write", {
    file_path: "deep/new/file.txt",
    content: large,
  });
  assert.equal(result.isError, false);
  assert.equal(
    await readFile(join(target, "deep/new/file.txt"), "utf8"),
    large,
  );
  assert.deepEqual(applies, ["--request", "--request-file"]);
  assert.deepEqual(
    await readdir(join(home, ".cache/cowboy/file-buffers")),
    [],
  );
  // CRLF and a BOM survive a splice byte for byte.
  const crlf = Buffer.from("﻿one\r\ntwo\r\n");
  await writeFile(join(target, "crlf.txt"), crlf);
  await tools.call("read", { file_path: "crlf.txt" });
  result = await tools.call("edit", {
    file_path: "crlf.txt",
    old_string: "two",
    new_string: "three",
  });
  assert.equal(result.isError, false);
  assert.deepEqual(
    await readFile(join(target, "crlf.txt")),
    Buffer.from("﻿one\r\nthree\r\n"),
  );
});

test("copies are bounded by size and age, and dead writers' temporaries go", async (t) => {
  const { tools, local, buffers } = await fixture(t, {
    policy: { ...POLICY, cache_max_bytes: 3000 },
  });
  const policy = await tools.fileBufferPolicy();
  for (const fill of ["a", "b", "c", "d"]) {
    await tools.fileBufferPut(Buffer.from(fill.repeat(1000)), policy);
    await new Promise((resolve) => setTimeout(resolve, 15));
  }
  // Stores between sweeps may exceed the budget briefly; a sweep restores it
  // by dropping the least recently used copy.
  await tools.fileBufferSweep(policy);
  assert.equal((await buffers()).length, 3);
  assert.ok(
    !(await buffers()).includes(
      createHash("sha256").update("a".repeat(1000)).digest("hex"),
    ),
  );
  const directory = join(local, "file-buffers");
  const dead = `${"0".repeat(64)}.999999999.${crypto.randomUUID()}`;
  await writeFile(join(directory, dead), "partial");
  const old = new Date(Date.now() - 7200 * 1000);
  for (const name of await buffers()) {
    await utimes(join(directory, name), old, old);
  }
  await tools.fileBufferSweep(policy);
  assert.deepEqual(await readdir(directory), []);
  // Disabled buffers delete every copy at the next sweep.
  await tools.fileBufferPut(Buffer.from("kept"), policy);
  await tools.fileBufferSweep({ ...policy, enabled: false });
  assert.deepEqual(await buffers(), []);
});

test("read-only sessions and sessions without a helper keep no copies", async (t) => {
  const { tools, target, buffers } = await fixture(t, { policy: POLICY });
  tools.fileHelper = undefined;
  await writeFile(join(target, "f.txt"), "x\n");
  await tools.call("read", { file_path: "f.txt" });
  assert.deepEqual(await buffers(), []);
  tools.fileHelper = fileHelper;
  tools.readOnly = true;
  await tools.call("read", { file_path: "f.txt" });
  assert.deepEqual(await buffers(), []);
});

test("an older Machine without a projection uses the documented defaults", async (t) => {
  const { tools } = await fixture(t);
  const policy = await tools.fileBufferPolicy();
  assert.equal(policy.enabled, true);
  assert.equal(policy.max_file_size, 2 * 1024 * 1024);
  assert.equal(await tools.fileBuffersFor("/x/.env"), undefined);
  assert.equal(await tools.fileBuffersFor("/home/u/.ssh/config"), undefined);
  assert.ok(await tools.fileBuffersFor("/x/src/main.rs"));
  tools.useFileBuffers(join(tmpdir(), "cowboy-missing-policy.json"));
  assert.equal((await tools.fileBufferPolicy()).enabled, false);
});

test("globs and splices", () => {
  const matches = fileBufferMatcher([
    "*.pem",
    "id_rsa*",
    "/srv/**/cache/*",
    "a?c",
  ]);
  assert.ok(matches("/x/KEY.PEM"));
  assert.ok(matches("/home/u/.ssh/id_rsa.pub"));
  assert.ok(matches("/srv/a/b/cache/file"));
  assert.ok(!matches("/srv/a/b/cache/deeper/file"));
  assert.ok(matches("/abc"));
  assert.ok(!matches("/x/pem"));
  assert.ok(!matches("/x/a.pem.txt"));
  assert.ok(fileBufferMatcher(["a+b(c)"])("/a+b(c)"));
  const cases = [
    ["", ""],
    ["", "new"],
    ["old", ""],
    ["same", "same"],
    ["abcabc", "abc"],
    ["aaa", "aaaa"],
    ["hello world", "hello brave world"],
  ];
  for (const [from, to] of cases) {
    const base = Buffer.from(from);
    let out = base;
    for (const splice of bufferSplice(base, Buffer.from(to))) {
      out = Buffer.concat([
        out.subarray(0, splice.offset),
        Buffer.from(splice.insert, "base64"),
        out.subarray(splice.offset + splice.delete),
      ]);
    }
    assert.equal(out.toString(), to, `${from} -> ${to}`);
  }
});
