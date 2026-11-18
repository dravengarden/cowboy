import assert from "node:assert/strict";
import { appendFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { AgyLog, classify, parseRecords, refusalClass } from "./evidence.mjs";

// Real `agy` 1.5.9 log lines captured on Hawk with the dedicated test
// account (the verification URL's token is redacted).
const ok = readFileSync(new URL("./testdata/turn-ok.txt", import.meta.url), "utf8");
const verification = readFileSync(
  new URL("./testdata/turn-verification.txt", import.meta.url),
  "utf8",
);
const OK_SESSION = "41fea08c-692b-4737-acd5-099de1e2f2cd";
const REFUSED_SESSION = "2d67a645-da8f-46e6-b9fb-6e2f9fd20870";

test("a successful multi-step turn is ok only with its barrier", () => {
  const records = parseRecords(ok, OK_SESSION);
  assert.equal(records.filter((record) => record.kind === "success").length, 5);
  assert.deepEqual(classify(records), {
    class: "ok",
    successes: 5,
    barrier: true,
    transient: 0,
    refusals: [],
  });
  const withoutBarrier = ok.split("\n").slice(0, -2).join("\n");
  assert.equal(classify(parseRecords(withoutBarrier, OK_SESSION)).class, "unknown_refusal");
  // Another session's barrier is not this turn's.
  assert.equal(classify(parseRecords(ok, REFUSED_SESSION)).class, "unknown_refusal");
  assert.equal(classify([]).class, "unknown_refusal");
  // A cancelled turn is ok only after its barrier: without it, its backend
  // steps may still run or refuse.
  assert.equal(classify([], { stopReason: "cancelled" }).class, "unknown_refusal");
  assert.equal(classify([{ kind: "barrier" }], { stopReason: "cancelled" }).class, "ok");
});

test("a retried transient failure is not a refusal; a retried 429 is", () => {
  const retry = readFileSync(new URL("./testdata/turn-retry.txt", import.meta.url), "utf8");
  const session = "54f1aff0-8f8e-4c96-acf4-abc76bd68e92";
  const settled = classify(parseRecords(retry, session));
  assert.equal(settled.class, "ok");
  assert.equal(settled.transient, 1);
  assert.equal(settled.successes, 4);
  const limited = retry.replace("UNAVAILABLE (code 503)", "RESOURCE_EXHAUSTED (code 429)");
  assert.equal(classify(parseRecords(limited, session)).class, "rate_limited");
  // A retried code that is not known to be transient is never passed as ok.
  const unexpected = retry.replace("UNAVAILABLE (code 503)", "FAILED_PRECONDITION (code 400)");
  const parked = classify(parseRecords(unexpected, session));
  assert.equal(parked.class, "unknown_refusal");
  assert.equal(parked.transient, 0);
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
  assert.equal(settled.barrier, true);
  assert.deepEqual(settled.refusals[0], {
    status: "PERMISSION_DENIED",
    code: 403,
    reasons: ["VALIDATION_REQUIRED"],
  });
  // Without the structured error the same 403 is an unknown refusal, and a
  // refusal wins over success records.
  assert.equal(classify(turn).class, "unknown_refusal");
  const mixed = [...parseRecords(ok, OK_SESSION), ...turn];
  assert.equal(classify(mixed, { prior }).class, "verification_required");
  assert.equal(classify(turn, { prior, stopReason: "cancelled" }).class, "verification_required");
});

test("refusal classes and precedence", () => {
  assert.equal(refusalClass({ reasons: ["UNSUPPORTED_LOCATION"] }), "location_unsupported");
  assert.equal(refusalClass({ status: "RESOURCE_EXHAUSTED", code: 429 }), "rate_limited");
  assert.equal(refusalClass({ code: 429 }), "rate_limited");
  assert.equal(refusalClass({ status: "PERMISSION_DENIED", code: 403 }), "unknown_refusal");
  const records = [
    { kind: "barrier" },
    { kind: "request_failure", status: "RESOURCE_EXHAUSTED", code: 429, reasons: [] },
    {
      kind: "request_failure",
      status: "PERMISSION_DENIED",
      code: 403,
      reasons: ["UNSUPPORTED_LOCATION"],
    },
  ];
  assert.equal(classify(records).class, "location_unsupported");
  assert.equal(classify(records.slice(0, 2)).class, "rate_limited");
  // Model text that merely mentions a status never matches.
  const text =
    'I1117 05:11:38.187128     363 server.go:1] agent said "generating and executing: PERMISSION_DENIED (code 403)"';
  assert.deepEqual(parseRecords(text), []);
});

test("the log reader settles a turn after its barrier with a per-turn cursor", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "agy-log-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "acp-server-20261118_000000-4242.log");
  const lines = verification.split("\n");
  const start = lines.findIndex((line) => line.includes("ACP Request received: session/prompt"));
  writeFileSync(path, `${lines.slice(0, start).join("\n")}\n`);
  writeFileSync(join(directory, "acp-server-20261118_000000-99.log"), ok);
  const log = new AgyLog(directory, 4242, { pollMs: 10, barrierTimeoutMs: 2000 });
  const cursor = await log.mark();
  setTimeout(() => appendFileSync(path, `${lines.slice(start, start + 15).join("\n")}\n`), 50);
  const settled = await log.settle(cursor, REFUSED_SESSION, "end_turn");
  assert.equal(settled.class, "verification_required");
  const next = await log.mark();
  appendFileSync(path, ok.replaceAll(OK_SESSION, REFUSED_SESSION));
  // Earlier refusals only explain later step failures; they are not records
  // of the next turn, which settles on its own window.
  assert.equal((await log.settle(next, REFUSED_SESSION, "end_turn")).class, "ok");
  const quick = new AgyLog(directory, 4242, { pollMs: 10, barrierTimeoutMs: 50 });
  const end = await quick.mark();
  assert.equal((await quick.settle(end, REFUSED_SESSION, "end_turn")).class, "unknown_refusal");
  const absent = new AgyLog(directory, 7, { pollMs: 10, barrierTimeoutMs: 50 });
  assert.equal(await absent.mark(), undefined);
  assert.equal(
    (await absent.settle(undefined, REFUSED_SESSION, "end_turn")).class,
    "unknown_refusal",
  );
});
