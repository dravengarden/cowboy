import { test } from "bun:test";
import assert from "node:assert/strict";
import claudeCodeHost from "./claude-code/host.json" with { type: "json" };
import { accountView, KNOWN_PLANS } from "./claude-code/collector/index.js";
import { collectorTargets, group } from "./claude-deepseek/collector/index.js";
import {
  collect as collectCodex,
  isTokenExpired as codexTokenExpired,
  nearestCredit as nearestCodexCredit,
  readLines as codexLines,
} from "./codex/collector/index.js";
import billingCapture from "./grok-billing-cli.fixture.json" with { type: "json" };
import {
  AuthenticationRequired,
  activePlan,
  billingProjection,
  credentialFromJson,
  encodeStringField,
  GROK_CLI_VERSION,
  handle as handleGrok,
  nearestCredit as nearestGrokCredit,
  readLines as grokLines,
} from "./grok/collector/index.js";
import grokDeclaration from "./grok/collector/portable-account.json" with { type: "json" };
import grokHost from "./grok/host.json" with { type: "json" };
import grokProvider from "./grok/provider.json" with { type: "json" };

function equal(actual, expected, message) {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(`${message}: got ${left}, expected ${right}`);
  }
}

for (const [provider, readLines] of [
  ["Codex", codexLines],
  ["Grok", grokLines],
]) {
  test(`${provider} collector reads fragmented UTF-8 JSON RPC without nonstandard stream globals`, async () => {
    const bytes = new TextEncoder().encode('{"id":1,"result":"中文"}\r\n{"id":2}\n{"id":3}');
    const stream = new ReadableStream({
      start(controller) {
        for (const byte of bytes) controller.enqueue(new Uint8Array([byte]));
        controller.close();
      },
    });
    const messages = [];
    for await (const line of readLines(stream)) messages.push(JSON.parse(line));
    equal(messages, [{ id: 1, result: "中文" }, { id: 2 }, { id: 3 }], "RPC messages");
  });
}

test("Codex reset selection is deterministic and ignores unavailable credits", () => {
  equal(
    nearestCodexCredit({
      rateLimitResetCredits: {
        credits: [
          { id: "later", status: "available", expiresAt: 20 },
          { id: "spent", status: "consumed", expiresAt: 1 },
          { id: "first", status: "available", expiresAt: 10 },
        ],
      },
    }),
    "first",
    "nearest Codex credit",
  );
});

test("DeepSeek collector groups shared account fingerprints without duplicating lanes", () => {
  equal(
    group([
      {
        account_fingerprint: "same",
        agent: "codex",
        is_available: true,
        balance_infos: [1],
      },
      {
        account_fingerprint: "same",
        agent: "claude",
        is_available: false,
        balance_infos: [2],
      },
      {
        account_fingerprint: "same",
        agent: "codex",
        is_available: true,
        balance_infos: [3],
      },
    ]),
    [
      {
        accountFingerprint: "same",
        agents: ["claude", "codex"],
        isAvailable: true,
        balanceInfos: [1],
      },
    ],
    "grouped balance account",
  );
});

test("DeepSeek exact sidecar targets bind by lane id rather than URL order", () => {
  equal(
    collectorTargets(
      JSON.stringify([
        { id: "claude", url: "http://127.0.0.1:4102/provider-info" },
        { id: "codex", url: "http://127.0.0.1:4101/provider-info" },
      ]),
      "http://legacy.invalid/first,http://legacy.invalid/second",
      ["codex", "claude"],
      "deepseek",
    ),
    [
      { id: "claude", url: "http://127.0.0.1:4102/provider-info" },
      { id: "codex", url: "http://127.0.0.1:4101/provider-info" },
    ],
    "exact sidecar targets",
  );
});

