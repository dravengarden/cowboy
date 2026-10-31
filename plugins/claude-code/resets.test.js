import { test } from "bun:test";
import { assertEquals, assertRejects } from "@std/assert";
import {
  consumeReset,
  readCredential,
  readPlanTier,
  resetClient,
  resetCredits,
  ResetFailure,
} from "./collector/resets.js";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import provider from "./provider.json" with { type: "json" };

const grant = (changes = {}) => ({
  id: "launch-reset",
  resets_left: 2,
  ends_at: "2099-01-01T00:00:00Z",
  clears: ["five_hour", "seven_day"],
  usable_now: true,
  paused: false,
  ...changes,
});
const block = (changes = {}) => ({
  eligible: true,
  next_grant_id: "launch-reset",
  grants: [grant()],
  ...changes,
});

test("Claude reset projection distinguishes unknown, held and server-selected usable grants", () => {
  assertEquals(resetCredits(null), undefined);
  assertEquals(resetCredits({ eligible: false }), {
    availableCount: 0,
    credits: [],
  });
  const result = resetCredits(
    block({
      grants: [
        grant(),
        grant({ id: "another", resets_left: 1, ends_at: null }),
      ],
    }),
  );
  assertEquals(result.availableCount, 2);
  assertEquals(result.credits.map((c) => [c.id, c.status]), [[
    "launch-reset:2",
    "available",
  ]]);
  assertEquals(result.credits[0].title, "5h + Weekly reset (2 left)");
  for (
    const change of [{ paused: true }, { usable_now: false }, {
      ends_at: "2000-01-01T00:00:00Z",
    }]
  ) {
    assertEquals(
      resetCredits(block({ grants: [grant(change)] })).credits.some((c) =>
        c.status === "available"
      ),
      false,
    );
  }
  assertEquals(
    resetCredits(block({ cooldown_until: "2099-01-01T00:00:00Z" })).credits,
    [],
  );
});

function clientFixture(
  { status = block(), reply = { result: "reset" }, error } = {},
) {
  const calls = [];
  return {
    calls,
    client: async (path, body) => {
      calls.push({ path, body });
      if (path.startsWith("/api/oauth/usage")) return { cedar_ember: status };
      if (path === "/api/oauth/profile") {
        return {
          organization: { uuid: "12345678-1234-1234-1234-123456789abc" },
        };
      }
      if (error) throw error;
      return reply;
    },
  };
}
const request = {
  idempotency_key: "cowboy-reset-fixture-1",
  expected_credit_id: "launch-reset:2",
};

test("slow preflight refuses the irreversible POST before its response budget is exhausted", async () => {
  const { client, calls } = clientFixture();
  const error = await assertRejects(
    () => consumeReset(client, request, Date.now() + 1000),
    ResetFailure,
  );
  assertEquals(error.callMayHaveReachedProvider, false);
  assertEquals(calls.filter((call) => call.body).length, 0);
});

test("Claude reset posts exactly the server-selected grant with caller idempotency", async () => {
  const { client, calls } = clientFixture();
  assertEquals(await consumeReset(client, request), {
    outcome: "reset",
    credit_id: "launch-reset:2",
  });
  assertEquals(calls.at(-1).body, {
    program: "cedar_ember",
    grant_id: "launch-reset",
    request_id: request.idempotency_key,
  });
  assertEquals(calls.filter((c) => c.body).length, 1);
});

test("stale confirmation refuses a second use of the same multi-use grant", async () => {
  const { client, calls } = clientFixture({
    status: block({ grants: [grant({ resets_left: 1 })] }),
  });
  const error = await assertRejects(
    () => consumeReset(client, request),
    ResetFailure,
  );
  assertEquals(error.callMayHaveReachedProvider, false);
  assertEquals(calls.filter((c) => c.body).length, 0);
});

test("unavailable and malformed preflight never sends a reset", async () => {
  for (
    const status of [
      null,
      block({ eligible: false }),
      block({ grants: [grant({ usable_now: false })] }),
      block({ grants: [grant({ id: "../escape" })] }),
      block({ grants: [grant(), grant()] }),
    ]
  ) {
    const { client, calls } = clientFixture({ status });
    const error = await assertRejects(
      () => consumeReset(client, request),
      ResetFailure,
    );
    assertEquals(error.callMayHaveReachedProvider, false);
    assertEquals(calls.filter((c) => c.body).length, 0);
  }
});

