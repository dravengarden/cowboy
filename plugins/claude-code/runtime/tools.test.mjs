import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { TASK_OUTPUT_PREFIX, WorkspaceTools } from "./tools.mjs";

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
  assert.deepEqual(cursors, [null, 1]);
  const unknown = await resumed.nativeCall("Read", {
    file_path: TASK_OUTPUT_PREFIX + "foreign",
  });
  assert.match(unknown.deny, /does not belong/);
  assert.equal(cursors.length, 2);
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
        cursors.push(params.afterSeq);
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
    assert.equal(finished.isError, false);
    assert.equal(stopped.isError, false);
    assert.equal(JSON.parse(finished.content[0].text).output, "done");
    assert.equal(JSON.parse(stopped.content[0].text).output, "");
    assert.deepEqual(cursors, [null, 1]);
    assert.equal(tools.operations.size, 0);
  },
);
