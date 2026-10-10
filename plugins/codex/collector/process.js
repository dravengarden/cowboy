// Child processes for this collector.
//
// A Plugin package is self-contained, so each collector that starts its
// provider CLI carries this module instead of sharing one.

const STDIO = { piped: "pipe", inherit: "inherit", null: "ignore" };

function writable(sink) {
  return new WritableStream({
    async write(chunk) {
      sink.write(chunk);
      await sink.flush();
    },
    async close() {
      await sink.end();
    },
  });
}

class ChildProcess {
  #child;
  #stdin;

  constructor(child) {
    this.#child = child;
    this.pid = child.pid;
    this.status = child.exited.then((code) => ({
      success: code === 0,
      code: code ?? 1,
      signal: child.signalCode ?? null,
    }));
  }

  get stdin() {
    return this.#stdin ??= writable(this.#child.stdin);
  }

  get stdout() {
    return this.#child.stdout;
  }

  get stderr() {
    return this.#child.stderr;
  }

  kill(signal = "SIGTERM") {
    this.#child.kill(signal);
  }
}

/** Start `command` with `options.args`. `env` extends the host environment
 * unless `clearEnv` is set; each stdio stream is "piped", "inherit" or "null".
 */
export function spawn(command, options = {}) {
  const env = options.clearEnv
    ? { ...options.env }
    : { ...process.env, ...options.env };
  // Found on the child's PATH when it sets one, else on the host's, so a
  // cleared environment can still name a command.
  const executable = command.includes("/")
    ? command
    : Bun.which(command, { PATH: env.PATH ?? process.env.PATH ?? "" }) ??
      command;
  return new ChildProcess(Bun.spawn({
    cmd: [executable, ...(options.args ?? [])],
    env,
    stdin: STDIO[options.stdin ?? "inherit"],
    stdout: STDIO[options.stdout ?? "inherit"],
    stderr: STDIO[options.stderr ?? "inherit"],
  }));
}