test("lost or unrecognized reset replies remain unknown without automatic retry or secret diagnostics", async () => {
  for (
    const fixture of [
      { error: new Error("secret-token") },
      { reply: {} },
      {
        reply: { result: "future" },
      },
      { reply: { result: "unavailable" } },
      {
        reply: { result: "already_used", reason: "stamp_indeterminate" },
      },
      { reply: { result: "unavailable", reason: "reset_unconfirmed" } },
    ]
  ) {
    const { client, calls } = clientFixture(fixture);
    const error = await assertRejects(
      () => consumeReset(client, request),
      ResetFailure,
    );
    assertEquals(error.callMayHaveReachedProvider, true);
    assertEquals(error.message.includes("secret-token"), false);
    assertEquals(calls.filter((c) => c.body).length, 1);
  }
  for (
    const result of ["not_limited", "cooldown", "ineligible"]
  ) {
    const { client } = clientFixture({ reply: { result } });
    const error = await assertRejects(
      () => consumeReset(client, request),
      ResetFailure,
    );
    assertEquals(error.callMayHaveReachedProvider, false);
  }
  const { client } = clientFixture({ reply: { result: "already_used" } });
  assertEquals((await consumeReset(client, request)).outcome, "already_used");
});

test("HTTP client pins origin, refuses redirects, bounds output and times out", async () => {
  const calls = [];
  const client = resetClient(
    "fixture-token",
    Date.now() + 1000,
    async (url, options) => {
      calls.push({ url, options });
      return Response.json({ ok: true });
    },
  );
  await client("/api/oauth/usage");
  assertEquals(calls[0].url, "https://api.anthropic.com/api/oauth/usage");
  assertEquals(calls[0].options.redirect, "error");
  assertEquals(calls[0].options.headers.Authorization, "Bearer fixture-token");
  const native = provider.runtime.dependencies.find((dependency) =>
    dependency.id === "anthropic-claude-code"
  );
  assertEquals(
    calls[0].options.headers["User-Agent"],
    `claude-cli/${native.version} (external, cli)`,
  );
  await assertRejects(() =>
    resetClient("fixture", Date.now() - 1, () => {
      throw Error("must not fetch");
    })("/api/oauth/usage"), ResetFailure);
  await assertRejects(
    () =>
      resetClient(
        "fixture",
        Date.now() + 1000,
        async () => new Response("x".repeat(256 * 1024 + 1)),
      )("/api/oauth/usage"),
    ResetFailure,
  );
  await assertRejects(
    () =>
      resetClient(
        "fixture",
        Date.now() + 1000,
        async () => new Response("secret", { status: 401 }),
      )("/api/oauth/usage"),
    ResetFailure,
    "authentication required",
  );
});

test("credential read uses only the explicitly projected shared directory and never writes", async () => {
  const directory = await mkdtemp(join(tmpdir(), "claude-reset-test-"));
  try {
    const path = join(directory, ".credentials.json");
    const data = {
      claudeAiOauth: {
        accessToken: "fixture-token",
        refreshToken: "never-used",
        scopes: ["user:profile"],
        expiresAt: Date.now() + 60000,
      },
    };
    await writeFile(path, JSON.stringify(data));
    assertEquals(await readCredential(directory), "fixture-token");
    assertEquals(await Bun.file(path).json(), data);
    await assertRejects(() => readCredential(undefined), ResetFailure);
    data.claudeAiOauth.expiresAt = 1;
    await writeFile(path, JSON.stringify(data));
    await assertRejects(
      () => readCredential(directory),
      ResetFailure,
      "authentication required",
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("plan tier read is best-effort and accepts only a plain identifier", async () => {
  const directory = await mkdtemp(join(tmpdir(), "claude-tier-test-"));
  try {
    const path = join(directory, ".credentials.json");
    assertEquals(await readPlanTier(directory), undefined);
    assertEquals(await readPlanTier(undefined), undefined);
    await writeFile(
      path,
      JSON.stringify({
        claudeAiOauth: { rateLimitTier: "default_claude_max_20x" },
      }),
    );
    assertEquals(await readPlanTier(directory), "default_claude_max_20x");
    await writeFile(
      path,
      JSON.stringify({
        claudeAiOauth: { rateLimitTier: "Bearer secret value" },
      }),
    );
    assertEquals(await readPlanTier(directory), undefined);
    await writeFile(path, "not json");
    assertEquals(await readPlanTier(directory), undefined);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
