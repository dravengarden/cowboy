// The `@plugin-js` API for signed Plugin collectors, on Bun.
//
// Collectors were written against the previous runtime's `Deno` global, and
// packages installed on a Machine outlive the Machine release that runs them.
// The host preloads this module so one collector source runs on a host of
// either generation. It implements only the surface signed collectors use;
// delete it once no supported Plugin release refers to `Deno`.

const STDIO = { piped: "pipe", inherit: "inherit", null: "ignore" };

function environment(options) {
  if (options.clearEnv) return { ...options.env };
  return { ...process.env, ...options.env };
}

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

async function bytes(stream) {
  return stream instanceof ReadableStream
    ? new Uint8Array(await new Response(stream).arrayBuffer())
    : new Uint8Array();
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

  async output() {
    const [stdout, stderr, status] = await Promise.all([
      bytes(this.#child.stdout),
      bytes(this.#child.stderr),
      this.status,
    ]);
    return { ...status, stdout, stderr };
  }
}

class Command {
  #command;
  #options;

  constructor(command, options = {}) {
    this.#command = String(command);
    this.#options = options;
  }

  spawn() {
    return this.#start("inherit", "inherit");
  }

  output() {
    return this.#start("piped", "null").output();
  }

  #start(output, input) {
    const options = this.#options;
    const env = environment(options);
    // Found on the child's PATH when it sets one, else on the host's, so a
    // cleared environment can still name a command.
    const executable = this.#command.includes("/")
      ? this.#command
      : Bun.which(this.#command, { PATH: env.PATH ?? process.env.PATH ?? "" }) ??
        this.#command;
    return new ChildProcess(Bun.spawn({
      cmd: [executable, ...(options.args ?? [])],
      ...(options.cwd === undefined ? {} : { cwd: String(options.cwd) }),
      env,
      stdin: STDIO[options.stdin ?? input],
      stdout: STDIO[options.stdout ?? output],
      stderr: STDIO[options.stderr ?? output],
    }));
  }
}

let stdinReadable;

globalThis.Deno = {
  args: process.argv.slice(2),
  env: {
    get: (name) => process.env[name],
    has: (name) => process.env[name] !== undefined,
    set: (name, value) => {
      process.env[name] = value;
    },
    delete: (name) => {
      delete process.env[name];
    },
    toObject: () => ({ ...process.env }),
  },
  get exitCode() {
    return process.exitCode ?? 0;
  },
  set exitCode(code) {
    process.exitCode = code;
  },
  exit: (code) => process.exit(code),
  stdin: {
    get readable() {
      return stdinReadable ??= Bun.stdin.stream();
    },
  },
  stdout: {
    write: async (chunk) => {
      await Bun.write(Bun.stdout, chunk);
      return chunk.byteLength;
    },
  },
  readTextFile: (path) => Bun.file(path).text(),
  Command,
};
