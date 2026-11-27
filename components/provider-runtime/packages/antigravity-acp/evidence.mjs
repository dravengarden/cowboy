// Backend refusal evidence for one Antigravity turn (design R1-7, R2-12,
// R3-10). `agy` reports Google's refusals (account verification, location,
// quota) as ordinary agent text with `stopReason: end_turn`, so the turn's
// outcome is read from the unit's own `agy` log instead: only backend status
// fields (HTTP code, `google.rpc` status and `ErrorInfo.reason`), never model
// or message text. The exact lines are documented in this package's README.
import { randomUUID } from "node:crypto";
import { open, readdir, stat } from "node:fs/promises";
import { join } from "node:path";

export const CLASSES = Object.freeze([
  "ok",
  "verification_required",
  "location_unsupported",
  "rate_limited",
  "suspended",
  "unknown_refusal",
]);
// Highest first: the class a turn settles to when several refusals appear.
export const PRECEDENCE = [
  "suspended",
  "verification_required",
  "location_unsupported",
  "unknown_refusal",
  "rate_limited",
];
const MAX_WINDOW = 4 * 1024 * 1024;
const MAX_JSON_BLOCK = 64 * 1024;

const PREFIX = String.raw`^[IWEF]\d{4} \d{2}:\d{2}:\d{2}\.\d+\s+\d+ `;
const GLOG_TIME = /^[IWEF](\d{2})(\d{2}) (\d{2}):(\d{2}):(\d{2})\.(\d{1,6})\s+\d+ /;

function parseGlogDayMs(line) {
  const match = GLOG_TIME.exec(line);
  if (!match) return undefined;
  const hours = Number(match[3]);
  const minutes = Number(match[4]);
  const seconds = Number(match[5]);
  const micros = Number(match[6].padEnd(6, "0").slice(0, 6));
  if (hours > 23 || minutes > 59 || seconds > 59) return undefined;
  return ((hours * 60 + minutes) * 60 + seconds) * 1000 + Math.floor(micros / 1000);
}

function monotonicDeltaMs(fromDayMs, toDayMs) {
  if (fromDayMs === undefined || toDayMs === undefined) return undefined;
  const DAY_MS = 86_400_000;
  let delta = toDayMs - fromDayMs;
  if (delta < -DAY_MS / 2) delta += DAY_MS;
  return delta >= 0 && delta <= DAY_MS ? delta : undefined;
}

/**
 * Extract strictly numeric, content-free backend step timing diagnostics from
 * one turn's log window through its barrier. Never reads URLs, Trace IDs,
 * Response IDs, or text content.
 */
