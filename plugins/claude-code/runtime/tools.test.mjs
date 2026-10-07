import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { TASK_OUTPUT_PREFIX, WorkspaceTools } from "./tools.mjs";
import { READ_RANGE } from "./read-range.mjs";

test("range reads keep whole-file conflict stamps without transferring the file", async (t) => {
  const { tools, files, calls, state, connection, binding } = await fixture(t);
  const source = "x".repeat(127) + "\n";
  const original = source.repeat(8192);
  files.set("/target with space/file", Buffer.from(original));
  tools.rangePython = "python3";
  const directory = await mkdtemp(join(tmpdir(), "cowboy-range-target-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const target = join(directory, "source");
  let transferred = 0;
  tools.command = async (argv) => {
    const script = argv.indexOf("-c") + 1;
    assert.deepEqual(argv.slice(1, script), ["-I", "-S", "-B", "-c"]);
    assert.equal(argv[script], READ_RANGE);
    await writeFile(target, files.get("/target with space/file"));
    const output = execFileSync(argv[0], [
      ...argv.slice(1, script + 1),
      target,
      ...argv.slice(script + 2),
    ], {
      encoding: "utf8",
    });
    transferred += Buffer.byteLength(output);
    return { exitCode: 0, closed: true, output };
  };
  for (const offset of [1, 101]) {
    const result = await tools.nativeCall("Read", {
      file_path: "file",
      offset,
      limit: 10,
    });
    assert.equal(result.result.file.numLines, 10);
    assert.equal(result.result.file.startLine, offset);
    assert.equal(result.result.file.totalLines, 8193);
  }
  assert.deepEqual(calls.map((call) => call.method), [
    "fs/getMetadata",
    "fs/getMetadata",
  ]);
  assert.ok(transferred < 4096);
  assert.equal((await readFile(state, "utf8")).includes(source), false);
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  // A change outside the selected range still revokes write authority.
  files.set("/target with space/file", Buffer.from(original + "external"));
  const refused = await resumed.nativeCall("Write", {
    file_path: "file",
    content: "lost",
  });
  assert.match(refused.deny, /changed/);
  assert.equal(
    files.get("/target with space/file").toString(),
    original + "external",
  );
  files.set("/target with space/file", Buffer.from(original));
  const edited = await resumed.nativeCall("Edit", {
    file_path: "file",
    old_string: source,
    new_string: "changed\n",
    replace_all: true,
  });
  assert.equal(edited.deny, undefined);
  assert.ok(
    files.get("/target with space/file").toString().startsWith("changed\n"),
  );
});

test("range helper handles empty files, CRLF, Unicode, missing and nonregular files", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-range-helper-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const target = join(directory, "file");
  const run = (path, offset = 1, limit = 10) =>
    JSON.parse(execFileSync(
      "python3",
      ["-c", READ_RANGE, path, String(offset), String(limit)],
      { encoding: "utf8" },
    ));
  await writeFile(target, "");
  assert.equal(run(target).numLines, 1);
  assert.equal(run(target, 2).numLines, 0);
  await writeFile(target, "中文\r\nsecond\n");
  assert.equal(
    Buffer.from(run(target, 1, 1).dataBase64, "base64").toString(),
    "中文\r",
  );
  assert.match(run(join(directory, "missing")).error, /failed/);
  assert.match(run(directory).error, /failed/);
  await writeFile(target, Buffer.from([255]));
  assert.match(run(target).error, /failed/);
  const changed = READ_RANGE.replace(
    "current = os.stat(path)",
    "os.truncate(path, 0)\n        current = os.stat(path)",
  );
  await writeFile(target, "before");
  const raced = JSON.parse(
    execFileSync("python3", ["-c", changed, target, "1", "1"], {
      encoding: "utf8",
    }),
  );
  assert.match(raced.error, /changed/);
  await writeFile(
    target,
    Buffer.concat([Buffer.from("%PDF"), Buffer.alloc(64)]),
  );
  assert.equal(run(target).fallback, true);
});

test("range results fail closed without granting edit authority", async (t) => {
  const { tools, files } = await fixture(t);
  files.set("/target with space/file", Buffer.alloc(128 * 1024));
  tools.rangePython = "python3";
  tools.command = async () => ({
    exitCode: 0,
    output: JSON.stringify({ schema: 1, sha256: "bad", size: 10 }),
  });
  assert.match(
    (await tools.nativeCall("Read", { file_path: "file" })).deny,
    /Invalid/,
  );
  assert.deepEqual(tools.state.reads, {});
});

test("small files retain two native RPCs without a utility startup", async (t) => {
  const { tools, files, calls } = await fixture(t);
  tools.rangePython = "python3";
  tools.command = () => {
    throw new Error("Short Read must not start a utility");
  };
  files.set("/target with space/file", Buffer.from("short\n"));
  assert.equal(
    (await tools.nativeCall("Read", { file_path: "file" })).deny,
    undefined,
  );
  assert.deepEqual(calls.map((call) => call.method), [
    "fs/getMetadata",
    "fs/readFile",
  ]);
});

test("an undelivered Read cannot grant edit authority after state storage fails", async (t) => {
  const { tools, files } = await fixture(t);
  files.set("/target with space/file", Buffer.from("before"));
  const save = tools.save.bind(tools);
  tools.save = async () => {
    const error = new Error("Fixture storage failure");
    error.code = "EIO";
    throw error;
  };
  assert.match(
    (await tools.nativeCall("Read", { file_path: "file" })).deny,
    /saved/,
  );
  assert.deepEqual(tools.state.reads, {});
  tools.save = save;
  assert.match(
    (await tools.nativeCall("Write", {
      file_path: "file",
      content: "lost",
    })).deny,
    /not been read/,
  );
  assert.equal(files.get("/target with space/file").toString(), "before");
});

test("bounded read stamps expire to reread rather than granting stale write access", async (t) => {
  const { tools, files } = await fixture(t);
  const digest = "a".repeat(64);
  tools.remember("/target with space/old", digest);
  for (let index = 0; index < 80; index++) {
    tools.remember("/target with space/" + index + "x".repeat(10000), digest);
  }
  assert.ok(Buffer.byteLength(JSON.stringify(tools.state.reads)) <= 512 * 1024);
  assert.equal(tools.state.reads["/target with space/old"], undefined);
  files.set("/target with space/old", Buffer.from("original"));
  assert.match(
    (await tools.nativeCall("Write", {
      file_path: "old",
      content: "lost",
    })).deny,
    /not been read/,
  );
  assert.equal(files.get("/target with space/old").toString(), "original");
});

test("failed atomic state replacement removes only its owned temporary file", async (t) => {
  const { tools, state } = await fixture(t);
  await mkdir(state);
  const unrelated = state + ".keep";
  await writeFile(unrelated, "keep");
  await assert.rejects(tools.save());
  assert.deepEqual((await readdir(join(state, ".."))).sort(), [
    "state.json",
    "state.json.keep",
  ]);
  assert.equal(await readFile(unrelated, "utf8"), "keep");
});

test("concurrent failed Reads roll back before subsequent state saves", async (t) => {
  const { tools, state, files } = await fixture(t);
  await mkdir(state);
  files.set("/target with space/a", Buffer.from("a"));
  files.set("/target with space/b", Buffer.from("b"));
  const results = await Promise.all(
    ["a", "b"].map((file_path) => tools.nativeCall("Read", { file_path })),
  );
  assert.ok(results.every((result) => result.deny));
  assert.deepEqual(tools.state.reads, {});
  await rm(state, { recursive: true });
  await tools.save();
  assert.deepEqual(JSON.parse(await readFile(state, "utf8")).reads, {});
});

test("startup cleans dead-writer temporaries and preserves live writers and unrelated files", async (t) => {
  const { tools, state } = await fixture(t);
  const child = spawn(process.execPath, ["-e", ""], { stdio: "ignore" });
  const pid = child.pid;
  await once(child, "exit");
  const suffix = "12345678-1234-1234-1234-123456789abc";
  const dead = `${state}.${pid}.${suffix}`;
  const live = `${state}.${process.pid}.${suffix}`;
  const legacy = `${state}.${suffix}`;
  await writeFile(dead, "partial", { mode: 0o600 });
  await writeFile(live, "live", { mode: 0o600 });
  await writeFile(legacy, "unowned", { mode: 0o600 });
  await tools.load();
  await assert.rejects(readFile(dead), { code: "ENOENT" });
  assert.equal(await readFile(live, "utf8"), "live");
  assert.equal(await readFile(legacy, "utf8"), "unowned");
});

test("closed private utility records expire without deleting user task handles", async (t) => {
  const { tools, connection } = await fixture(t);
  const user = "user-task";
  tools.state.jobs[user] = { afterSeq: null, exited: false };
  connection.call = async (method, params) => {
    if (method === "process/start") return { processId: params.processId };
    assert.equal(method, "process/read");
    return { chunks: [], closed: true, exited: true, exitCode: 0 };
  };
  for (let index = 0; index < 20; index++) await tools.command(["true"]);
  assert.deepEqual(Object.keys(tools.state.jobs), [user]);
});

test("quiet output retains the cancellation-compatible one-second wait bound", async (t) => {
  const { tools, connection } = await fixture(t);
  let clock = 0;
  const waits = [];
  const original = Date.now;
  tools.state.jobs.quiet = { afterSeq: null, exited: false };
  connection.call = async (method, params) => {
    assert.equal(method, "process/read");
    waits.push(params.waitMs);
    clock += params.waitMs;
    return {
      chunks: [],
      exited: clock >= 60000,
      closed: clock >= 60000,
      exitCode: clock >= 60000 ? 0 : null,
    };
  };
  try {
    Date.now = () => clock;
    assert.equal((await tools.collectOutput("quiet", 120000)).exitCode, 0);
  } finally {
    Date.now = original;
  }
  assert.deepEqual(waits, Array(60).fill(1000));
});

async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-claude-tools-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const files = new Map();
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push({ method, params });
      const path = params.path ? fileURLToPath(params.path) : "";
      if (method === "fs/createDirectory") return {};
      if (method === "fs/writeFile") {
        files.set(path, Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (!files.has(path)) {
        const error = new Error("Missing fixture file");
        error.remote = { code: -32000, message: "No such file" };
        throw error;
      }
      if (method === "fs/getMetadata") {
        return {
          isFile: true,
          size: files.get(path).length,
        };
      }
      if (method === "fs/readFile") {
        return {
          dataBase64: files.get(path).toString("base64"),
        };
      }
      throw new Error("Unexpected fixture operation");
    },
  };
  const binding = {
    workspace: { cwd: "/target with space" },
    environment: { id: "fixture" },
  };
  const state = join(directory, "state.json");
  const tools = new WorkspaceTools(connection, binding, state);
  await tools.load();
  return { tools, files, calls, connection, binding, state };
}

