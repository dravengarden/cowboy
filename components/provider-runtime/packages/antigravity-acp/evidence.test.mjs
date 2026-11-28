import assert from "node:assert/strict";
import { appendFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AgyLog,
  BARRIER_METHOD,
  classify,
  parseRecords,
  parseStepTimings,
  refusalClass,
  stepTimingsFor,
} from "./evidence.mjs";

const ok = readFileSync(new URL("./testdata/turn-ok.txt", import.meta.url), "utf8");
const verification = readFileSync(
  new URL("./testdata/turn-verification.txt", import.meta.url),
  "utf8",
);
const OK_SESSION = "41fea08c-692b-4737-acd5-099de1e2f2cd";
const REFUSED_SESSION = "2d67a645-da8f-46e6-b9fb-6e2f9fd20870";

test("a turn without explicit Google refusals settles ok without requiring a barrier", () => {
  const records = parseRecords(ok, OK_SESSION);
  assert.equal(records.filter((record) => record.kind === "success").length, 5);
  assert.deepEqual(classify(records), {
    class: "ok",
    successes: 5,
    barrier: true,
    prompts: 1,
    dispatches: 1,
    transient: 0,
    refusals: [],
  });
  const withoutBarrier = ok.split("\n").slice(0, -2).join("\n");
  assert.equal(classify(parseRecords(withoutBarrier, OK_SESSION)).class, "ok");
  assert.equal(classify([]).class, "ok");
  assert.equal(classify([], { stopReason: "cancelled" }).class, "ok");
});

test("a retried transient or unrecognized failure is ok; a retried 429 is rate_limited", () => {
  const retry = readFileSync(new URL("./testdata/turn-retry.txt", import.meta.url), "utf8");
  const session = "54f1aff0-8f8e-4c96-acf4-abc76bd68e92";
  const settled = classify(parseRecords(retry, session));
  assert.equal(settled.class, "ok");
  assert.equal(settled.transient, 1);
  assert.equal(settled.successes, 4);
  const limited = retry.replace("UNAVAILABLE (code 503)", "RESOURCE_EXHAUSTED (code 429)");
  assert.equal(classify(parseRecords(limited, session)).class, "rate_limited");
  const unexpected = retry.replace("UNAVAILABLE (code 503)", "FAILED_PRECONDITION (code 400)");
  assert.equal(classify(parseRecords(unexpected, session)).class, "ok");
});

test("VALIDATION_REQUIRED is attributed from the process's structured error", () => {
  const lines = verification.split("\n");
  const start = lines.findIndex((line) => line.includes("ACP Request received: session/prompt"));
  const prior = parseRecords(lines.slice(0, start).join("\n"));
  assert.deepEqual(prior, [
    {
      kind: "request_failure",
      status: "PERMISSION_DENIED",
      code: 403,
      reasons: ["VALIDATION_REQUIRED"],
    },
  ]);
  const turn = parseRecords(lines.slice(start, start + 15).join("\n"), REFUSED_SESSION);
  assert.equal(turn.filter((record) => record.kind === "step_failure").length, 2);
  const settled = classify(turn, { prior });
  assert.equal(settled.class, "verification_required");
  assert.deepEqual(settled.refusals[0], {
    status: "PERMISSION_DENIED",
    code: 403,
    reasons: ["VALIDATION_REQUIRED"],
  });
  // Without structured ErrorInfo reasons, a bare 403 does not synthesize unknown_refusal.
  assert.equal(classify(turn).class, "ok");
  const mixed = [...parseRecords(ok, OK_SESSION), ...turn];
  assert.equal(classify(mixed, { prior }).class, "verification_required");
  assert.equal(classify(turn, { prior, stopReason: "cancelled" }).class, "verification_required");
});

test("refusal classes and precedence", () => {
  assert.equal(refusalClass({ reasons: ["UNSUPPORTED_LOCATION"] }), "location_unsupported");
  assert.equal(refusalClass({ status: "RESOURCE_EXHAUSTED", code: 429 }), "rate_limited");
  assert.equal(refusalClass({ code: 429 }), "rate_limited");
  assert.equal(refusalClass({ status: "PERMISSION_DENIED", code: 403 }), null);
  const records = [
    { kind: "request_failure", status: "RESOURCE_EXHAUSTED", code: 429, reasons: [] },
    {
      kind: "request_failure",
      status: "PERMISSION_DENIED",
      code: 403,
      reasons: ["UNSUPPORTED_LOCATION"],
    },
  ];
  assert.equal(classify(records).class, "location_unsupported");
  assert.equal(classify(records.slice(0, 1)).class, "rate_limited");
});

