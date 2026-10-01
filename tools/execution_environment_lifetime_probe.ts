/** Native executor lifetime acceptance in a loopback-only Linux namespace.
 *
 * Fixed commands run in a disposable directory, with closed environment and no
 * model credentials. This tests upstream behavior, not a Cowboy execution grant.
 */

import { parseArgs } from "node:util";

function require(condition: unknown, detail: string): asserts condition {
  if (!condition) throw new Error(detail);
}

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
// Native JSON-RPC is inspected at each use; this fixture does not define its SDK.
// deno-lint-ignore no-explicit-any
type RecordValue = Record<string, any>;
const FRAME_LIMIT = 4 * 1024 * 1024;

async function stop(process: Deno.ChildProcess) {
  try {
    process.kill("SIGTERM");
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
  await process.status;
}

class Connection {
  private next = 0;
  private pending = new Map<number, {
    resolve: (value: RecordValue) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }>();

  private constructor(readonly socket: WebSocket) {
    socket.onmessage = (event) => {
      if (typeof event.data !== "string" || event.data.length > FRAME_LIMIT) {
        this.fail("invalid executor frame");
        socket.close();
        return;
      }
      let message: RecordValue;
      try {
        message = JSON.parse(event.data);
      } catch {
        this.fail("invalid executor JSON");
        socket.close();
        return;
      }
      // Process output is retained and queried explicitly in this fixture.
      if (!("id" in message)) return;
      const request = this.pending.get(message.id);
      if (!request) {
        this.fail("unknown executor response identity");
        socket.close();
        return;
      }
      this.pending.delete(message.id);
      clearTimeout(request.timeout);
      request.resolve(message);
    };
    socket.onclose = () => this.fail("executor connection ended");
    socket.onerror = () => this.fail("executor connection failed");
  }

  private fail(detail: string) {
    for (const request of this.pending.values()) {
      clearTimeout(request.timeout);
      request.reject(new Error(detail));
    }
    this.pending.clear();
  }

  static async open(url: string): Promise<Connection> {
    const socket = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        socket.close();
        reject(new Error("executor connect timed out"));
      }, 3000);
      socket.onopen = () => {
        clearTimeout(timeout);
        resolve();
      };
      socket.onerror = () => {
        clearTimeout(timeout);
        reject(new Error("executor connect failed"));
      };
    });
    return new Connection(socket);
  }

  async request(
    method: string,
    params: RecordValue,
    error = false,
  ): Promise<RecordValue> {
    require(this.pending.size < 16, "fixture pending budget exceeded");
    const id = ++this.next;
    const message = await new Promise<RecordValue>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error("executor response timed out"));
      }, 5000);
      this.pending.set(id, { resolve, reject, timeout });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
    if (error) {
      require(message.error && !message.result, "expected executor rejection");
      return message.error;
    }
    require(message.result && !message.error, `${method} failed`);
    return message.result;
  }

  async initialize(resumeSessionId?: string) {
    const result = await this.request("initialize", {
      clientName: "cowboy-executor-lifetime-fixture",
      ...(resumeSessionId ? { resumeSessionId } : {}),
    });
    this.socket.send(JSON.stringify({ method: "initialized", params: {} }));
    return result;
  }

  async close() {
    if (this.socket.readyState === WebSocket.CLOSED) return;
    const ended = new Promise<void>((resolve) => {
      this.socket.addEventListener("close", () => resolve(), { once: true });
    });
    this.socket.close();
    await Promise.race([ended, delay(1000)]);
  }
}