test("Grok collector extracts OIDC credentials and highest active plan", () => {
  equal(
    credentialFromJson({
      profile: { auth_mode: "oidc", key: " token ", user_id: " user " },
    }),
    {
      key: "token",
      userId: "user",
    },
    "Grok credential",
  );
  equal(
    activePlan({
      subscriptions: [
        {
          status: "SUBSCRIPTION_STATUS_ACTIVE",
          tier: "SUBSCRIPTION_TIER_GROK_PRO",
        },
        {
          status: "SUBSCRIPTION_STATUS_ACTIVE",
          tier: "SUBSCRIPTION_TIER_SUPER_GROK_PRO",
        },
      ],
    }),
    "SuperGrok Heavy",
    "Grok plan",
  );
});

test("Grok reset selection and protobuf string encoding stay deterministic", () => {
  equal(
    nearestGrokCredit([
      { id: "later", expiresAt: 20 },
      { id: "first", expiresAt: 10 },
    ]).id,
    "first",
    "nearest Grok credit",
  );
  equal([...encodeStringField(10, "abc")], [82, 3, 97, 98, 99], "protobuf field");
});

test("Anthropic collector projects only renderable account fields", () => {
  equal(
    accountView({
      subscriptionType: "max",
      email: "user@example.com",
      orgName: "Example Org",
      authMethod: "claude.ai",
      orgId: "69b1849f-1e71-41e4-94b0-c39529299651",
    }),
    {
      account: {
        account: {
          planType: "max",
          email: "user@example.com",
          organization: "Example Org",
          authMethod: "claude.ai",
        },
      },
    },
    "Anthropic account view",
  );
  // An unrecognised plan must still badge as a subscription rather than leak a
  // raw upstream identifier into the card.
  equal(
    accountView({ subscriptionType: "some_future_tier" }).account.account.planType,
    "subscription",
    "unknown Anthropic plan",
  );
  equal(accountView({}), {}, "absent Anthropic account");
  equal(accountView({ subscriptionType: "   " }), {}, "blank Anthropic plan");
});

test("Anthropic declares reset through its signed collector", () => {
  const usage = claudeCodeHost.usage;
  equal(usage.reset, "anthropic", "Anthropic reset verb");
  equal(usage.reset_argv, usage.collector_argv, "Anthropic reset argv");
  equal(usage.order, 0, "Anthropic card order");
  // The widget renders by shape, so the opaque id only has to be stable. It
  // stays silent until the native query or a session event reports a numeric
  // window; unknown limits must not become a full remaining balance.
  equal(usage.widget_shape, "percent", "Anthropic widget shape");
  equal(usage.widget_window, 10080, "Anthropic widget window");
  equal(usage.top_bar_windows, [300, 10080], "Anthropic top bar windows");
  // Compound labels are filtered out of the top bar, so every declared window
  // must have an account-wide label backing it.
  const accountWide = usage.limit_labels
    .filter((label) => !label.label.includes(" · "))
    .map((label) => label.window_minutes);
  equal(
    usage.top_bar_windows.every((w) => accountWide.includes(w)),
    true,
    "top bar windows have account-wide labels",
  );
  equal(KNOWN_PLANS.has("max") && KNOWN_PLANS.has("pro"), true, "known Anthropic plans");
});

function fakeCodex(responses) {
  const calls = [];
  return {
    calls,
    async request(method, params) {
      calls.push([method, params ?? null]);
      const next = responses[method].shift();
      if (next instanceof Error) throw next;
      return next;
    },
  };
}

const expired = new Error(
  'account/rateLimits/read: {"code":-32603,"message":"failed to fetch codex rate limits: GET https://chatgpt.com/backend-api/wham/usage failed: 401 Unauthorized; body={\\"code\\": \\"token_expired\\"}"}',
);

