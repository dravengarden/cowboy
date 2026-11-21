import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AGY_USAGE_ARGUMENTS,
  accessToken,
  envelope,
  quotaDocument,
  refusedEligibility,
  runAgy,
  usageReport,
} from "./usage.mjs";

// `agy` 1.5.9 print mode on OVH for a paid Google AI Pro account whose free
// tier is `UNSUPPORTED_LOCATION` (2026-11-20).
const REFUSED = JSON.stringify({
  conversation_id: "",
  status: "ERROR",
  response: "",
  error:
    "Eligibility check failed: Your current account is not eligible for Antigravity, because it is not currently available in your location.",
  duration_seconds: 0,
  num_turns: 0,
  finished_reason: "error",
});
// Cloud Code `retrieveUserQuotaSummary` for the same account, trimmed.
const SUMMARY = {
  groups: [
    {
      buckets: [
        {
          bucketId: "3p-weekly",
          displayName: "Weekly Limit Remaining",
          window: "weekly",
          resetTime: "2026-11-27T22:13:04Z",
          remainingFraction: 1,
        },
        {
          bucketId: "3p-5h",
          displayName: "Five Hour Limit Remaining",
          window: "5h",
          resetTime: "2026-11-21T03:13:04Z",
        },
      ],
      displayName: "Claude and GPT models",
      description: "Models within this group: Claude Opus, Claude Sonnet, GPT-OSS",
    },
  ],
  description: "Within each group, models share a weekly limit and a 5-hour limit.",
};

function home(t, token = { token: { access_token: "ya29.a0-test-access-token" } }) {
  const base = mkdtempSync(join(tmpdir(), "agy-usage-"));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  mkdirSync(join(base, ".gemini/antigravity-cli"), { recursive: true });
  if (token !== null) {
    writeFileSync(
      join(base, ".gemini/antigravity-cli/antigravity-oauth-token"),
      typeof token === "string" ? token : JSON.stringify(token),
    );
  }
  return base;
}

const native = (code, text) => async () => ({ code, output: Buffer.from(`${text}\n`) });

test("print mode's eligibility refusal is recognised, nothing else", () => {
  assert.equal(refusedEligibility(1, envelope(Buffer.from(`noise\n${REFUSED}\n`))), true);
  assert.equal(refusedEligibility(0, envelope(Buffer.from(REFUSED))), true);
  assert.equal(refusedEligibility(0, { status: "SUCCESS" }), false);
  assert.equal(refusedEligibility(1, { status: "ERROR", error: "Authentication required" }), false);
  assert.equal(
    refusedEligibility(1, { status: "ERROR", error: "User location is not supported" }),
    false,
  );
  assert.equal(refusedEligibility(1, undefined), false);
});

test("the summary becomes print mode's quota document", () => {
  assert.deepEqual(quotaDocument(SUMMARY), {
    groups: [
      {
        name: "Claude and GPT models",
        buckets: [
          {
            name: "Weekly Limit Remaining",
            id: "3p-weekly",
            window: "weekly",
            remaining_fraction: 1,
            reset_time: "2026-11-27T22:13:04Z",
          },
          // Proto3 omits a zero fraction: a reset with no fraction is spent.
          {
            name: "Five Hour Limit Remaining",
            id: "3p-5h",
            window: "5h",
            remaining_fraction: 0,
            reset_time: "2026-11-21T03:13:04Z",
          },
        ],
      },
    ],
  });
  assert.equal(quotaDocument({}), undefined);
  assert.equal(quotaDocument({ groups: [{ buckets: [] }] }), undefined);
  assert.equal(quotaDocument({ groups: [{ displayName: "g", buckets: [{}] }] }), undefined);
});

test("a native report passes through unchanged", async (t) => {
  let asked = false;
  const quota = async () => {
    asked = true;
  };
  const ok = '{"status":"SUCCESS","num_turns":0,"command":{"name":"/usage","data":{"groups":[]}}}';
  const report = await usageReport({ agy: "agy", home: home(t), run: native(0, ok), quota });
  assert.equal(report.code, 0);
  assert.equal(report.output.toString(), `${ok}\n`);
  const auth = await usageReport({
    agy: "agy",
    home: home(t),
    run: native(1, '{"status":"ERROR","error":"Authentication required"}'),
    quota,
  });
  assert.equal(auth.code, 1);
  assert.equal(asked, false);
});

test("an eligibility refusal is answered from Cloud Code with agy's token", async (t) => {
  let sent;
  const report = await usageReport({
    agy: "agy",
    home: home(t),
    run: native(1, REFUSED),
    quota: async (token) => {
      sent = token;
      return SUMMARY;
    },
  });
  assert.equal(sent, "ya29.a0-test-access-token");
  assert.equal(report.code, 0);
  const parsed = JSON.parse(report.output.toString());
  assert.equal(parsed.status, "SUCCESS");
  assert.equal(parsed.num_turns, 0);
  assert.equal(parsed.command.name, "/usage");
  assert.equal(parsed.command.data.groups[0].name, "Claude and GPT models");
  assert.ok(!report.output.toString().includes("ya29"));
});

test("without a token or a summary agy's own refusal stands", async (t) => {
  const refused = native(1, REFUSED);
  for (const [token, quota] of [
    [null, async () => SUMMARY],
    ["not json", async () => SUMMARY],
    [{ token: { access_token: "short" } }, async () => SUMMARY],
    [{ token: { access_token: "ya29.a0-test-access-token" } }, async () => undefined],
    [{ token: { access_token: "ya29.a0-test-access-token" } }, async () => ({ error: {} })],
  ]) {
    const report = await usageReport({ agy: "agy", home: home(t, token), run: refused, quota });
    assert.equal(report.code, 1);
    assert.equal(report.output.toString(), `${REFUSED}\n`);
  }
  assert.equal(await accessToken("/nonexistent"), undefined);
});

test("print mode runs with the fixed arguments and bounded output", async (t) => {
  const base = home(t);
  const agy = join(base, "agy");
  writeFileSync(agy, `#!/bin/sh\nprintf '%s\\n' "$*"\nexit 3\n`, { mode: 0o755 });
  const { code, output } = await runAgy(agy);
  assert.equal(code, 3);
  assert.equal(output.toString(), `${AGY_USAGE_ARGUMENTS.join(" ")}\n`);
  const missing = await runAgy(join(base, "missing"));
  assert.equal(missing.code, null);
  const slow = join(base, "slow");
  writeFileSync(slow, "#!/bin/sh\nexec sleep 30\n", { mode: 0o755 });
  assert.equal((await runAgy(slow, { timeoutMs: 100 })).code, null);
});