test("interrupt includes foreground starts whose acknowledgement is still pending", async (t) => {
  const { tools, connection } = await fixture(t);
  const admitted = Promise.withResolvers();
  const acknowledge = Promise.withResolvers();
  const terminated = Promise.withResolvers();
  const stops = [];
  connection.call = async (method, params) => {
    if (method === "process/start") {
      admitted.resolve(params.processId);
      await acknowledge.promise;
      return { processId: params.processId };
    }
    if (method === "process/terminate") {
      stops.push(params.processId);
      terminated.resolve();
      return {};
    }
    if (method === "process/read") {
      await terminated.promise;
      return { chunks: [], exited: true, closed: true, exitCode: 143 };
    }
    throw new Error(`Unexpected ${method}`);
  };
  const running = tools.call("bash", { command: "sleep 600" });
  const id = await admitted.promise;
  const interrupt = tools.cancelForeground();
  acknowledge.resolve();
  await interrupt;
  assert.deepEqual(stops, [id]);
  await running;
  assert.equal(tools.foreground.size, 0);
});

test("a changed file is refused until read again, including after cold resume", async (t) => {
  const { tools, files, connection, binding, state } = await fixture(t);
  files.set("/target with space/file", Buffer.from("before\n"));
  assert.equal(
    (await tools.call("edit", {
      file_path: "file",
      old_string: "before",
      new_string: "after",
    })).isError,
    true,
  );
  await tools.call("read", { file_path: "file" });
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  assert.equal(
    (await resumed.call("edit", {
      file_path: "file",
      old_string: "before",
      new_string: "after",
    })).isError,
    false,
  );
  files.set("/target with space/file", Buffer.from("external\n"));
  assert.equal(
    (await resumed.call("write", { file_path: "file", content: "lost" }))
      .isError,
    true,
  );
  assert.equal(files.get("/target with space/file").toString(), "external\n");
  const foreign = new WorkspaceTools(connection, {
    ...binding,
    environment: { id: "foreign" },
  }, state);
  await assert.rejects(foreign.load(), /another execution environment/);
});