async function main() {
  const { values } = parseArgs({
    args: Deno.args,
    options: {
      "native-cli": { type: "string" },
      version: { type: "string" },
      sha256: { type: "string" },
      receipt: { type: "string" },
    },
    strict: true,
  });
  const binary = values["native-cli"];
  const receipt = values.receipt;
  require(
    typeof binary === "string" && binary.startsWith("/"),
    "native CLI must be absolute",
  );
  require(
    typeof receipt === "string" && receipt.startsWith("/"),
    "receipt must be absolute",
  );
  try {
    await Deno.lstat(receipt);
    throw new Error("receipt already exists");
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
  require(
    await Deno.readLink("/proc/self/ns/net") !==
      await Deno.readLink("/proc/1/ns/net"),
    "run inside a fresh network namespace",
  );
  const interfaces = [
    ...new Set(Deno.networkInterfaces().map((entry) => entry.name)),
  ];
  require(
    interfaces.length === 1 && interfaces[0] === "lo",
    "only loopback may exist",
  );
  const bytes = await Deno.readFile(binary);
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
  )
    .map((value) => value.toString(16).padStart(2, "0")).join("");
  require(digest === values.sha256, "native CLI digest differs");
  const root = await Deno.makeTempDir({ prefix: "cowboy-executor-lifetime-" });
  const target = `${root}/target`;
  await Deno.mkdir(target);
  await Deno.mkdir(`${root}/home`);
  const environment = {
    HOME: `${root}/home`,
    CODEX_HOME: `${root}/home/.codex`,
    PATH: "/run/current-system/sw/bin",
  };
  const version = await new Deno.Command(binary, {
    args: ["--version"],
    clearEnv: true,
    env: environment,
    cwd: target,
    stdout: "piped",
    stderr: "null",
  }).output();
  require(
    version.success &&
      new TextDecoder().decode(version.stdout).trim() ===
        `codex-cli ${values.version}`,
    "native version differs",
  );
  const port = Deno.listen({ hostname: "127.0.0.1", port: 0 });
  const url = `ws://127.0.0.1:${(port.addr as Deno.NetAddr).port}`;
  port.close();
  const process = new Deno.Command(binary, {
    args: ["exec-server", "--listen", url],
    clearEnv: true,
    env: environment,
    cwd: target,
    stdin: "null",
    stdout: "null",
    stderr: "null",
  }).spawn();
  const checks = ["exact_native_version_and_digest"];
  let connection: Connection | undefined;
  let cleaned = false;
  const startedAt = performance.now();
  try {
    for (let attempt = 0; attempt < 50; attempt++) {
      try {
        connection = await Connection.open(url);
        break;
      } catch {
        await delay(100);
      }
    }
    require(connection, "executor did not become ready");
    const initialized = await connection.initialize();
    const sessionId = initialized.sessionId;
    require(typeof sessionId === "string", "executor session missing");
    const processId = crypto.randomUUID();
    const command = {
      processId,
      argv: [
        "/run/current-system/sw/bin/bash",
        "-c",
        "printf '%s' \"$$\" > process.pid; printf 'once\\n' >> starts; exec /run/current-system/sw/bin/sleep 180",
      ],
      cwd: new URL(`file://${target}`).href,
      env: {},
      tty: false,
      pipeStdin: false,
      arg0: null,
      envPolicy: {
        inherit: "none",
        ignoreDefaultExcludes: false,
        exclude: [],
        set: {},
        includeOnly: [],
      },
    };
    await connection.request("process/start", command);
    await connection.request("process/start", command, true);
    await connection.request("process/start", {
      ...command,
      argv: ["/run/current-system/sw/bin/false"],
    }, true);
    require(
      await Deno.readTextFile(`${target}/starts`) === "once\n",
      "duplicate process start executed",
    );
    checks.push("duplicate_and_changed_process_id_refuse_reexecution");
    await connection.close();
    await delay(500);
    connection = await Connection.open(url);
    require(
      (await connection.initialize(sessionId)).sessionId === sessionId,
      "short reconnect changed identity",
    );
    let state = await connection.request("process/read", {
      processId,
      waitMs: 1,
      maxBytes: 1024,
    });
    require(!state.exited, "short disconnect killed the owned process");
    checks.push("short_disconnect_resumes_original_process");

    // A target-local keeper can retain this connection while the network to
    // the Agent or Controller is absent. No app-server is needed for retention.
    await delay(35000);
    state = await connection.request("process/read", {
      processId,
      waitMs: 1,
      maxBytes: 1024,
    });
    require(!state.exited, "attached target owner did not retain the process");
    checks.push("attached_target_owner_retains_process_beyond_disconnect_ttl");
    await connection.close();
    await delay(35000);
    connection = await Connection.open(url);
    await connection.request("initialize", {
      clientName: "expired-fixture",
      resumeSessionId: sessionId,
    }, true);
    const pid = (await Deno.readTextFile(`${target}/process.pid`)).trim();
    require(/^[1-9][0-9]{0,9}$/.test(pid), "invalid fixture process identity");
    const alive = await new Deno.Command("/run/current-system/sw/bin/kill", {
      args: ["-0", pid],
      clearEnv: true,
      env: environment,
      stdout: "null",
      stderr: "null",
    }).output();
    require(!alive.success, "expired executor session retained a live process");
    require(
      await Deno.readTextFile(`${target}/starts`) === "once\n",
      "reconnect replayed the start",
    );
    checks.push("long_detach_expires_session_and_stops_process_without_replay");
  } finally {
    await connection?.close();
    await stop(process);
    await Deno.remove(root, { recursive: true });
    cleaned = true;
  }
  const result = {
    schema: "cowboy.executor-lifetime-probe/v1",
    accepted: true,
    native_cli: { version: values.version, sha256: digest },
    checks,
    topology: "isolated_loopback_native_executor",
    elapsed_ms: Math.round(performance.now() - startedAt),
    attached_observation_ms: 35000,
    detached_observation_ms: 35000,
    cleanup: cleaned,
    real_model_requests: 0,
    production_credentials: false,
    proves_cowboy_routing: false,
    proves_target_keeper_implementation: false,
    proves_file_write_idempotency: false,
    proves_subscription: false,
  };
  await Deno.writeTextFile(receipt, `${JSON.stringify(result, null, 2)}\n`, {
    createNew: true,
    mode: 0o600,
  });
  console.log(
    JSON.stringify({
      accepted: true,
      checks: checks.length,
      real_model_requests: 0,
    }),
  );
}

await main();
