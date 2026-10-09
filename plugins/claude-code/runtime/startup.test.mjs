import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  parseStartupSurvey,
  STARTUP_SURVEY,
  WorkspaceTools,
} from "./tools.mjs";

// The executor's file and process methods over this machine. `refuse`
// rejects a process start whose argv it matches, as a target without the
// survey's utilities would fail it.
function localConnection(home, { refuse, path } = {}) {
  const processes = new Map();
  const calls = [];
  const remote = (error) =>
    Object.assign(new Error(error.message), {
      remote: {
        code: -32000,
        message: error.code === "ENOENT"
          ? "No such file or directory (os error 2)"
          : error.message,
      },
    });
  return {
    calls,
    info: { userHomeDir: pathToFileURL(home).href },
    async call(method, params) {
      calls.push(method);
      const path = params.path ? fileURLToPath(params.path) : undefined;
      if (method === "fs/getMetadata") {
        try {
          const link = await lstat(path);
          const target = link.isSymbolicLink() ? await stat(path) : link;
          return {
            isFile: target.isFile(),
            isDirectory: target.isDirectory(),
            isSymlink: link.isSymbolicLink(),
            size: target.size,
          };
        } catch (error) {
          throw remote(error);
        }
      }
      if (method === "fs/readFile") {
        try {
          return { dataBase64: (await readFile(path)).toString("base64") };
        } catch (error) {
          throw remote(error);
        }
      }
      if (method === "process/start") {
        if (refuse?.(params.argv)) throw new Error("Target refused start");
        const child = spawn(params.argv[0], params.argv.slice(1), {
          cwd: fileURLToPath(params.cwd),
          env: path ? { ...process.env, PATH: path } : process.env,
          stdio: ["ignore", "pipe", "pipe"],
        });
        const job = { chunks: [], seq: 0, exited: false, wake: () => {} };
        for (const stream of ["stdout", "stderr"]) {
          child[stream].on("data", (data) => {
            job.chunks.push({
              seq: ++job.seq,
              stream,
              chunk: data.toString("base64"),
            });
            job.wake();
          });
        }
        child.on("close", (code) => {
          Object.assign(job, { exited: true, code });
          job.wake();
        });
        processes.set(params.processId, job);
        return { processId: params.processId };
      }
      if (method === "process/read") {
        const job = processes.get(params.processId);
        const after = params.afterSeq ?? 0;
        if (!job.exited && !job.chunks.some((chunk) => chunk.seq > after)) {
          await new Promise((resolve) => {
            job.wake = resolve;
            setTimeout(resolve, params.waitMs);
          });
        }
        const chunks = job.chunks.filter((chunk) => chunk.seq > after);
        return {
          chunks,
          exited: job.exited,
          closed: job.exited,
          exitCode: job.exited ? job.code : null,
        };
      }
      if (method === "process/terminate") return {};
      throw new Error(`Unexpected ${method}`);
    },
  };
}