test("quoted paths, CRLF, Unicode and literal replacement strings retain their bytes", async (t) => {
  const { tools, files } = await fixture(t);
  const path = "child/quoted '\" $() 中文.txt";
  assert.equal(
    (await tools.call("write", {
      file_path: path,
      content: "\uFEFFfirst\r\nsecond\r\n",
    })).isError,
    false,
  );
  assert.equal(
    (await tools.call("edit", {
      file_path: path,
      old_string: "first\nsecond\n",
      new_string: "🐎 $& $` $'\nnext\n",
    })).isError,
    false,
  );
  assert.equal(
    files.get("/target with space/" + path).toString(),
    "\uFEFF🐎 $& $` $'\r\nnext\r\n",
  );
});

test("native file results expose target paths and exact diffs, including EOF changes", async (t) => {
  const { tools, files } = await fixture(t);
  files.set("/target with space/file", Buffer.from("before"));
  const read = await tools.nativeCall("Read", { file_path: "file" });
  assert.equal(read.result.file.filePath, "/target with space/file");
  assert.equal(read.result.file.content, "before");
  const edit = await tools.nativeCall("Edit", {
    file_path: "file",
    old_string: "before",
    new_string: "after\n",
  });
  assert.deepEqual(edit.result.structuredPatch, [{
    oldStart: 1,
    oldLines: 1,
    newStart: 1,
    newLines: 1,
    lines: ["-before", "\\ No newline at end of file", "+after"],
  }]);
  files.set("/target with space/file", Buffer.from("external"));
  const refused = await tools.nativeCall("Write", {
    file_path: "file",
    content: "lost",
  });
  assert.match(refused.deny, /changed/);
  assert.equal(files.get("/target with space/file").toString(), "external");
});

