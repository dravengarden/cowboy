import claudeCodeHost from "./claude-code/host.json" with { type: "json" };
import { accountView, KNOWN_PLANS } from "./claude-code/collector/index.js";
import { collectorTargets, group } from "./claude-deepseek/collector/index.js";
import {
  nearestCredit as nearestCodexCredit,
  readLines as codexLines,
} from "./codex/collector/index.js";
import {
  activePlan,
  credentialFromJson,
  encodeStringField,
  nearestCredit as nearestGrokCredit,
  readLines as grokLines,
} from "./grok/collector/index.js";

function equal(actual, expected, message) {
  const left = JSON.stringify(actual);
  const right = JSON.stringify(expected);
  if (left !== right) {
    throw new Error(`${message}: got ${left}, expected ${right}`);
  }
}

for (
  const [provider, readLines] of [["Codex", codexLines], ["Grok", grokLines]]
) {
  Deno.test(`${provider} collector reads fragmented UTF-8 JSON RPC without nonstandard stream globals`, async () => {
    const bytes = new TextEncoder().encode(
      '{"id":1,"result":"中文"}\r\n{"id":2}\n{"id":3}',
    );
    const stream = new ReadableStream({
      start(controller) {
        for (const byte of bytes) controller.enqueue(new Uint8Array([byte]));
        controller.close();
      },
    });
    const messages = [];
    for await (const line of readLines(stream)) messages.push(JSON.parse(line));
    equal(
      messages,
      [{ id: 1, result: "中文" }, { id: 2 }, { id: 3 }],
      "RPC messages",
    );
  });
}

Deno.test("Codex reset selection is deterministic and ignores unavailable credits", () => {
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

Deno.test("DeepSeek collector groups shared account fingerprints without duplicating lanes", () => {
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
    [{
      accountFingerprint: "same",
      agents: ["claude", "codex"],
      isAvailable: true,
      balanceInfos: [1],
    }],
    "grouped balance account",
  );
});

Deno.test("DeepSeek exact sidecar targets bind by lane id rather than URL order", () => {
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

Deno.test("Grok collector extracts OIDC credentials and highest active plan", () => {
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

Deno.test("Grok reset selection and protobuf string encoding stay deterministic", () => {
  equal(
    nearestGrokCredit([
      { id: "later", expiresAt: 20 },
      { id: "first", expiresAt: 10 },
    ]).id,
    "first",
    "nearest Grok credit",
  );
  equal(
    [...encodeStringField(10, "abc")],
    [82, 3, 97, 98, 99],
    "protobuf field",
  );
});

Deno.test("Anthropic collector projects only renderable account fields", () => {
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
    accountView({ subscriptionType: "some_future_tier" }).account.account
      .planType,
    "subscription",
    "unknown Anthropic plan",
  );
  equal(accountView({}), {}, "absent Anthropic account");
  equal(accountView({ subscriptionType: "   " }), {}, "blank Anthropic plan");
});

Deno.test("Anthropic exposes no reset capability to collect", () => {
  const usage = claudeCodeHost.usage;
  equal(usage.reset, undefined, "Anthropic reset verb");
  equal(usage.reset_argv, undefined, "Anthropic reset argv");
  equal(usage.order, 0, "Anthropic card order");
  // Claude Code has no reset-credit concept, so the collector must refuse any
  // operation other than the read-only collect.
  equal(
    KNOWN_PLANS.has("max") && KNOWN_PLANS.has("pro"),
    true,
    "known Anthropic plans",
  );
});