test("AgyLog settles immediately without barrier RPC polling and attaches step timings", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "cowboy-agy-ev-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "acp-server-20261122-5151.log");
  writeFileSync(path, "");
  const log = new AgyLog(directory, 5151);

  // Turn 1: ok with step timings
  const c1 = await log.mark();
  appendFileSync(path, `${ok}\n`);
  const r1 = await log.settle(c1, OK_SESSION, "end_turn");
  assert.equal(r1.class, "ok");
  assert.equal(r1.successes, 5);
  const timings = stepTimingsFor(r1);
  assert.equal(timings?.barrier_matched, true);
  assert.equal(timings?.backend_steps, 5);

  // Turn 2: explicit verification refusal attributed via prior structured error
  const lines = verification.split("\n");
  const start = lines.findIndex((line) => line.includes("ACP Request received: session/prompt"));
  appendFileSync(path, `${lines.slice(0, start).join("\n")}\n`);
  const c2 = await log.mark();
  appendFileSync(path, `${lines.slice(start, start + 15).join("\n")}\n`);
  const r2 = await log.settle(c2, REFUSED_SESSION, "end_turn");
  assert.equal(r2.class, "verification_required");

  // Missing log or undefined cursor cleanly settles ok immediately
  const missing = new AgyLog(directory, 99999);
  assert.equal(await missing.mark(), undefined);
  assert.equal((await missing.settle(undefined, "s", "end_turn")).class, "ok");
});

test("prompt receipts and dispatches are counted cleanly", () => {
  const glog = "I1124 23:17:44.130546  176836 ";
  const receipt = `${glog}server.go:213] ACP Request received: session/prompt`;
  const send = `${glog}server.go:1883] Sending user message to conversation 21e17488-d900-4ac3-954e-31c2cec5f86b (items=2, media=3)`;
  const answer = `${glog}prompt.go:358] Responding to pending ask_question for session 21e17488-d900-4ac3-954e-31c2cec5f86b step 896: "yes"`;
  const records = parseRecords([receipt, send, answer].join("\n"));
  assert.deepEqual(
    records.map((record) => record.kind),
    ["prompt", "dispatch", "dispatch"],
  );
  assert.equal(classify(records).dispatches, 2);
  assert.equal(classify(records).prompts, 1);
  const refused = parseRecords([receipt].join("\n"));
  assert.deepEqual(classify(refused), {
    class: "ok",
    successes: 0,
    barrier: false,
    prompts: 1,
    dispatches: 0,
    transient: 0,
    refusals: [],
  });
});

test("parseStepTimings extracts content-free numeric step timings with or without a barrier", () => {
  const timings = parseStepTimings(ok);
  assert.deepEqual(timings, {
    schema: 1,
    barrier_matched: true,
    backend_steps: 5,
    transient_retries: 0,
    first_step_offset_ms: 780,
    step_span_ms: 19119,
    max_inter_step_ms: 8460,
  });
  const retry = readFileSync(new URL("./testdata/turn-retry.txt", import.meta.url), "utf8");
  const retryTimings = parseStepTimings(retry);
  assert.equal(retryTimings.backend_steps, 4);
  assert.equal(retryTimings.transient_retries, 1);
  assert.ok(retryTimings.step_span_ms > 0);
  const midnight = [
    "I1127 23:59:59.500000 1 server.go:213] ACP Request received: session/prompt",
    "I1128 00:00:00.250000 1 http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0x1 ResponseID: R1",
    "I1128 00:00:01.750000 1 http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0x2 ResponseID: R2",
    `I1128 00:00:02.000000 1 server.go:213] ACP Request received: ${BARRIER_METHOD}/mid`,
  ].join("\n");
  assert.deepEqual(parseStepTimings(midnight, undefined, "mid"), {
    schema: 1,
    barrier_matched: true,
    backend_steps: 2,
    transient_retries: 0,
    first_step_offset_ms: 750,
    step_span_ms: 1500,
    max_inter_step_ms: 1500,
  });
  assert.equal(parseStepTimings("", undefined, "missing"), undefined);
});