test("native Read task handles retain output cursors across cold resume", async (t) => {
  const { tools, connection, binding, state } = await fixture(t);
  const id = "00000000-0000-0000-0000-000000000001";
  tools.state.jobs[id] = { afterSeq: null, exited: false };
  await tools.save();
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  const cursors = [];
  connection.call = async (method, params) => {
    assert.equal(method, "process/read");
    cursors.push(params.afterSeq);
    return {
      chunks: params.afterSeq === null
        ? [{
          seq: 1,
          stream: "stdout",
          chunk: Buffer.from("done").toString("base64"),
        }]
        : [],
      closed: true,
      exited: true,
      exitCode: 0,
    };
  };
  const path = TASK_OUTPUT_PREFIX + id;
  const first = await resumed.nativeCall("Read", { file_path: path });
  assert.equal(first.result.file.content, "done\nExit code: 0");
  const second = await resumed.nativeCall("Read", { file_path: path });
  assert.equal(second.result.file.content, "Exit code: 0");
  // A closed process is drained until a read returns nothing.
  assert.deepEqual(cursors, [null, 1, 1]);
  const unknown = await resumed.nativeCall("Read", {
    file_path: TASK_OUTPUT_PREFIX + "foreign",
  });
  assert.match(unknown.deny, /does not belong/);
  assert.equal(cursors.length, 3);
});

