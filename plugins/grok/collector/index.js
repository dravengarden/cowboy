/* Signed xAI account-usage/reset collector. Executes outside Cowboy core. */

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const WEB_BASE = "https://grok.com";
const CHILD_ENV_KEYS = [
  "HOME",
  "PATH",
  "TMPDIR",
  "USER",
  "XDG_CONFIG_HOME",
  "XDG_DATA_HOME",
  "XDG_CACHE_HOME",
  "SSL_CERT_FILE",
  "NIX_SSL_CERT_FILE",
];

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

class AcpRpc {
  constructor(child) {
    this.child = child;
    this.writer = child.stdin.getWriter();
    this.lines = child.stdout
      .pipeThrough(new TextDecoderStream())
      .pipeThrough(new TextLineStream())[Symbol.asyncIterator]();
    this.nextId = 1;
  }

  static async start() {
    const command = Deno.env.get("COWBOY_PLUGIN_COMMAND_GROK") ??
      Deno.env.get("COWBOY_ACP_GROK_CMD") ?? Deno.args[0] ?? "grok";
    const env = { GROK_FOLDER_TRUST: "0" };
    for (const key of CHILD_ENV_KEYS) {
      const value = Deno.env.get(key);
      if (value) env[key] = value;
    }
    const child = new Deno.Command(command, {
      args: [
        "--no-auto-update",
        "--experimental-memory",
        "--rules",
        "Read and follow the closest AGENTS.md project instructions before taking any action.",
        "agent",
        "--always-approve",
        "--no-leader",
        "stdio",
      ],
      env,
      stdin: "piped",
      stdout: "piped",
      stderr: "null",
    }).spawn();
    const rpc = new AcpRpc(child);
    await rpc.request("initialize", {
      protocolVersion: 1,
      clientCapabilities: {
        fs: { readTextFile: false, writeTextFile: false },
        terminal: false,
      },
      clientInfo: { name: "cowboy-usage", title: "Cowboy", version: "1" },
    });
    return rpc;
  }