test("Codex collector refreshes an expired token once and retries", async () => {
  const rpc = fakeCodex({
    "account/read": [
      { account: { type: "chatgpt" } },
      {
        account: { type: "chatgpt", planType: "pro" },
      },
    ],
    "account/rateLimits/read": [expired, { rateLimits: { primary: {} } }],
    "account/usage/read": [{}],
  });
  const result = await collectCodex(rpc);
  equal(result.account, { account: { type: "chatgpt", planType: "pro" } }, "account");
  equal(
    rpc.calls.filter(([method]) => method === "account/read").map(([, p]) => p),
    [{ refreshToken: false }, { refreshToken: true }],
    "refresh only after expiry",
  );
});

test("a named account's Codex poll never redeems the refresh token", async () => {
  const rpc = fakeCodex({
    "account/read": [{ account: { type: "chatgpt" } }],
    "account/rateLimits/read": [expired],
  });
  let message = "";
  await collectCodex(rpc, { operation: "collect", token_refresh: false }).catch((error) => {
    message = error.message;
  });
  equal(message.includes("authentication required"), true, "reports authentication");
  equal(
    rpc.calls.filter(([method]) => method === "account/read").map(([, p]) => p),
    [{ refreshToken: false }],
    "no refresh",
  );
});

test("Codex collector never refreshes on other failures or a second expiry", async () => {
  const other = fakeCodex({
    "account/read": [{ account: { type: "chatgpt" } }],
    "account/rateLimits/read": [new Error("account/rateLimits/read: 503")],
  });
  let failed = false;
  await collectCodex(other).catch(() => {
    failed = true;
  });
  equal([failed, other.calls.length], [true, 2], "other failure");
  const twice = fakeCodex({
    "account/read": [{}, {}],
    "account/rateLimits/read": [expired, expired],
  });
  failed = false;
  await collectCodex(twice).catch(() => {
    failed = true;
  });
  equal([failed, twice.calls.length], [true, 4], "single refresh");
  equal(codexTokenExpired(new Error("plain")), false, "plain error");
});

const grokCredential = { key: "fixture-token", userId: "user-fixture" };

/** Billing answers `body`; plan and resets fail as they do without an account. */
function fakeGrokFetch(billing) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, headers: init?.headers });
    if (url.startsWith("https://cli-chat-proxy.grok.com/")) {
      return typeof billing === "number"
        ? new Response("{}", { status: billing })
        : Response.json(billing);
    }
    return new Response("", { status: 404 });
  };
  return { calls, fetch };
}

test("Grok direct billing reproduces the pinned CLI's _x.ai/billing projection", () => {
  equal(billingCapture.cli_version, GROK_CLI_VERSION, "captured CLI");
  for (const capture of billingCapture.cases) {
    if (capture.cli.result && capture.direct !== "error") {
      equal(billingProjection(capture.body), capture.cli.result, capture.name);
    } else {
      let refused = false;
      try {
        billingProjection(capture.body);
      } catch {
        refused = true;
      }
      equal(refused, true, `${capture.name} is refused like the CLI`);
    }
  }
});

test("Grok direct billing sends the CLI's request and pins its version", async () => {
  const pinned = grokProvider.runtime.dependencies.find((entry) => entry.id === "xai-grok");
  equal(pinned.version, GROK_CLI_VERSION, "header version follows the runtime pin");
  const { calls, fetch } = fakeGrokFetch(billingCapture.cases[0].body);
  await handleGrok(
    { operation: "collect", token_refresh: false },
    { credential: async () => grokCredential, fetch, startRpc: () => assert.fail("spawned") },
  );
  const captured = billingCapture.request;
  equal(calls[0].url, `${captured.base_url}/billing?format=credits`, "billing URL");
  for (const name of [
    "x-xai-token-auth",
    "x-userid",
    "x-grok-client-version",
    "x-grok-client-mode",
  ]) {
    equal(calls[0].headers[name], captured.headers[name], name);
  }
  equal(calls[0].headers.authorization, "Bearer fixture-token", "bearer");
});