test("task output keeps split UTF-8 separate per stream across cold resume", async (t) => {
  const { tools, connection, binding, state } = await fixture(t);
  const id = "utf8-job";
  tools.state.jobs[id] = { afterSeq: null, exited: false };
  const stdout = Buffer.from("中");
  const stderr = Buffer.from("🐎");
  let sequence = 0;
  const chunk = (stream, bytes) => ({
    seq: ++sequence,
    stream,
    chunk: bytes.toString("base64"),
  });
  const responses = [
    {
      chunks: [
        chunk("stdout", stdout.subarray(0, 2)),
        chunk(
          "stderr",
          Buffer.concat([Buffer.alloc(65534, 120), stderr.subarray(0, 2)]),
        ),
      ],
      closed: false,
      exited: false,
    },
    {
      chunks: [
        chunk("stdout", stdout.subarray(2)),
        chunk("stderr", stderr.subarray(2)),
      ],
      closed: true,
      exited: true,
      exitCode: 0,
    },
    { chunks: [], closed: true, exited: true, exitCode: 0 },
    {
      chunks: [],
      closed: true,
      exited: true,
      exitCode: 0,
    },
  ];
  connection.call = async (method) => {
    assert.equal(method, "process/read");
    return responses.shift();
  };
  const first = await tools.collect(id, 10000);
  assert.equal(first.output, "x".repeat(65534));
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  const last = await resumed.collect(id, 10000);
  assert.equal(last.output, "中🐎");
  assert.equal((await resumed.collect(id, 10000)).output, "");
});

test("interleaved output and terminal incomplete UTF-8 do not corrupt other streams", async (t) => {
  const { tools, connection } = await fixture(t);
  tools.state.jobs.job = { afterSeq: null, exited: false };
  connection.call = async (_method, params) => ({
    chunks: params.afterSeq !== null ? [] : [
      {
        seq: 1,
        stream: "stdout",
        chunk: Buffer.from([0xe4, 0xb8]).toString("base64"),
      },
      {
        seq: 2,
        stream: "stderr",
        chunk: Buffer.from("error").toString("base64"),
      },
      {
        seq: 3,
        stream: "stdout",
        chunk: Buffer.from([0xad, 0xf0, 0x9f]).toString("base64"),
      },
    ],
    exited: true,
    closed: true,
    exitCode: 1,
  });
  assert.equal((await tools.collect("job", 1000)).output, "error中�");
});

test("failed output state commit preserves the cursor for retry", async (t) => {
  const { tools, connection, state } = await fixture(t);
  tools.state.jobs.job = { afterSeq: null, exited: false };
  const cursors = [];
  connection.call = async (_method, params) => {
    cursors.push(params.afterSeq);
    return {
      chunks: params.afterSeq === null
        ? [{
          seq: 1,
          stream: "stdout",
          chunk: Buffer.from("retained").toString("base64"),
        }]
        : [],
      exited: true,
      closed: true,
      exitCode: 0,
    };
  };
  // Actual atomic rename failure, after save's in-memory update has run.
  await mkdir(state);
  await assert.rejects(tools.collect("job", 1000));
  assert.deepEqual(tools.state.jobs.job, { afterSeq: null, exited: false });
  await rm(state, { recursive: true });
  assert.equal((await tools.collect("job", 1000)).output, "retained");
  assert.deepEqual(cursors, [null, 1, null, 1]);
});

test("byte-split output preserves BOMs and native invalid UTF-8 replacement", async (t) => {
  const { tools, connection } = await fixture(t);
  for (
    const bytes of [
      Buffer.from("\ufeff中文🐎\0end"),
      Buffer.from([0xe0, 0x80, 0x80, 0xff, 0xed, 0xa0, 0x80]),
      Buffer.from([0xf0, 0x90, 0x80]),
    ]
  ) {
    tools.state.jobs.job = { afterSeq: null, exited: false };
    connection.call = async (_method, params) => ({
      chunks: params.afterSeq !== null
        ? []
        : Array.from(bytes, (byte, index) => ({
          seq: index + 1,
          stream: "stdout",
          chunk: Buffer.from([byte]).toString("base64"),
        })),
      exited: true,
      closed: true,
      exitCode: 0,
    });
    assert.equal(
      (await tools.collect("job", 1000)).output,
      bytes.toString("utf8"),
    );
  }
});

