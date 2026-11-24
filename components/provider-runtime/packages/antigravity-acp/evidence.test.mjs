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
  refusalClass,
  requestBarrierPattern,
} from "./evidence.mjs";

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
    prompts: 1,
    dispatches: 1,
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

// What `agy` logged reading an unknown extension request
// (`_cowboy/evidence_barrier/n-3`), sandboxed on Hawk without an account or
// network: 1.5.9 from `acp/server.go`, 1.5.14 from `acp/server/server.go`.
const BARRIER_LINES = {
  "1.5.9":
    "I1122 04:42:16.136600      86 server.go:988] ACP Request received: _cowboy/evidence_barrier/n-3",
  "1.5.14":
    "I1122 04:42:24.197416      70 server.go:213] ACP Request received: _cowboy/evidence_barrier/n-3",
};

test("the request barrier matches what agy 1.5.9 and 1.5.14 log, and only its nonce", () => {
  for (const line of Object.values(BARRIER_LINES)) {
    assert.match(line, requestBarrierPattern("n-3"));
    assert.doesNotMatch(line, requestBarrierPattern("n-4"));
    assert.doesNotMatch(line, requestBarrierPattern("n"));
  }
  assert.equal(BARRIER_METHOD, "_cowboy/evidence_barrier");
  // The same words in anything but agy's own request line never match.
  assert.deepEqual(
    parseRecords(
      'I1122 04:42:24.197416      70 server.go:1] said "ACP Request received: _cowboy/evidence_barrier/n-3"',
      undefined,
      "n-3",
    ),
    [],
  );
});

test("a 1.5.14 turn, whose stream outlives it, settles on its request barrier alone", () => {
  // The ok turn as agy 1.5.10+ logs it: no stream barrier after its last step.
  const lines = ok.split("\n").filter((line) => !line.includes("Stream goroutine exited"));
  const turn = [...lines, BARRIER_LINES["1.5.14"]].join("\n");
  assert.deepEqual(classify(parseRecords(turn, OK_SESSION, "n-3")), {
    class: "ok",
    successes: 5,
    barrier: true,
    prompts: 1,
    dispatches: 1,
    transient: 0,
    refusals: [],
  });
  // Another turn's barrier request is not this one's.
  assert.equal(classify(parseRecords(turn, OK_SESSION, "n-4")).class, "unknown_refusal");
  // With a barrier request, the 1.5.9 stream barrier no longer ends a turn.
  assert.equal(classify(parseRecords(ok, OK_SESSION, "n-3")).class, "unknown_refusal");
  // A refusal before the barrier still wins.
  const refused = verification.split("\n").filter((line) => !line.includes("Stream goroutine"));
  const settled = classify(
    parseRecords([...refused, BARRIER_LINES["1.5.9"]].join("\n"), REFUSED_SESSION, "n-3"),
    { prior: parseRecords(verification) },
  );
  assert.equal(settled.class, "verification_required");
  assert.equal(settled.barrier, true);
});

test("the log reader sends one barrier request per settle and waits for its line", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "agy-log-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "acp-server-20261122_000000-5151.log");
  writeFileSync(path, "I1122 00:00:00.000000     1 serve.go:30] Starting ACP server\n");
  const sent = [];
  // `agy`: it logs the request as it reads it, a little later.
  const agy = (message) => {
    sent.push(message);
    setTimeout(
      () =>
        appendFileSync(
          path,
          `I1122 00:00:01.000000     70 server.go:213] ACP Request received: ${message.method}\n`,
        ),
      30,
    );
  };
  const log = new AgyLog(directory, 5151, { pollMs: 10, barrierTimeoutMs: 2000, request: agy });
  const lines = ok.split("\n").filter((line) => !line.includes("Stream goroutine exited"));
  const cursor = await log.mark();
  appendFileSync(path, `${lines.join("\n")}\n`);
  const settled = await log.settle(cursor, OK_SESSION, "end_turn");
  assert.equal(settled.class, "ok");
  assert.equal(settled.barrier, true);
  assert.equal(sent.length, 1);
  const [request] = sent;
  const nonce = request.method.slice(`${BARRIER_METHOD}/`.length);
  assert.match(nonce, /^[0-9a-f-]{36}$/);
  assert.deepEqual(request, {
    jsonrpc: "2.0",
    id: `cowboy-evidence-${nonce}`,
    method: `${BARRIER_METHOD}/${nonce}`,
    params: {},
  });
  // A barrier `agy` never reads (the proxy not attached, a closed pipe) ends
  // the wait at its deadline: neither the 1.5.9 stream barrier nor another
  // turn's barrier request (a subagent's, settling alongside) is this one's.
  const silent = new AgyLog(directory, 5151, {
    pollMs: 10,
    barrierTimeoutMs: 50,
    request: (message) => sent.push(message),
  });
  const start = await silent.mark();
  appendFileSync(
    path,
    `${ok}\nI1122 00:00:02.000000     71 server.go:213] ACP Request received: ${request.method}\n`,
  );
  const unread = await silent.settle(start, OK_SESSION, "end_turn");
  assert.equal(unread.class, "unknown_refusal");
  assert.equal(unread.barrier, false);
  assert.equal(unread.successes, 5);
  assert.equal(sent.length, 2);
  assert.notEqual(sent[1].method, request.method);
  // A request that cannot be sent settles at once.
  const broken = new AgyLog(directory, 5151, {
    request: () => {
      throw new Error("EPIPE");
    },
  });
  assert.equal(
    (await broken.settle(await broken.mark(), OK_SESSION, "end_turn")).class,
    "unknown_refusal",
  );
});

test("prompt receipts and dispatches count only through the barrier", () => {
  const glog = "I1124 23:17:44.130546  176836 ";
  const receipt = `${glog}server.go:213] ACP Request received: session/prompt`;
  const send = `${glog}server.go:1883] Sending user message to conversation 21e17488-d900-4ac3-954e-31c2cec5f86b (items=2, media=3)`;
  const answer = `${glog}prompt.go:358] Responding to pending ask_question for session 21e17488-d900-4ac3-954e-31c2cec5f86b step 896: "yes"`;
  const barrier = `${glog}server.go:213] ACP Request received: ${BARRIER_METHOD}/n-9`;
  const records = parseRecords([receipt, send, answer, barrier].join("\n"), undefined, "n-9");
  assert.deepEqual(
    records.map((record) => record.kind),
    ["prompt", "dispatch", "dispatch", "barrier"],
  );
  assert.equal(classify(records).dispatches, 2);
  assert.equal(classify(records).prompts, 1);
  // sess-1795490077099: `agy` refused the prompt before its agent loop saw
  // it; the window holds its receipt, the barrier and nothing else.
  const refused = parseRecords([receipt, barrier].join("\n"), undefined, "n-9");
  assert.deepEqual(classify(refused), {
    class: "unknown_refusal",
    successes: 0,
    barrier: true,
    prompts: 1,
    dispatches: 0,
    transient: 0,
    refusals: [],
  });
  // Another session's prompt and dispatch read after the barrier are not
  // this turn's.
  const later = parseRecords([receipt, barrier, receipt, send].join("\n"), undefined, "n-9");
  assert.equal(classify(later).prompts, 1);
  assert.equal(classify(later).dispatches, 0);
});
