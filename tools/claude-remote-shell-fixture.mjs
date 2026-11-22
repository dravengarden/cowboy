// The local executor stand-in and Bash fixture shared by the remote shell
// tests, which are split across files so their real-time waits overlap.
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { WorkspaceTools } from "../plugins/claude-code/runtime/tools.mjs";

// A local stand-in for the executor: real processes and files, so the
// native-shaped shell wrapper is exercised by an actual shell.
function localConnection(home) {
  const processes = new Map();
  return {
    info: { userHomeDir: pathToFileURL(home).href },
    async call(method, params) {
      const path = params.path ? fileURLToPath(params.path) : undefined;
      if (method === "process/start") {
        const child = spawn(params.argv[0], params.argv.slice(1), {
          cwd: fileURLToPath(params.cwd),
          env: { ...process.env, ...params.envPolicy.set },
          stdio: [params.pipeStdin ? "pipe" : "ignore", "pipe", "pipe"],
          // As the executor's, each process leads a group of its own.
          detached: true,
        });
        const job = {
          child,
          chunks: [],
          seq: 0,
          exitCode: null,
          closed: false,
        };
        for (const stream of ["stdout", "stderr"]) {
          child[stream].on("data", (data) =>
            job.chunks.push({
              seq: ++job.seq,
              stream,
              chunk: data.toString("base64"),
            }),
          );
        }
        job.done = new Promise((resolve) =>
          child.on("close", (code, signal) => {
            job.exitCode = code ?? (signal ? 143 : 1);
            job.closed = true;
            resolve();
          }),
        );
        processes.set(params.processId, job);
        return { processId: params.processId };
      }
      if (method === "process/write") {
        const job = processes.get(params.processId);
        if (!job) return { status: "unknownProcess" };
        job.child.stdin.write(Buffer.from(params.chunk, "base64"));
        return { status: "accepted" };
      }
      if (method === "process/terminate") {
        // The executor ends the process's group.
        const pid = processes.get(params.processId)?.child.pid;
        try {
          if (pid) process.kill(-pid, "SIGTERM");
        } catch {
          // Already ended.
        }
        return {};
      }
      if (method === "process/read") {
        const job = processes.get(params.processId);
        await Promise.race([
          job.done,
          new Promise((resolve) => setTimeout(resolve, params.waitMs)),
        ]);
        // As the executor's, a read is bounded and a closed process can still
        // hold unread output.
        let size = 0;
        const chunks = job.chunks
          .filter((chunk) => params.afterSeq === null || chunk.seq > params.afterSeq)
          .filter(
            (chunk, index) =>
              (size += Buffer.from(chunk.chunk, "base64").length) <= params.maxBytes || index === 0,
          );
        return {
          chunks,
          exited: job.closed,
          closed: job.closed,
          exitCode: job.exitCode,
        };
      }
      if (method === "fs/getMetadata") {
        // The executor reports an absent file as a remote error.
        const info = await stat(path).catch((error) => {
          if (error.code !== "ENOENT") throw error;
          throw Object.assign(new Error("absent"), {
            remote: { message: "No such file or directory" },
          });
        });
        return { isFile: info.isFile(), size: info.size };
      }
      if (method === "fs/readFile") {
        // The executor reports a refused read as a remote error too.
        const bytes = await readFile(path).catch((error) => {
          throw Object.assign(new Error(error.message), {
            remote: { message: error.message },
          });
        });
        return { dataBase64: bytes.toString("base64") };
      }
      if (method === "fs/writeFile") {
        await writeFile(path, Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (method === "fs/remove") {
        await rm(path, { force: true, recursive: params.recursive === true });
        return {};
      }
      if (method === "fs/readDirectory") {
        const entries = await readdir(path, { withFileTypes: true });
        return {
          entries: entries.map((entry) => ({
            fileName: entry.name,
            isDirectory: entry.isDirectory(),
            isFile: entry.isFile(),
          })),
        };
      }
      throw new Error(`Unexpected ${method}`);
    },
  };
}

export async function shellFixture(t) {
  const root = await mkdtemp(join(tmpdir(), "cowboy-shell-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const project = join(root, "project");
  const home = join(root, "home");
  await mkdir(join(project, "sub"), { recursive: true });
  await mkdir(home);
  const binding = {
    workspace: { cwd: project },
    environment: { id: "fixture" },
  };
  const tools = new WorkspaceTools(localConnection(home), binding, join(root, "state.json"));
  await tools.load();
  tools.shell = "/bin/sh";
  const bash = async (args) => {
    const result = await tools.dispatch("Bash", args);
    return result.deny !== undefined ? { error: result.deny } : result.result.stdout;
  };
  return { tools, bash, project, home };
}