test("ambiguous edits and foreign task ids have no effects", async (t) => {
  const { tools, files, calls } = await fixture(t);
  files.set("/target with space/file", Buffer.from("one one"));
  await tools.call("read", { file_path: "file" });
  assert.equal(
    (await tools.call("edit", {
      file_path: "file",
      old_string: "one",
      new_string: "two",
    })).isError,
    true,
  );
  assert.equal(files.get("/target with space/file").toString(), "one one");
  const before = calls.length;
  assert.equal(
    (await tools.call("taskstop", { task_id: "foreign" })).isError,
    true,
  );
  assert.equal(calls.length, before);
  assert.equal(
    (await tools.call("edit", {
      file_path: "file",
      old_string: "one",
      new_string: "two",
      replace_all: true,
    })).isError,
    false,
  );
  assert.equal(files.get("/target with space/file").toString(), "two two");
});

test("tilde paths use executor home and share read stamps with absolute target paths", async (t) => {
  const { tools, files, connection, calls } = await fixture(t);
  connection.info = { userHomeDir: "file:///target%20home" };
  files.set("/target home/file", Buffer.from("before"));
  assert.equal(tools.path("~"), "/target home");
  assert.equal(tools.path("./~literal"), "/target with space/~literal");
  const read = await tools.nativeCall("Read", { file_path: "~/file" });
  assert.equal(read.result.file.filePath, "/target home/file");
  const edit = await tools.nativeCall("Edit", {
    file_path: "/target home/file",
    old_string: "before",
    new_string: "after",
  });
  assert.equal(edit.deny, undefined);
  assert.equal(files.get("/target home/file").toString(), "after");
  assert.ok(
    calls.every((call) =>
      fileURLToPath(call.params.path).startsWith("/target home")
    ),
  );
  connection.info = {};
  assert.throws(() => tools.path("~/file"), /Target home is unavailable/);
  assert.throws(
    () => tools.path("~another/file"),
    /relative or absolute target path/,
  );
});

test("Write can replace a previously read image without a post-write decode failure", async (t) => {
  const { tools, files, calls } = await fixture(t);
  files.set(
    "/target with space/pixel",
    Buffer.from("89504e470d0a1a0a0001020304", "hex"),
  );
  assert.equal(
    (await tools.nativeCall("Read", { file_path: "pixel" })).result.type,
    "image",
  );
  const result = await tools.nativeCall("Write", {
    file_path: "pixel",
    content: "replacement\n",
  });
  assert.equal(result.deny, undefined);
  assert.equal(result.result.originalFile, null);
  assert.equal(result.result.type, "update");
  assert.equal(
    files.get("/target with space/pixel").toString(),
    "replacement\n",
  );
  assert.equal(
    calls.filter((call) => call.method === "fs/writeFile").length,
    1,
  );
});

test("failed post-write state commit requires a fresh Read before another edit", async (t) => {
  const { tools, files, state } = await fixture(t);
  files.set("/target with space/file", Buffer.from("before"));
  await tools.nativeCall("Read", { file_path: "file" });
  const stamp = tools.state.reads["/target with space/file"];
  await rm(state);
  await mkdir(state);
  const result = await tools.nativeCall("Write", {
    file_path: "file",
    content: "after",
  });
  assert.match(result.deny, /could not be saved/);
  assert.equal(files.get("/target with space/file").toString(), "after");
  assert.equal(tools.state.reads["/target with space/file"], stamp);
  await rm(state, { recursive: true });
  await tools.save();
  const edit = await tools.nativeCall("Edit", {
    file_path: "file",
    old_string: "after",
    new_string: "lost",
  });
  assert.match(edit.deny, /Read it before editing/);
  assert.equal(files.get("/target with space/file").toString(), "after");
  await tools.nativeCall("Read", { file_path: "file" });
  assert.equal(
    (await tools.nativeCall("Edit", {
      file_path: "file",
      old_string: "after",
      new_string: "confirmed",
    })).deny,
    undefined,
  );
});

test("read images as binary content and preserve notebook metadata", async (t) => {
  const { tools, files } = await fixture(t);
  const image = Buffer.from("89504e470d0a1a0a0001020304", "hex");
  files.set("/target with space/pixel", image);
  const read = await tools.call("read", { file_path: "pixel" });
  assert.equal(read.content[0].type, "image");
  assert.deepEqual(Buffer.from(read.content[0].data, "base64"), image);
  files.set(
    "/target with space/book.ipynb",
    Buffer.from(
      JSON.stringify({
        nbformat: 4,
        metadata: { preserve: true },
        cells: [
          {
            id: "cell",
            cell_type: "code",
            metadata: {},
            source: ["before"],
            outputs: [],
          },
        ],
      }),
    ),
  );
  await tools.call("read", { file_path: "book.ipynb" });
  assert.equal(
    (await tools.call("notebookedit", {
      notebook_path: "book.ipynb",
      cell_id: "cell",
      new_source: "print('中文')\n",
    })).isError,
    false,
  );
  const notebook = JSON.parse(files.get("/target with space/book.ipynb"));
  assert.deepEqual(notebook.metadata, { preserve: true });
  assert.deepEqual(notebook.cells[0].source, ["print('中文')\n"]);
});

