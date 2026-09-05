/* Signed Codex account-usage collector. Executes outside Cowboy core. */

async function input() {
  const text = await new Response(Deno.stdin.readable).text();
  return text.trim() === "" ? { operation: "collect" } : JSON.parse(text);
}

class ResetFailure extends Error {
  constructor(
    message,
    callMayHaveReachedProvider = false,
    creditId = undefined,
  ) {
    super(message);
    this.callMayHaveReachedProvider = callMayHaveReachedProvider;
    this.creditId = creditId;
  }
}

class JsonRpcLines {
  constructor(child) {
    this.child = child;
    this.writer = child.stdin.getWriter();
    this.lines = child.stdout
      .pipeThrough(new TextDecoderStream())
      .pipeThrough(new TextLineStream())[Symbol.asyncIterator]();
    this.nextId = 1;
  }

  static async start() {
    const command = Deno.env.get("COWBOY_PLUGIN_COMMAND_CODEX") ??
      Deno.env.get("COWBOY_CODEX_COMMAND") ?? Deno.args[0] ?? "codex";
    const child = new Deno.Command(command, {
      args: [
        "app-server",
        "--stdio",
        "-c",
        "features.memories=false",
        "-c",
        "analytics.enabled=false",
      ],
      stdin: "piped",
      stdout: "piped",
      stderr: "null",
    }).spawn();
    const rpc = new JsonRpcLines(child);
    await rpc.request("initialize", {
      clientInfo: { name: "cowboy-usage", title: "Cowboy", version: "1" },
      capabilities: { experimentalApi: true },
    });
    await rpc.notify("initialized", {});
    return rpc;
  }

  async write(value) {
    await this.writer.write(
      new TextEncoder().encode(`${JSON.stringify(value)}\n`),
    );
  }

  async notify(method, params) {
    await this.write({ method, params });
  }

  async request(method, params) {
    const id = this.nextId++;
    await this.write({ id, method, params });
    for (;;) {
      const next = await this.lines.next();
      if (next.done) throw new Error("Codex App Server closed");
      let message;
      try {
        message = JSON.parse(next.value);
      } catch {
        continue;
      }
      if (message?.id !== id) continue;
      if (message.error !== undefined) {
        throw new Error(`${method}: ${JSON.stringify(message.error)}`);
      }
      return message.result ?? null;
    }
  }

  close() {
    try {
      this.child.kill("SIGTERM");
    } catch {
      // The provider may already have exited.
    }
  }
}

export function nearestCredit(rateLimits) {
  const credits = rateLimits?.rateLimitResetCredits?.credits;
  if (!Array.isArray(credits)) return undefined;
  return credits
    .filter((credit) =>
      credit?.status === "available" && typeof credit.id === "string"
    )
    .sort((left, right) =>
      (left.expiresAt ?? Number.MAX_SAFE_INTEGER) -
        (right.expiresAt ?? Number.MAX_SAFE_INTEGER) ||
      (left.grantedAt ?? Number.MAX_SAFE_INTEGER) -
        (right.grantedAt ?? Number.MAX_SAFE_INTEGER) ||
      left.id.localeCompare(right.id)
    )[0]?.id;
}

async function collect(rpc) {
  const account = await rpc.request("account/read", { refreshToken: false });
  const rateLimits = await rpc.request("account/rateLimits/read", {});
  let activity;
  try {
    activity = await rpc.request("account/usage/read", {});
  } catch {
    activity = undefined;
  }
  return {
    provider: "openai",
    status: "available",
    source: "Codex App Server",
    observed_at_ms: Date.now(),
    account,
    rate_limits: rateLimits,
    ...(activity === undefined ? {} : { activity }),
  };
}

async function reset(rpc, request) {
  const rateLimits = await rpc.request("account/rateLimits/read", {});
  const creditId = nearestCredit(rateLimits);
  if (!creditId) throw new ResetFailure("no available reset credit");
  if (request.expected_credit_id && request.expected_credit_id !== creditId) {
    throw new ResetFailure(
      "nearest reset credit changed; refresh and confirm again",
      false,
      creditId,
    );
  }
  let result;
  try {
    result = await rpc.request("account/rateLimitResetCredit/consume", {
      creditId,
      idempotencyKey: request.idempotency_key,
    });
  } catch (error) {
    throw new ResetFailure(
      error instanceof Error ? error.message : String(error),
      true,
      creditId,
    );
  }
  return { outcome: result?.outcome ?? "unknown", credit_id: creditId };
}

async function main() {
  let rpc;
  let request;
  try {
    request = await input();
    rpc = await JsonRpcLines.start();
    const result = request.operation === "consume_reset"
      ? await reset(rpc, request)
      : await collect(rpc);
    console.log(JSON.stringify(result));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (request?.operation === "consume_reset") {
      console.log(JSON.stringify({
        error: {
          message,
          call_may_have_reached_provider: error instanceof ResetFailure
            ? error.callMayHaveReachedProvider
            : false,
          ...(error instanceof ResetFailure && error.creditId
            ? { credit_id: error.creditId }
            : {}),
        },
      }));
    } else {
      console.error(message);
      Deno.exitCode = 1;
    }
  } finally {
    rpc?.close();
  }
}

if (import.meta.main) {
  await main();
}