test("Grok refresh-free usage never starts the CLI and matches the CLI path", async () => {
  for (const capture of billingCapture.cases.filter((value) => value.cli.result)) {
    if (capture.direct === "error") continue;
    const direct = fakeGrokFetch(capture.body);
    const withoutCli = await handleGrok(
      { operation: "collect", token_refresh: false },
      {
        credential: async () => grokCredential,
        fetch: direct.fetch,
        startRpc: () => assert.fail("token_refresh:false started the Grok CLI"),
      },
    );
    let closed = false;
    const viaCli = await handleGrok(
      { operation: "collect" },
      {
        credential: async () => grokCredential,
        fetch: fakeGrokFetch(capture.body).fetch,
        startRpc: async () => ({
          request: async (method) => {
            equal(method, "_x.ai/billing", "CLI method");
            return structuredClone(capture.cli.result);
          },
          close: () => {
            closed = true;
          },
        }),
      },
    );
    equal(closed, true, "CLI path closes its process");
    equal(
      { ...withoutCli, observed_at_ms: 0 },
      { ...viaCli, observed_at_ms: 0 },
      `${capture.name} renders the same usage`,
    );
  }
});

test("Grok refresh-free usage reports authentication required and never retries", async () => {
  const refuse = () => assert.fail("token_refresh:false started the Grok CLI");
  const outcome = async (credential, billing, now = () => 1_000_000) => {
    const { calls, fetch } = fakeGrokFetch(billing);
    try {
      await handleGrok(
        { operation: "collect", token_refresh: false },
        { credential: async () => credential, fetch, now, startRpc: refuse },
      );
      return { calls: calls.length, error: undefined };
    } catch (error) {
      return { calls: calls.length, error };
    }
  };
  for (const [label, credential, billing, fetches] of [
    ["expired", { ...grokCredential, expiresAt: 1_000_000 + 30_000 }, {}, 0],
    ["long expired", { ...grokCredential, expiresAt: 1 }, {}, 0],
    ["signed out", undefined, {}, 0],
    ["401", grokCredential, 401, 1],
    ["403", { ...grokCredential, expiresAt: 1_000_000 + 3_600_000 }, 403, 1],
  ]) {
    const result = await outcome(credential, billing);
    equal(result.error instanceof AuthenticationRequired, true, `${label} is authentication`);
    equal(
      result.error.message.includes("authentication required"),
      true,
      `${label} message classifies as authentication`,
    );
    equal(result.calls, fetches, `${label} makes no further request`);
  }
  const failed = await outcome(grokCredential, 503);
  equal(failed.error instanceof AuthenticationRequired, false, "503 is a fetch failure");
  equal(failed.calls, 1, "503 is not retried");
});

test("Grok declares refresh-free usage and reaches only its read endpoints", () => {
  equal(grokDeclaration, { schema: 1, token_refresh: "request" }, "declaration");
  for (const argv of [grokHost.usage.collector_argv, grokHost.usage.reset_argv]) {
    const net = argv.find((value) => value.startsWith("--allow-net="));
    equal(net, "--allow-net=grok.com:443,cli-chat-proxy.grok.com:443", "network grant");
    equal(
      argv.some((value) => value.startsWith("--allow-write")),
      false,
      "no write grant",
    );
  }
  const jwt = (claims) => `e30.${btoa(JSON.stringify(claims)).replace(/=+$/, "")}.c2ln`;
  equal(
    credentialFromJson({
      scope: {
        auth_mode: "oidc",
        key: jwt({ exp: 1 }),
        user_id: "user",
        expires_at: "2026-11-21T03:00:00Z",
      },
    }).expiresAt,
    Date.parse("2026-11-21T03:00:00Z"),
    "stored expiry",
  );
  equal(
    credentialFromJson({ scope: { auth_mode: "oidc", key: jwt({ exp: 1800 }), user_id: "u" } })
      .expiresAt,
    1_800_000,
    "token expiry",
  );
});
