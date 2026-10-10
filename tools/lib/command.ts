// One subprocess shape for the repository's build and conformance tools:
// explicit stdio, an explicit environment, and a result that never throws on a
// non-zero exit. Bun.spawn is the only implementation.

export type Stdio = "piped" | "inherit" | "null";

export interface CommandOptions {
  args?: readonly string[];
  cwd?: string | URL;
  /** Variables for the child. Added to this process's environment unless
   * `clearEnv` is set, in which case they are the whole environment. */
  env?: Record<string, string>;
  clearEnv?: boolean;
  stdin?: Stdio;
  stdout?: Stdio;
  stderr?: Stdio;
  /** Kills the child when aborted. */
  signal?: AbortSignal;
}

export interface CommandStatus {
  success: boolean;
  code: number;
  signal: string | null;
}

export interface CommandOutput extends CommandStatus {
  stdout: Uint8Array;
  stderr: Uint8Array;
}

const EMPTY = new Uint8Array();

function stdio(mode: Stdio): "pipe" | "inherit" | "ignore" {
  return mode === "piped" ? "pipe" : mode === "null" ? "ignore" : "inherit";
}

function environment(options: CommandOptions): Record<string, string> {
  if (options.clearEnv) return { ...options.env };
  const inherited: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (value !== undefined) inherited[name] = value;
  }
  return { ...inherited, ...options.env };
}

async function bytes(
  stream: ReadableStream<Uint8Array> | undefined,
): Promise<Uint8Array> {
  return stream ? new Uint8Array(await new Response(stream).arrayBuffer()) : EMPTY;
}

export class ChildProcess {
  readonly pid: number;
  readonly status: Promise<CommandStatus>;
  readonly #child: ReturnType<typeof Bun.spawn>;

  constructor(child: ReturnType<typeof Bun.spawn>) {
    this.#child = child;
    this.pid = child.pid;
    this.status = child.exited.then((code) => ({
      success: code === 0,
      code,
      signal: child.signalCode ?? null,
    }));
  }

  /** Present only when the stream was requested as "piped". */
  get stdout(): ReadableStream<Uint8Array> {
    return this.#stream(this.#child.stdout, "stdout");
  }

  get stderr(): ReadableStream<Uint8Array> {
    return this.#stream(this.#child.stderr, "stderr");
  }

  get stdin(): { write(chunk: string | Uint8Array): void; end(): void } {
    const sink = this.#child.stdin;
    if (sink === undefined || typeof sink === "number") {
      throw new TypeError("stdin was not piped");
    }
    return {
      write: (chunk) => void sink.write(chunk),
      end: () => void sink.end(),
    };
  }

  kill(signal: NodeJS.Signals | number = "SIGTERM"): void {
    this.#child.kill(signal);
  }

  async output(): Promise<CommandOutput> {
    const out = this.#child.stdout;
    const err = this.#child.stderr;
    const [stdout, stderr, status] = await Promise.all([
      bytes(out instanceof ReadableStream ? out : undefined),
      bytes(err instanceof ReadableStream ? err : undefined),
      this.status,
    ]);
    return { ...status, stdout, stderr };
  }

  #stream(value: unknown, name: string): ReadableStream<Uint8Array> {
    if (value instanceof ReadableStream) return value;
    throw new TypeError(`${name} was not piped`);
  }
}

export class Command {
  readonly #command: string;
  readonly #options: CommandOptions;

  constructor(command: string | URL, options: CommandOptions = {}) {
    this.#command = command instanceof URL ? Bun.fileURLToPath(command) : command;
    this.#options = options;
  }

  /** Start the child. Streams are inherited unless requested otherwise. */
  spawn(): ChildProcess {
    return this.#start("inherit");
  }

  /** Run to completion, capturing stdout and stderr unless told otherwise. */
  output(): Promise<CommandOutput> {
    return this.#start("piped", "null").output();
  }

  #start(defaultOutput: Stdio, defaultInput: Stdio = "inherit"): ChildProcess {
    const options = this.#options;
    const cwd = options.cwd instanceof URL
      ? Bun.fileURLToPath(options.cwd)
      : options.cwd;
    // The executable is found on the child's PATH when it sets one, else on
    // this process's, so a cleared environment can still name a tool.
    const path = options.env?.PATH ?? process.env.PATH ?? "";
    const executable = this.#command.includes("/")
      ? this.#command
      : Bun.which(this.#command, { PATH: path }) ?? this.#command;
    return new ChildProcess(Bun.spawn({
      cmd: [executable, ...(options.args ?? [])],
      ...(cwd === undefined ? {} : { cwd }),
      env: environment(options),
      stdin: stdio(options.stdin ?? defaultInput),
      stdout: stdio(options.stdout ?? defaultOutput),
      stderr: stdio(options.stderr ?? defaultOutput),
      ...(options.signal === undefined ? {} : { signal: options.signal }),
    }));
  }
}