async function tree(t) {
  const root = await mkdtemp(join(tmpdir(), "cowboy-claude-startup-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = join(root, "home");
  const repository = join(root, "w", "repo");
  const cwd = join(repository, "sub");
  const files = {
    [join(home, "notes.md")]: "USER_NOTES\n",
    [join(home, ".claude", "skills", "alpha", "SKILL.md")]:
      "---\nname: alpha\ndescription: Alpha\n---\nALPHA\n",
    [join(home, ".claude", "commands", "deploy.md")]: "DEPLOY\n",
    [join(home, ".claude.json")]: JSON.stringify({
      mcpServers: { tool: { command: "echo", args: ["${COWBOY_TEST_UNSET}"] } },
    }),
    [join(root, "w", "CLAUDE.md")]: "OUTSIDE_REPOSITORY\n",
    [join(repository, "CLAUDE.md")]: "ROOT\n@docs/imported.md\n",
    [join(repository, "docs", "imported.md")]: "IMPORTED\n",
    [join(repository, ".claude", "rules", "always.md")]: "RULE\n",
    [join(repository, ".claude", "skills", "beta", "SKILL.md")]:
      "---\nname: beta\ndescription: Beta\n---\nBETA\n",
    [join(repository, ".mcp.json")]: JSON.stringify({
      mcpServers: { project: { command: "true" } },
    }),
    [join(cwd, ".claude", "settings.json")]: JSON.stringify({
      hooks: { Stop: [{ hooks: [{ type: "command", command: "a" }] }] },
    }),
    [join(cwd, "CLAUDE.local.md")]: "LOCAL\n",
  };
  for (const [path, content] of Object.entries(files)) {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content);
  }
  // A linked candidate is read through its link; a broken one is absent.
  await symlink(join(home, "notes.md"), join(home, ".claude", "CLAUDE.md"));
  await symlink(join(root, "missing"), join(cwd, "CLAUDE.md"));
  const git = (...args) =>
    execFileSync("git", ["-C", repository, ...args], { stdio: "ignore" });
  git("init", "-q", "-b", "main");
  git("config", "user.name", "Startup Test");
  git("config", "user.email", "startup@example.invalid");
  git("add", "CLAUDE.md");
  git("commit", "-q", "-m", "initial");
  return { root, home, cwd };
}

async function discover(t, { home, cwd }, options) {
  const state = await mkdtemp(join(tmpdir(), "cowboy-claude-startup-state-"));
  t.after(() => rm(state, { recursive: true, force: true }));
  const connection = localConnection(home, options);
  const tools = new WorkspaceTools(
    connection,
    { workspace: { cwd }, environment: { id: "startup" } },
    join(state, "state.json"),
  );
  await tools.load();
  // As launch.mjs: the walks start beside context() and share its survey.
  const contextStarted = tools.context();
  const walks = Promise.all([
    tools.projectHooks(),
    tools.skillFiles(),
    tools.mcpInputs(),
  ]);
  const context = await contextStarted;
  const [hooks, skills, mcp] = await walks;
  tools.endStartup();
  await tools.shellSnapshot;
  await tools.saves;
  const { nonce: _nonce, ...stable } = context;
  return {
    result: { context: stable, hooks, skills, mcp },
    calls: connection.calls,
  };
}

test("the startup survey answers every walk as the per-walk queries do", async (t) => {
  const paths = await tree(t);
  const surveyed = await discover(t, paths);
  const queried = await discover(t, paths, {
    refuse: (argv) => argv.some((arg) => arg.includes("survey()")),
  });
  assert.deepEqual(surveyed.result, queried.result);
  assert.deepEqual(
    surveyed.result.context.instructionFiles.map((file) => file.content),
    [
      "USER_NOTES\n",
      "OUTSIDE_REPOSITORY\n",
      "ROOT\n@docs/imported.md\n",
      "IMPORTED\n",
      "RULE\n",
      "LOCAL\n",
    ],
  );
  assert.match(surveyed.result.context.git, /Current branch: main/);
  assert.match(surveyed.result.context.git, /Git user: Startup Test/);
  assert.deepEqual(
    surveyed.result.skills.map((skill) => skill.name).sort(),
    ["alpha", "beta", "deploy"],
  );
  assert.equal(surveyed.result.mcp.repositoryRoot, dirname(paths.cwd));
  const count = (calls, method) =>
    calls.filter((call) => call === method).length;
  // Only the broken link and the import are asked about individually.
  assert.ok(count(surveyed.calls, "fs/getMetadata") <= 2);
  assert.ok(
    surveyed.calls.length * 3 < queried.calls.length,
    `${surveyed.calls.length} calls, per-walk ${queried.calls.length}`,
  );
});

test("long Git answers render as the per-walk queries render them", async (t) => {
  const paths = await tree(t);
  const repository = dirname(paths.cwd);
  // Five 1600-byte subjects exceed any short cap; 600 modified names make
  // a status far beyond the 2000 characters the block shows.
  for (let index = 0; index < 5; index++) {
    execFileSync("git", [
      "-C",
      repository,
      "commit",
      "-q",
      "--allow-empty",
      "-m",
      `${index} ${"subject 中文 ".repeat(100)}`,
    ]);
  }
  // Tracked, then modified: git lists each one.
  await mkdir(join(repository, "tracked"));
  for (let index = 0; index < 600; index++) {
    await writeFile(join(repository, "tracked", `file-${index}-名前.txt`), "a");
  }
  execFileSync("git", ["-C", repository, "add", "tracked"]);
  execFileSync("git", ["-C", repository, "commit", "-q", "-m", "tracked"]);
  for (let index = 0; index < 600; index++) {
    await writeFile(join(repository, "tracked", `file-${index}-名前.txt`), "b");
  }
  const surveyed = await discover(t, paths);
  const queried = await discover(t, paths, {
    refuse: (argv) => argv.some((arg) => arg.includes("survey()")),
  });
  assert.deepEqual(surveyed.result, queried.result);
  assert.match(surveyed.result.context.git, /truncated because it exceeds 2k/);
  // Every listed subject is complete (the newest commit is "tracked").
  for (let index = 1; index < 5; index++) {
    assert.ok(
      surveyed.result.context.git.includes(
        `${index} ${"subject 中文 ".repeat(100).trim()}`,
      ),
    );
  }
});

test("a target missing a survey utility answers through the per-walk queries", async (t) => {
  const paths = await tree(t);
  // Every utility either path uses, except the survey's base64.
  const bin = join(paths.root, "bin");
  await mkdir(bin);
  for (
    const tool of [
      "bash",
      "sh",
      "git",
      "find",
      "uname",
      "head",
      "tr",
      "sed",
      "wc",
      "printenv",
    ]
  ) {
    const resolved = execFileSync("bash", [
      "-c",
      'command -v "$1"',
      "bash",
      tool,
    ], {
      encoding: "utf8",
    }).trim();
    await symlink(resolved, join(bin, tool));
  }
  const status = (() => {
    try {
      execFileSync(join(bin, "bash"), [
        "-c",
        STARTUP_SURVEY,
        "bash",
        "--",
        "--",
      ], {
        env: { PATH: bin },
        stdio: "ignore",
      });
      return 0;
    } catch (error) {
      return error.status;
    }
  })();
  assert.equal(status, 3);
  const surveyed = await discover(t, paths, { path: bin });
  const queried = await discover(t, paths, {
    path: bin,
    refuse: (argv) => argv.some((arg) => arg.includes("survey()")),
  });
  assert.deepEqual(surveyed.result, queried.result);
  assert.match(surveyed.result.context.git, /Current branch: main/);
});

test("a malformed survey falls back to the per-walk queries", () => {
  const candidates = ["/a", "/b"];
  const answer = (name, code, text) => [
    `\x1egit ${name} ${code}`,
    Buffer.from(text).toString("base64"),
  ];
  const head = [
    "\x1eplatform",
    "/bin/bash",
    "Linux 6",
    "/bin/bash",
    "",
    ...answer("inside", 0, "true\n"),
    ...answer("branch", 0, "main\n"),
    ...answer("origin", 128, "fatal: not a symbolic ref\n"),
    ...answer("master", 1, ""),
    ...answer("main", 0, "0123\n"),
    ...answer("user", 0, "Name\n"),
    ...answer("status", 0, " M file\n"),
    ...answer("log", 0, "0123 subject\n"),
  ];
  const toplevel = answer("toplevel", 0, "/r\n");
  const tail = [
    "\x1erules",
    "12\t/r/.claude/rules/x.md",
    "\x1eskills",
    "\x1f/h/.claude/skills",
    "?\t/h/.claude/skills/s/SKILL.md",
    "\x1ecandidates",
    "F7",
    "A",
  ];
  const survey = (...parts) => [...parts.flat(), "\x1eend", ""].join("\n");
  const parsed = parseStartupSurvey(survey(head, toplevel, tail), candidates);
  assert.equal(parsed.git.status.output, " M file\n");
  assert.deepEqual(parsed.facts.get("/a"), { isFile: true, size: 7 });
  assert.equal(parsed.facts.get("/b"), null);
  assert.deepEqual(parsed.facts.get("/r/.claude/rules/x.md"), {
    isFile: true,
    size: 12,
  });
  assert.equal(parsed.facts.has("/h/.claude/skills/s/SKILL.md"), false);
  assert.deepEqual(parsed.skills.get("/h/.claude/skills"), [
    "/h/.claude/skills/s/SKILL.md",
  ]);
  const refused = (output, expected = candidates) =>
    assert.equal(parseStartupSurvey(output, expected), undefined);
  // Output before the survey.
  refused("base64: not found\n" + survey(head, toplevel, tail));
  // An empty successful answer, or a missing one.
  refused(survey(head, answer("toplevel", 0, ""), tail));
  refused(survey(head.slice(0, -2), toplevel, tail));
  // A listed name that imitates markers: a newline and a Git section.
  refused(
    survey(head, toplevel, [
      ...tail.slice(0, 4),
      "?\t/h/.claude/commands/a",
      ...answer("status", 0, ""),
      "\x1eignored.md",
      ...tail.slice(4),
    ]),
  );
  refused(
    survey(head, toplevel, [
      ...tail.slice(0, 4),
      "not a listing",
      ...tail.slice(4),
    ]),
  );
  // Truncated output, a different candidate count or an unknown answer.
  refused([...head, ...toplevel, ...tail].join("\n"));
  refused(survey(head, toplevel, tail), ["/a"]);
  refused(survey(head, toplevel, [...tail.slice(0, -1), "X"]));
});