export function parseStepTimings(text, sessionId, nonce) {
  const lines = text.split("\n");
  const barrier =
    nonce !== undefined
      ? requestBarrierPattern(nonce)
      : sessionId === undefined
        ? undefined
        : barrierPattern(sessionId);
  let anchorMs;
  let sawBarrier = false;
  const stepTimes = [];
  let transientRetries = 0;
  for (const line of lines) {
    if (barrier?.test(line)) {
      sawBarrier = true;
      break;
    }
    if (SUCCESS.test(line)) {
      const ts = parseGlogDayMs(line);
      if (ts !== undefined) stepTimes.push(ts);
      continue;
    }
    if (anchorMs === undefined && (PROMPT_RECEIPT.test(line) || DISPATCH.test(line))) {
      anchorMs = parseGlogDayMs(line);
      continue;
    }
    const retry = RETRY.exec(line);
    if (retry && TRANSIENT_CODES.has(Number(retry[2]))) {
      transientRetries += 1;
    }
  }
  if (!sawBarrier && stepTimes.length === 0 && transientRetries === 0) {
    return undefined;
  }
  let firstStepOffsetMs = null;
  let stepSpanMs = null;
  let maxInterStepMs = null;
  if (stepTimes.length > 0) {
    const offset = monotonicDeltaMs(anchorMs, stepTimes[0]);
    if (offset !== undefined) firstStepOffsetMs = offset;
    const span = monotonicDeltaMs(stepTimes[0], stepTimes[stepTimes.length - 1]);
    if (span !== undefined) stepSpanMs = span;
    if (stepTimes.length > 1) {
      let maxGap = 0;
      for (let i = 1; i < stepTimes.length; i++) {
        const gap = monotonicDeltaMs(stepTimes[i - 1], stepTimes[i]);
        if (gap !== undefined && gap > maxGap) maxGap = gap;
      }
      maxInterStepMs = maxGap;
    }
  }
  return {
    schema: 1,
    backend_steps: stepTimes.length,
    transient_retries: transientRetries,
    first_step_offset_ms: firstStepOffsetMs,
    step_span_ms: stepSpanMs,
    max_inter_step_ms: maxInterStepMs,
  };
}
// One successful model request (a backend step) of the turn.
const SUCCESS = new RegExp(
  `${PREFIX}http_helpers\\.go:\\d+\\] URL: https://[a-z0-9.-]+\\.googleapis\\.com/v1internal:streamGenerateContent\\?alt=sse Trace: 0x[0-9a-f]+ ResponseID: \\S+$`,
);
// The agent loop's terminal failure of a backend step: status and code.
const STEP_FAILURE = new RegExp(
  `${PREFIX}errorreport\\.go:\\d+\\] (?:agent executor error: )?generating and executing: ([A-Z_]+) \\(code (\\d{3})\\)`,
);
// A failed attempt the agent loop retries: status and code.
const RETRY = new RegExp(
  `${PREFIX}run\\.go:\\d+\\] Run: attempt \\d+ failed \\(([A-Z_]+) \\(code (\\d{3})\\)`,
);
// The only retried failures that count as transient backend capacity; every
// other retried failure (an account refusal, or a code not known here) is
// evidence of a refusal and parks the account rather than passing as ok.
const TRANSIENT_CODES = new Set([500, 502, 503, 504]);
const transient = (record) => record.kind === "retry" && TRANSIENT_CODES.has(record.code);
// `agy` handing the prompt to its agent loop: a new user message, or the
// answer to a pending `ask_question`. Its absence in a turn's window (with
// no backend record either) is what proves a rejected prompt never left the
// process (`localRejection` in `proxy.mjs`).
const DISPATCH = new RegExp(
  `${PREFIX}(?:server\\.go:\\d+\\] Sending user message to conversation \\S+ \\(items=\\d+, media=\\d+\\)$|prompt\\.go:\\d+\\] Responding to pending ask_question for session )`,
);
// `agy` reading a prompt request (logged before it handles it).
const PROMPT_RECEIPT = new RegExp(
  `${PREFIX}server\\.go:\\d+\\] ACP Request received: session/prompt$`,
);
// A failed backend request whose structured error follows as JSON lines.
const REQUEST_FAILURE = new RegExp(
  `${PREFIX}http_helpers\\.go:\\d+\\] Failed to make code assist backend request \\((https://[^)\\s]+)\\): \\{$`,
);

/**
 * The stream barrier `agy` 1.5.9 writes after the last backend step of a
 * turn. From 1.5.10 the conversation stream outlives the turn (it carries a
 * woken agent's replies between turns), so the line no longer marks a turn's
 * end; it is read only by an `AgyLog` without a barrier request.
 */
export function barrierPattern(sessionId) {
  return new RegExp(
    `${PREFIX}server\\.go:\\d+\\] Stream goroutine exited for ${escape(sessionId)}, sending completion signal$`,
  );
}

/**
 * The request the proxy sends `agy` once a turn has replied, as
 * `<method>/<nonce>`. `agy` answers it "Method not found", but it logs every
 * request it reads before handling it (`ACP Request received: <method>`, in
 * 1.5.9 and 1.5.14 alike), so that line follows every line the turn logged
 * before its reply: the turn's barrier.
 */
export const BARRIER_METHOD = "_cowboy/evidence_barrier";

/** The line `agy` writes when it reads the barrier request `nonce` names. */
export function requestBarrierPattern(nonce) {
  return new RegExp(
    `${PREFIX}server\\.go:\\d+\\] ACP Request received: ${escape(`${BARRIER_METHOD}/${nonce}`)}$`,
  );
}