test("concurrent state saves retain the latest read stamps", async (t) => {
  const { tools, state } = await fixture(t);
  tools.state.reads.first = "first";
  const first = tools.save();
  tools.state.reads.second = "second";
  await Promise.all([first, tools.save()]);
  assert.deepEqual(JSON.parse(await readFile(state)).reads, {
    first: "first",
    second: "second",
  });
});

test("a failed read does not authorize overwriting an unread file", async (t) => {
  const { tools, files } = await fixture(t);
  files.set("/target with space/binary", Buffer.from([0xff, 0xfe, 0]));
  assert.match(
    (await tools.nativeCall("Read", { file_path: "binary" })).deny,
    /not valid UTF-8/,
  );
  assert.equal(
    (await tools.call("read", { file_path: "binary" })).isError,
    true,
  );
  assert.equal(
    (await tools.call("write", { file_path: "binary", content: "lost" }))
      .isError,
    true,
  );
  assert.deepEqual(
    files.get("/target with space/binary"),
    Buffer.from([0xff, 0xfe, 0]),
  );
});

function deferred() {
  let resolve;
  const promise = new Promise((done) => resolve = done);
  return { promise, resolve };
}

test("aliased file mutations cannot both validate the same stale bytes", {
  timeout: 3000,
}, async (t) => {
  const { tools, files, connection } = await fixture(t);
  files.set("/target with space/source", Buffer.from("before"));
  files.set("/target with space/independent", Buffer.from("available"));
  const original = connection.call.bind(connection);
  const entered = deferred();
  const release = deferred();
  let writes = 0;
  connection.call = async (method, params) => {
    const path = params.path ? fileURLToPath(params.path) : "";
    if (path.endsWith("/alias")) {
      params = { ...params, path: "file:///target%20with%20space/source" };
    }
    if (method === "fs/writeFile") {
      writes++;
      if (writes === 1) {
        entered.resolve();
        await release.promise;
      }
    }
    return original(method, params);
  };
  await tools.nativeCall("Read", { file_path: "source" });
  await tools.nativeCall("Read", { file_path: "alias" });
  const first = tools.nativeCall("Edit", {
    file_path: "source",
    old_string: "before",
    new_string: "first",
  });
  await entered.promise;
  const second = tools.nativeCall("Edit", {
    file_path: "alias",
    old_string: "before",
    new_string: "second",
  });
  // A separate real state save gives the queued mutation time to progress,
  // while also proving reads are not blocked by a pending target write.
  const independent = await tools.nativeCall("Read", {
    file_path: "independent",
  });
  assert.equal(independent.deny, undefined);
  release.resolve();
  const results = await Promise.all([first, second]);
  assert.equal(results[0].deny, undefined);
  assert.match(results[1].deny ?? "", /changed/);
  assert.equal(writes, 1);
  assert.equal(files.get("/target with space/source").toString(), "first");
});

test("independent reads proceed while same-file edits wait for their read", {
  timeout: 2000,
}, async (t) => {
  const { tools, files, connection } = await fixture(t);
  files.set("/target with space/a", Buffer.from("before"));
  files.set("/target with space/b", Buffer.from("independent"));
  const entered = deferred();
  const release = deferred();
  const original = connection.call.bind(connection);
  connection.call = async (method, params) => {
    if (method === "fs/readFile" && fileURLToPath(params.path).endsWith("/a")) {
      entered.resolve();
      await release.promise;
    }
    return original(method, params);
  };
  const read = tools.call("read", { file_path: "a" });
  await entered.promise;
  const edit = tools.call("edit", {
    file_path: "./a",
    old_string: "before",
    new_string: "after",
  });
  const other = await tools.call("read", { file_path: "b" });
  assert.equal(other.isError, false);
  assert.equal(files.get("/target with space/a").toString(), "before");
  release.resolve();
  assert.equal((await read).isError, false);
  assert.equal((await edit).isError, false);
  assert.equal(files.get("/target with space/a").toString(), "after");
});