  async request(method, params) {
    const id = this.nextId++;
    await this.writer.write(
      encoder.encode(
        `${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`,
      ),
    );
    for (;;) {
      const next = await this.lines.next();
      if (next.done) throw new Error("Grok ACP closed");
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

export function credentialFromJson(document) {
  const values =
    document && typeof document === "object" && !Array.isArray(document)
      ? [document, ...Object.values(document)]
      : [];
  for (const value of values) {
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    if (
      value.auth_mode === "oidc" && typeof value.key === "string" &&
      value.key.trim() !== "" &&
      typeof value.user_id === "string" && value.user_id.trim() !== ""
    ) {
      return { key: value.key.trim(), userId: value.user_id.trim() };
    }
  }
  return undefined;
}

async function loadCredential() {
  const inline = Deno.env.get("GROK_AUTH");
  if (inline?.trim()) return credentialFromJson(JSON.parse(inline));
  const paths = [
    Deno.env.get("GROK_AUTH_PATH"),
    Deno.env.get("GROK_HOME")
      ? `${Deno.env.get("GROK_HOME")}/auth.json`
      : undefined,
    Deno.env.get("HOME")
      ? `${Deno.env.get("HOME")}/.grok/auth.json`
      : undefined,
  ];
  for (const path of paths) {
    if (!path) continue;
    try {
      const credential = credentialFromJson(
        JSON.parse(await Deno.readTextFile(path)),
      );
      if (credential) return credential;
    } catch {
      // Try the next declared credential source.
    }
  }
  return undefined;
}

function authHeaders(credential, contentType) {
  return {
    authorization: `Bearer ${credential.key}`,
    "x-xai-token-auth": "xai-grok-cli",
    "x-userid": credential.userId,
    ...(contentType ? { "content-type": contentType, "x-grpc-web": "1" } : {}),
  };
}

async function boundedBytes(response, operation) {
  if (!response.ok) {
    throw new Error(`${operation} returned HTTP ${response.status}`);
  }
  const declared = Number(response.headers.get("content-length") ?? "0");
  if (declared > 1024 * 1024) {
    throw new Error(`${operation} response is too large`);
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.length > 1024 * 1024) {
    throw new Error(`${operation} response is too large`);
  }
  return bytes;
}

export function activePlan(document) {
  const labels = {
    SUBSCRIPTION_TIER_SUPER_GROK_LITE: [1, "SuperGrok Lite"],
    SUBSCRIPTION_TIER_GROK_PRO: [2, "SuperGrok"],
    SUBSCRIPTION_TIER_SUPER_GROK_PLUS: [3, "SuperGrok Plus"],
    SUBSCRIPTION_TIER_SUPER_GROK_PRO: [4, "SuperGrok Heavy"],
  };
  return Array.isArray(document?.subscriptions)
    ? document.subscriptions
      .filter((value) =>
        value?.status === "SUBSCRIPTION_STATUS_ACTIVE" && labels[value.tier]
      )
      .map((value) => labels[value.tier])
      .sort((left, right) => right[0] - left[0])[0]?.[1]
    : undefined;
}

function encodeVarint(value) {
  const bytes = [];
  let remaining = Number(value);
  do {
    let byte = remaining & 0x7f;
    remaining = Math.floor(remaining / 128);
    if (remaining > 0) byte |= 0x80;
    bytes.push(byte);
  } while (remaining > 0);
  return bytes;
}

export function encodeStringField(number, value) {
  const bytes = encoder.encode(value);
  return new Uint8Array([
    ...encodeVarint(number * 8 + 2),
    ...encodeVarint(bytes.length),
    ...bytes,
  ]);
}

function frameMessage(message) {
  const framed = new Uint8Array(message.length + 5);
  new DataView(framed.buffer).setUint32(1, message.length, false);
  framed.set(message, 5);
  return framed;
}

function decodeVarint(bytes, offset) {
  let value = 0;
  let scale = 1;
  for (let index = 0; index < 10 && offset + index < bytes.length; index++) {
    const byte = bytes[offset + index];
    value += (byte & 0x7f) * scale;
    if ((byte & 0x80) === 0) return [value, offset + index + 1];
    scale *= 128;
  }
  throw new Error("invalid protobuf varint");
}

function decodeFields(bytes) {
  const fields = [];
  let offset = 0;
  while (offset < bytes.length) {
    let tag;
    [tag, offset] = decodeVarint(bytes, offset);
    const number = Math.floor(tag / 8);
    const wire = tag & 7;
    if (wire === 0) {
      let value;
      [value, offset] = decodeVarint(bytes, offset);
      fields.push({ number, wire, value });
    } else if (wire === 2) {
      let length;
      [length, offset] = decodeVarint(bytes, offset);
      const end = offset + length;
      if (end > bytes.length) throw new Error("truncated protobuf field");
      fields.push({ number, wire, value: bytes.slice(offset, end) });
      offset = end;
    } else if (wire === 1) {
      offset += 8;
    } else if (wire === 5) {
      offset += 4;
    } else {
      throw new Error(`unsupported protobuf wire type ${wire}`);
    }
    if (offset > bytes.length) throw new Error("truncated protobuf scalar");
  }
  return fields;
}

function timestamp(bytes) {
  return decodeFields(bytes).find((field) =>
    field.number === 1 && field.wire === 0
  )?.value;
}

export function resetTokens(message) {
  return decodeFields(message)
    .filter((field) => field.number === 10 && field.wire === 2)
    .map((field) => {
      const fields = decodeFields(field.value);
      const idField = fields.find((value) =>
        value.number === 10 && value.wire === 2
      );
      if (!idField) throw new Error("reset token has no id");
      return {
        id: decoder.decode(idField.value),
        grantedAt: fields.find((value) =>
            value.number === 20 && value.wire === 2
          )
          ? timestamp(
            fields.find((value) => value.number === 20 && value.wire === 2)
              .value,
          )
          : undefined,
        expiresAt:
          fields.find((value) => value.number === 30 && value.wire === 2)
            ? timestamp(
              fields.find((value) => value.number === 30 && value.wire === 2)
                .value,
            )
            : undefined,
      };
    });
}

function decodeGrpcWeb(body) {
  let offset = 0;
  let message;
  let status;
  while (offset < body.length) {
    if (offset + 5 > body.length) {
      throw new Error("truncated gRPC-web envelope");
    }
    const flags = body[offset];
    const length = new DataView(body.buffer, body.byteOffset + offset + 1, 4)
      .getUint32(0, false);
    offset += 5;
    const payload = body.slice(offset, offset + length);
    if (payload.length !== length) {
      throw new Error("truncated gRPC-web payload");
    }
    if (flags === 0) message = payload;
    else if (flags === 0x80) {
      const match = decoder.decode(payload).match(
        /(?:^|\r?\n)grpc-status:\s*(\d+)/i,
      );
      status = match ? Number(match[1]) : undefined;
    } else throw new Error(`unsupported gRPC-web flags ${flags}`);
    offset += length;
  }
  if (status !== 0 || !message) {
    throw new Error(`gRPC-web status ${status ?? "missing"}`);
  }
  return message;
}

async function grpcWeb(credential, path, message = new Uint8Array()) {
  const response = await fetch(`${WEB_BASE}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: authHeaders(credential, "application/grpc-web+proto"),
    body: frameMessage(message),
    signal: AbortSignal.timeout(10_000),
  });
  return decodeGrpcWeb(await boundedBytes(response, `xAI RPC ${path}`));
}

async function accountMetadata() {
  const credential = await loadCredential();
  if (!credential) return {};
  const [planResult, resetsResult] = await Promise.allSettled([
    fetch(`${WEB_BASE}/rest/subscriptions`, {
      headers: authHeaders(credential),
      redirect: "manual",
      signal: AbortSignal.timeout(10_000),
    }).then((response) => boundedBytes(response, "xAI subscriptions"))
      .then((bytes) => activePlan(JSON.parse(decoder.decode(bytes)))),
    grpcWeb(credential, "/prod_mc_billing.ConsumerUiSvc/GetRemainingResets")
      .then(resetTokens),
  ]);
  return {
    ...(planResult.status === "fulfilled" && planResult.value
      ? { plan: planResult.value }
      : {}),
    ...(resetsResult.status === "fulfilled"
      ? { resets: resetsResult.value }
      : {}),
  };
}

function attachMetadata(billing, metadata) {
  const tier = billing?.subscription_tier ?? billing?.subscriptionTier;
  const plan = metadata.plan ??
    (typeof tier === "string" && tier.trim() ? tier : undefined);
  if (Array.isArray(metadata.resets)) {
    billing.rateLimitResetCredits = {
      availableCount: metadata.resets.length,
      credits: metadata.resets.map((reset) => ({
        id: reset.id,
        status: "available",
        title: "Usage reset",
        grantedAt: reset.grantedAt,
        expiresAt: reset.expiresAt,
      })),
    };
  }
  return { billing, plan };
}

export function nearestCredit(credits) {
  return [...credits]
    .filter((credit) => typeof credit.id === "string")
    .sort((left, right) =>
      (left.expiresAt ?? Number.MAX_SAFE_INTEGER) -
        (right.expiresAt ?? Number.MAX_SAFE_INTEGER) ||
      (left.grantedAt ?? Number.MAX_SAFE_INTEGER) -
        (right.grantedAt ?? Number.MAX_SAFE_INTEGER) ||
      left.id.localeCompare(right.id)
    )[0];
}

async function collect(rpc) {
  const billing = await rpc.request("_x.ai/billing", {});
  const metadata = await accountMetadata();
  const merged = attachMetadata(billing, metadata);
  return {
    provider: "xai",
    status: "available",
    source: "xAI",
    observed_at_ms: Date.now(),
    ...(merged.plan ? { account: { account: { planType: merged.plan } } } : {}),
    rate_limits: merged.billing,
  };
}

async function reset(rpc, request) {
  await rpc.request("_x.ai/billing", {});
  const credential = await loadCredential();
  if (!credential) {
    throw new ResetFailure("official Grok OIDC credential is unavailable");
  }
  const credits = resetTokens(
    await grpcWeb(
      credential,
      "/prod_mc_billing.ConsumerUiSvc/GetRemainingResets",
    ),
  ).filter((credit) =>
    credit.expiresAt === undefined || credit.expiresAt > Date.now() / 1000
  );
  const credit = nearestCredit(credits);
  if (!credit) throw new ResetFailure("no available reset");
  if (request.expected_credit_id && request.expected_credit_id !== credit.id) {
    throw new ResetFailure(
      "nearest reset credit changed; refresh and confirm again",
      false,
      credit.id,
    );
  }
  let remaining;
  try {
    remaining = resetTokens(
      await grpcWeb(
        credential,
        "/prod_mc_billing.ConsumerUiSvc/RedeemReset",
        encodeStringField(10, credit.id),
      ),
    ).length;
  } catch (error) {
    throw new ResetFailure(
      error instanceof Error ? error.message : String(error),
      true,
      credit.id,
    );
  }
  return {
    outcome: `consumed; ${remaining} reset${
      remaining === 1 ? "" : "s"
    } remaining`,
    credit_id: credit.id,
  };
}

async function main() {
  let rpc;
  let request;
  try {
    request = await input();
    rpc = await AcpRpc.start();
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