function escape(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Parse log text into backend records: `success`, `step_failure` (status,
 * code) and `request_failure` (status, code, reasons), `prompt` (`agy` read
 * a `session/prompt`) and `dispatch` (a prompt reached its agent loop). Structured JSON is
 * read only from a request failure's own block and only for its `code`,
 * `status` and `details[].reason` fields. The barrier is the request
 * barrier `nonce` names when given, else the session's stream barrier.
 */
export function parseRecords(text, sessionId, nonce) {
  const lines = text.split("\n");
  const barrier =
    nonce !== undefined
      ? requestBarrierPattern(nonce)
      : sessionId === undefined
        ? undefined
        : barrierPattern(sessionId);
  const records = [];
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    if (SUCCESS.test(line)) {
      records.push({ kind: "success" });
      continue;
    }
    if (barrier?.test(line)) {
      records.push({ kind: "barrier" });
      continue;
    }
    if (DISPATCH.test(line)) {
      records.push({ kind: "dispatch" });
      continue;
    }
    if (PROMPT_RECEIPT.test(line)) {
      records.push({ kind: "prompt" });
      continue;
    }
    const retry = RETRY.exec(line);
    if (retry) {
      records.push({ kind: "retry", status: retry[1], code: Number(retry[2]) });
      continue;
    }
    const step = STEP_FAILURE.exec(line);
    if (step) {
      records.push({ kind: "step_failure", status: step[1], code: Number(step[2]) });
      continue;
    }
    if (REQUEST_FAILURE.test(line)) {
      let block = "{";
      let end = index + 1;
      for (; end < lines.length && block.length <= MAX_JSON_BLOCK; end++) {
        block += `\n${lines[end]}`;
        if (lines[end] === "}") break;
      }
      index = end;
      records.push({ kind: "request_failure", ...structuredError(block) });
    }
  }
  return records;
}

function structuredError(block) {
  try {
    const error = JSON.parse(block)?.error;
    const reasons = [];
    for (const detail of Array.isArray(error?.details) ? error.details : []) {
      if (
        detail?.["@type"] === "type.googleapis.com/google.rpc.ErrorInfo" &&
        typeof detail.reason === "string" &&
        /^[A-Z_]{1,64}$/.test(detail.reason)
      ) {
        reasons.push(detail.reason);
      }
    }
    return {
      status: typeof error?.status === "string" ? error.status : undefined,
      code: Number.isInteger(error?.code) ? error.code : undefined,
      reasons,
    };
  } catch {
    // An unreadable block is still a failed backend request.
    return { status: undefined, code: undefined, reasons: [] };
  }
}

/** The class one refusal record maps to. */
export function refusalClass({ status, code, reasons = [] }) {
  if (reasons.includes("VALIDATION_REQUIRED")) return "verification_required";
  if (reasons.includes("UNSUPPORTED_LOCATION")) return "location_unsupported";
  if (status === "RESOURCE_EXHAUSTED" || code === 429) return "rate_limited";
  return "unknown_refusal";
}

/**
 * Classify a turn from the records of its window. `prior` holds request
 * failures earlier in the same process log: a step failure that carries no
 * reason of its own takes the reason of the latest earlier failure with the
 * same status and code (`agy` logs the structured error of a refused account
 * once, from its background quota request, and the turn's step failure
 * repeats only status and code).
 */
export function classify(records, { prior = [], stopReason } = {}) {
  const barrier = records.some((record) => record.kind === "barrier");
  const successes = records.filter((record) => record.kind === "success").length;
  const known = [...prior, ...records].filter((record) => record.kind === "request_failure");
  const refusals = [];
  for (const record of records) {
    if (record.kind === "request_failure") refusals.push(record);
    if (transient(record)) continue;
    if (record.kind === "step_failure" || record.kind === "retry") {
      const source = known.findLast(
        (failure) =>
          failure.status === record.status &&
          failure.code === record.code &&
          failure.reasons.length,
      );
      refusals.push({ ...record, reasons: source ? source.reasons : [] });
    }
  }
  // Prompt receipts and dispatches count only through the turn's barrier:
  // another session's prompt read after it is not this turn's.
  const end = records.findIndex((record) => record.kind === "barrier");
  const turn = end < 0 ? records : records.slice(0, end);
  const evidence = {
    successes,
    barrier,
    prompts: turn.filter((record) => record.kind === "prompt").length,
    dispatches: turn.filter((record) => record.kind === "dispatch").length,
    transient: records.filter(transient).length,
    refusals: refusals.map(({ status, code, reasons }) => ({
      status: status ?? null,
      code: code ?? null,
      reasons: reasons ?? [],
    })),
  };
  if (refusals.length) {
    const classes = new Set(refusals.map(refusalClass));
    return { class: PRECEDENCE.find((name) => classes.has(name)), ...evidence };
  }
  // Without the barrier the turn's backend steps may still be running or
  // refusing: the outcome is ambiguous and parks the account, cancelled or
  // not. A turn the client cancelled after its barrier may have had no
  // backend step at all; it is not a refusal.
  if (!barrier) return { class: "unknown_refusal", ...evidence };
  if (stopReason === "cancelled") return { class: "ok", ...evidence };
  if (successes === 0) return { class: "unknown_refusal", ...evidence };
  return { class: "ok", ...evidence };
}