test(
  "TaskStop terminates a collecting Bash without blocking independent files",
  {
    timeout: 2000,
  },
  async (t) => {
    const { tools, files, connection } = await fixture(t);
    tools.shell = "/bin/bash";
    files.set("/target with space/file", Buffer.from("available"));
    const collecting = deferred();
    const terminated = deferred();
    const original = connection.call.bind(connection);
    let task;
    const cursors = [];
    connection.call = async (method, params) => {
      if (method === "process/start") {
        task = params.processId;
        return { processId: task };
      }
      if (method === "process/terminate") {
        assert.equal(params.processId, task);
        terminated.resolve();
        return {};
      }
      if (method === "process/read") {
        cursors.push({
          afterSeq: params.afterSeq,
          observation: params.maxBytes === 1,
        });
        collecting.resolve();
        await terminated.promise;
        return {
          chunks: params.afterSeq === null
            ? [{
              seq: 1,
              stream: "stdout",
              chunk: Buffer.from("done").toString("base64"),
            }]
            : [],
          exited: true,
          closed: true,
          exitCode: 143,
        };
      }
      return original(method, params);
    };
    const bash = tools.call("bash", { command: "sleep 120" });
    await collecting.promise;
    assert.equal(
      (await tools.call("read", { file_path: "file" })).isError,
      false,
    );
    const stop = tools.call("taskstop", { task_id: task });
    const [finished, stopped] = await Promise.all([bash, stop]);
    // As natively, the killed foreground command is a non-zero exit error.
    assert.equal(finished.isError, true);
    assert.equal(stopped.isError, false);
    assert.equal(finished.content[0].text, "Exit code 143\ndone");
    assert.equal(JSON.parse(stopped.content[0].text).output, "");
    assert.deepEqual(
      cursors.filter((read) => !read.observation).map((read) => read.afterSeq),
      // The Bash drains its closed process; TaskStop then reads the end.
      [null, 1, 1],
    );
    assert.equal(cursors.filter((read) => read.observation).length, 1);
    assert.equal(tools.operations.size, 0);
  },
);

test("a closed process is drained across reads, not cut at the first", async (t) => {
  const { tools, connection } = await fixture(t);
  tools.state.jobs.job = { afterSeq: null, exited: false };
  // The executor reports closed while output beyond one read remains.
  const pages = ["first ", "second ", "third"];
  connection.call = async (_method, params) => {
    const next = params.afterSeq ?? 0;
    return {
      chunks: next < pages.length
        ? [{
          seq: next + 1,
          stream: "stdout",
          chunk: Buffer.from(pages[next]).toString("base64"),
        }]
        : [],
      exited: true,
      closed: true,
      exitCode: 0,
    };
  };
  assert.equal((await tools.collect("job", 1)).output, "first second third");
});

test("a stop during an output read survives the read's cursor update", async (t) => {
  const { tools, connection } = await fixture(t);
  tools.state.jobs.job = { afterSeq: null, exited: false };
  const reading = deferred();
  const release = deferred();
  connection.call = async (method, params) => {
    if (method === "process/terminate") return {};
    if (method === "process/read" && params.maxBytes === 1) {
      return { chunks: [], exited: false, closed: false };
    }
    reading.resolve();
    await release.promise;
    return {
      chunks: params.afterSeq === null
        ? [{
          seq: 1,
          stream: "stdout",
          chunk: Buffer.from("x").toString("base64"),
        }]
        : [],
      exited: false,
      closed: false,
    };
  };
  const collecting = tools.collect("job", 1);
  await reading.promise;
  await tools.cancelTasks(["job"]);
  release.resolve();
  await collecting;
  assert.equal(tools.state.jobs.job.stopped, true);
  assert.deepEqual(await tools.waitTask("job", null, 1), { stopped: true });
});
