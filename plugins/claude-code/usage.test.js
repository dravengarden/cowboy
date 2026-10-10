import { join } from "node:path";
import { tmpdir } from "node:os";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { childEnvironment, collect } from "./collector/index.js";
import {
  nativeJson,
  quotaView,
  REQUEST_ID,
  USAGE_ARGS,
} from "./collector/usage.js";

function equal(actual, expected) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `Expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

async function rejects(fn, message) {
  try {
    await fn();
  } catch (error) {
    equal(error.message, message);
    return;
  }
  throw new Error("Expected rejection");
}

const window = (utilization, resets_at = "2026-09-16T04:00:00Z") => ({
  utilization,
  resets_at,
});
const quota = (rate_limits) =>
  quotaView({ rate_limits_available: true, rate_limits });

test("native usage projects percentages, ISO resets, model windows and enabled extra usage", () => {
  const buckets = quota({
    five_hour: window(0),
    seven_day: window(23.5),
    seven_day_opus: window(55),
    seven_day_sonnet: null,
    seven_day_oauth_apps: window(null),
    model_scoped: [{ display_name: "Opus", ...window(60) }, {
      display_name: "Future",
      ...window(100),
    }],
    extra_usage: {
      is_enabled: true,
      utilization: 12,
      monthly_limit: 5000,
      used_credits: 600,
    },
  }).rate_limits.rateLimitsByLimitId;
  equal(buckets["claude-five_hour"].primary, {
    usedPercent: 0,
    windowDurationMins: 300,
    resetsAt: Date.parse("2026-09-16T04:00:00Z") / 1000,
  });
  equal(buckets["claude-seven_day"].primary.usedPercent, 23.5);
  equal(buckets["claude-seven_day_opus"], undefined);
  equal(buckets["claude-model-Opus"].primary.usedPercent, 60);
  equal(buckets["claude-model-Future"].primary.usedPercent, 100);
  equal(buckets["claude-extra-usage"], {
    limitName: "Extra usage",
    primary: { usedPercent: 12 },
  });
  equal(Object.keys(buckets).length, 5);
});

// DISABLE_TELEMETRY=1 closes the gate behind `model_scoped`, so a collector
// that keeps telemetry off only ever sees the unified `limits` array.
test("model windows survive a response without the legacy model_scoped projection", () => {
  const buckets = quota({
    five_hour: window(14),
    seven_day: window(47),
    limits: [
      { kind: "session", group: "session", percent: 14, scope: null },
      { kind: "weekly_all", group: "weekly", percent: 47, scope: null },
      {
        kind: "weekly_scoped",
        group: "weekly",
        percent: 10,
        resets_at: "2026-09-16T04:00:00Z",
        scope: { model: { id: null, display_name: "Fable" }, surface: null },
      },
      {
        kind: "weekly_scoped",
        group: "weekly",
        percent: 3,
        scope: { surface: { display_name: "Cowork" } },
      },
    ],
  }).rate_limits.rateLimitsByLimitId;
  equal(buckets["claude-model-Fable"], {
    limitName: "Fable",
    primary: {
      usedPercent: 10,
      windowDurationMins: 10080,
      resetsAt: Date.parse("2026-09-16T04:00:00Z") / 1000,
    },
  });
  equal(Object.keys(buckets).length, 3);
});

// Both shapes together must not double-count the same model.
test("a model reported by both projections yields one window", () => {
  const buckets = quota({
    seven_day: window(47),
    model_scoped: [{ display_name: "Fable", ...window(10) }],
    limits: [
      {
        kind: "weekly_scoped",
        percent: 10,
        resets_at: "2026-09-16T04:00:00Z",
        scope: { model: { display_name: "Fable" } },
      },
    ],
  }).rate_limits.rateLimitsByLimitId;
  equal(buckets["claude-model-Fable"].primary.usedPercent, 10);
  equal(Object.keys(buckets).length, 2);
});

test("unknown quota stays unknown and malformed responses are transient failures", async () => {
  equal(quotaView({ rate_limits_available: false, rate_limits: null }), {});
  equal(
    quota({
      five_hour: null,
      seven_day: window(null),
      extra_usage: { is_enabled: false },
    }),
    {
      rate_limits: { rateLimitsByLimitId: {} },
    },
  );
  equal(
    quota({ five_hour: window(1, "0") }).rate_limits
      .rateLimitsByLimitId["claude-five_hour"].primary,
    { usedPercent: 1, windowDurationMins: 300 },
  );
  for (
    const rate_limits of [
      null,
      {},
      { unknown_schema: [] },
      { five_hour: "25%" },
      { five_hour: window("25") },
      { five_hour: window(-1) },
      { five_hour: window(101) },
      { five_hour: window(NaN) },
      { model_scoped: {} },
      { limits: {} },
      { limits: ["weekly_scoped"] },
    ]
  ) {
    await rejects(
      () => quota(rate_limits),
      "Claude Code usage is temporarily unavailable.",
    );
  }
});

// Fake native processes exercise the same stdin/stdout/timeout path as the
// released CLI. No credentials, model requests, or network are involved.
async function fixture(source, run) {
  const dir = await mkdtemp(join(tmpdir(), "cowboy-claude-usage-"));
  const quote = (s) => `'${s.replaceAll("'", "'\\''")}'`;
  try {
    await writeFile(`${dir}/fixture.mjs`, source);
    const command = `${dir}/claude`;
    await writeFile(
      command,
      `#!/bin/sh\nexec ${quote(process.execPath)} run --no-env-file ${
        quote(`${dir}/fixture.mjs`)
      } "$@"\n`,
    );
    await chmod(command, 0o700);
    return await run({ command, env: {}, deadline: Date.now() + 3000 });
  } finally {
    await rm(dir, { recursive: true });
  }
}

const request = {
  type: "control_request",
  request_id: REQUEST_ID,
  request: { subtype: "get_usage", skip_behaviors: true },
};
const reply = (response) => ({
  type: "control_response",
  response: {
    subtype: "success",
    request_id: REQUEST_ID,
    response,
  },
});

test("collector queries account quota without a user prompt and stops a lingering CLI", async () => {
  await fixture(
    `
    if (process.argv.slice(2)[0] === "auth") {
      console.log(JSON.stringify({loggedIn:true, subscriptionType:"pro", email:"fixture@example.test"}));
    } else {
      const arg = (name) => process.argv.slice(2)[process.argv.slice(2).indexOf(name) + 1];
      if (!process.argv.slice(2).includes("--no-session-persistence") ||
          !process.argv.slice(2).includes("--strict-mcp-config") ||
          arg("--setting-sources") !== "" || arg("--tools") !== "" ||
          JSON.parse(arg("--settings")).disableAllHooks !== true ||
          Object.keys(JSON.parse(arg("--mcp-config")).mcpServers).length !== 0) {
        throw new Error("Usage query must disable project effects and session persistence");
      }
      const input = await Bun.stdin.text();
      if (input.trim() !== ${
      JSON.stringify(JSON.stringify(request))
    }) throw new Error("Unexpected prompt or command");
      console.log(JSON.stringify({type:"system", message:"ignored notification"}));
      console.log(JSON.stringify({type:"control_response", response:{request_id:"unrelated", subtype:"error"}}));
      const bytes = new TextEncoder().encode(${
      JSON.stringify(
        JSON.stringify(reply({
          subscription_type: "max",
          rate_limits_available: true,
          rate_limits: {
            five_hour: window(40),
            seven_day: window(70),
            model_scoped: [{ display_name: "模型", ...window(5) }],
          },
          session: { total_cost_usd: 0 },
          ignored_secret: "must-not-be-projected",
        })) + "\r\n",
      )
    });
      for (const byte of bytes) await new Promise((done) => process.stdout.write(new Uint8Array([byte]), done));
      await new Promise(() => setInterval(() => {}, 1000));
    }
  `,
    async ({ command, env }) => {
      const result = await collect({ command, env, timeoutMs: 3000 });
      equal(result.account.account.planType, "max");
      equal(
        result.rate_limits.rateLimitsByLimitId["claude-five_hour"].primary
          .usedPercent,
        40,
      );
      equal(
        result.rate_limits.rateLimitsByLimitId["claude-model-模型"].limitName,
        "模型",
      );
      equal(result.ignored_secret, undefined);
      equal(result.activity, undefined);
    },
  );
});

test("native stream bounds timeout, output, malformed JSON, EOF and wrong response ids", async () => {
  for (
    const [source, message] of [
      ["setInterval(() => {}, 1000);", "Claude Code usage timed out."],
      [
        'console.log("x".repeat(300000));',
        "Claude Code usage is temporarily unavailable.",
      ],
      [
        'console.log("not JSON secret-value");',
        "Claude Code usage is temporarily unavailable.",
      ],
      [
        'console.log(JSON.stringify({type:"control_response",response:{request_id:"wrong"}}));',
        "Claude Code usage is temporarily unavailable.",
      ],
      ["", "Claude Code usage is temporarily unavailable."],
    ]
  ) {
    await fixture(source, (execution) =>
      rejects(
        () =>
          nativeJson(USAGE_ARGS, request, {
            ...execution,
            deadline: Date.now() + 500,
          }),
        message,
      ));
  }
});

test("native control errors redact provider diagnostics and classify expired authentication", async () => {
  for (
    const [diagnostic, message] of [
      ["401 secret-token", "Claude Code usage authentication required."],
      [
        "unknown request get_usage secret-token",
        "Claude Code usage query is unsupported by this version.",
      ],
      ["429 secret-token", "Claude Code usage is temporarily unavailable."],
    ]
  ) {
    await fixture(
      `console.log(${
        JSON.stringify(JSON.stringify({
          type: "control_response",
          response: {
            subtype: "error",
            request_id: REQUEST_ID,
            error: diagnostic,
          },
        }))
      });`,
      (execution) =>
        rejects(() => nativeJson(USAGE_ARGS, request, execution), message),
    );
  }
});

test("signed-out account status prevents quota requests", async () => {
  await fixture(
    "console.log(JSON.stringify({loggedIn:false})); process.exit(1);",
    ({ command, env }) =>
      rejects(
        () => collect({ command, env }),
        "Claude Code usage authentication required.",
      ),
  );
});

test("collector child environment excludes inherited routing and credentials", () => {
  process.env["ANTHROPIC_COWBOY_USAGE_FIXTURE"] = "do-not-forward";
  try {
    const env = childEnvironment();
    equal(env.ANTHROPIC_COWBOY_USAGE_FIXTURE, undefined);
    equal(env.DISABLE_AUTOUPDATER, "1");
    equal(env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC, undefined);
    equal(env.DISABLE_TELEMETRY, "1");
    equal(env.DISABLE_ERROR_REPORTING, "1");
    equal(
      Object.keys(env).filter((key) =>
        /API_KEY|TOKEN|PROXY|BASE_URL/.test(key)
      ),
      [],
    );
  } finally {
    delete process.env["ANTHROPIC_COWBOY_USAGE_FIXTURE"];
  }
});

test("subscriber quota remains reachable when the parent disables nonessential traffic", async () => {
  const key = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";
  const previous = process.env[key];
  process.env[key] = "1";
  try {
    await fixture(
      `
      if (process.argv.slice(2)[0] === "auth") {
        console.log(JSON.stringify({loggedIn:true, subscriptionType:"max"}));
      } else {
        await Bun.stdin.text();
        const blocked = process.env[${JSON.stringify(key)}] === "1";
        console.log(JSON.stringify({type:"control_response", response:{
          subtype:"success", request_id:${
        JSON.stringify(REQUEST_ID)
      }, response:{
            rate_limits_available:true,
            rate_limits: blocked ? null : {five_hour:{utilization:4}}
          }
        }}));
      }
    `,
      async ({ command }) => {
        const result = await collect({ command });
        equal(
          result.rate_limits.rateLimitsByLimitId["claude-five_hour"].primary
            .usedPercent,
          4,
        );
      },
    );
  } finally {
    if (previous === undefined) delete process.env[key];
    else process.env[key] = previous;
  }
});