/**
 * The log of one `agy acp` process (`acp-server-<time>-<pid>.log` in its
 * HOME) read with a per-turn byte cursor and a bounded reader. `request`
 * sends `agy` one JSON-RPC message: with it every turn settles on its own
 * barrier request (see `BARRIER_METHOD`); without it, on the 1.5.9 stream
 * barrier.
 */
const STEP_TIMINGS = new WeakMap();

/** Retrieve content-free step timings associated with a `settle()` result. */
export function stepTimingsFor(classified) {
  return classified && typeof classified === "object" ? STEP_TIMINGS.get(classified) : undefined;
}

export class AgyLog {
  constructor(directory, pid, { pollMs = 100, barrierTimeoutMs = 10000, request } = {}) {
    this.directory = directory;
    this.pid = pid;
    this.pollMs = pollMs;
    this.barrierTimeoutMs = barrierTimeoutMs;
    this.request = request;
    this.path = undefined;
  }

  async locate() {
    if (this.path) return this.path;
    const suffix = `-${this.pid}.log`;
    const names = (await readdir(this.directory)).filter(
      (name) => name.startsWith("acp-server-") && name.endsWith(suffix),
    );
    if (names.length !== 1) throw new Error("agy log not found");
    this.path = join(this.directory, names[0]);
    return this.path;
  }

  /** The cursor of a turn about to start. */
  async mark() {
    try {
      return (await stat(await this.locate())).size;
    } catch {
      return undefined;
    }
  }

  async read(from, to) {
    const handle = await open(await this.locate(), "r");
    try {
      const length = Math.min(to - from, MAX_WINDOW);
      const buffer = Buffer.alloc(Math.max(0, length));
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, from);
      return buffer.subarray(0, bytesRead).toString("utf8");
    } finally {
      await handle.close();
    }
  }

  /**
   * Settle a finished turn: wait for its barrier line (bounded), then
   * classify. Missing or unreadable evidence is `unknown_refusal`. Called
   * only after the turn's reply has arrived: its barrier request is sent
   * now, with a nonce no earlier line can carry.
   */
  markExited() {
    this.childExited = true;
  }

  async settle(cursor, sessionId, stopReason) {
    if (cursor === undefined) return classify([]);
    let nonce;
    if (this.request) {
      nonce = randomUUID();
      try {
        // Its reply answers nothing the proxy waits for and is dropped.
        this.request({
          jsonrpc: "2.0",
          id: `cowboy-evidence-${nonce}`,
          method: `${BARRIER_METHOD}/${nonce}`,
          params: {},
        });
      } catch {
        return classify([], {});
      }
    }
    const deadline = Date.now() + this.barrierTimeoutMs;
    try {
      for (;;) {
        const exitedBeforeStat = this.childExited;
        const size = (await stat(await this.locate())).size;
        if (size - cursor > MAX_WINDOW) {
          if (!exitedBeforeStat && this.childExited) continue;
          // Never classify a truncated tail that omits an earlier prefix of the turn.
          return classify([], {});
        }
        const windowText = await this.read(cursor, size);
        const records = parseRecords(windowText, sessionId, nonce);
        if (records.some((record) => record.kind === "barrier")) {
          const timings = parseStepTimings(windowText, sessionId, nonce);
          const prior = parseRecords(
            await this.read(Math.max(0, cursor - MAX_WINDOW), cursor),
            undefined,
          );
          const result = classify(records, { prior, stopReason });
          if (timings) STEP_TIMINGS.set(result, timings);
          return result;
        }
        if (!exitedBeforeStat && this.childExited) continue;
        if (exitedBeforeStat || Date.now() >= deadline) {
          const timings = parseStepTimings(windowText, sessionId, nonce);
          const prior = parseRecords(
            await this.read(Math.max(0, cursor - MAX_WINDOW), cursor),
            undefined,
          );
          if (!exitedBeforeStat && this.childExited) continue;
          const result = classify(records, { prior, stopReason });
          if (timings) STEP_TIMINGS.set(result, timings);
          return result;
        }
        await new Promise((resolve) => setTimeout(resolve, this.pollMs));
      }
    } catch {
      return classify([], {});
    }
  }
}
