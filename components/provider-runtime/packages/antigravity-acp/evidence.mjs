// Backend refusal evidence for one Antigravity turn.
// `agy` may surface explicit Google backend refusals (account verification,
// location, quota) as ordinary agent text with `stopReason: end_turn`, so
// Cowboy scans the turn's `agy` log window strictly for structured backend
// status fields (HTTP code, `google.rpc` status and `ErrorInfo.reason`), never
// model or message text.
//
// Missing logs, truncated windows, crashes, and unrecognized errors never
// synthesize an account-parking `unknown_refusal`: only explicit proven Google
// refusals transition account state.
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
// Highest first: the class a turn settles to when several explicit refusals appear.
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
const TRANSIENT_CODES = new Set([500, 502, 503, 504]);
const transient = (record) => record.kind === "retry" && TRANSIENT_CODES.has(record.code);
const DISPATCH = new RegExp(
  `${PREFIX}(?:server\\.go:\\d+\\] Sending user message to conversation \\S+ \\(items=\\d+, media=\\d+\\)$|prompt\\.go:\\d+\\] Responding to pending ask_question for session )`,
);
const PROMPT_RECEIPT = new RegExp(
  `${PREFIX}server\\.go:\\d+\\] ACP Request received: session/prompt$`,
);
const REQUEST_FAILURE = new RegExp(
  `${PREFIX}http_helpers\\.go:\\d+\\] Failed to make code assist backend request \\((https://[^)\\s]+)\\): \\{$`,
);

/** Legacy 1.5.9 stream barrier pattern (retained for parsing older logs). */
export function barrierPattern(sessionId) {
  return new RegExp(
    `${PREFIX}server\\.go:\\d+\\] Stream goroutine exited for ${escape(sessionId)}, sending completion signal$`,
  );
}

/** Legacy barrier method name (no longer sent as a synthetic RPC). */
export const BARRIER_METHOD = "_cowboy/evidence_barrier";

/** Legacy request barrier line pattern. */
export function requestBarrierPattern(nonce) {
  return new RegExp(
    `${PREFIX}server\\.go:\\d+\\] ACP Request received: ${escape(`${BARRIER_METHOD}/${nonce}`)}$`,
  );
}

function escape(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Extract strictly numeric, content-free backend step timing diagnostics from
 * one turn's log window. Never reads URLs, Trace IDs, Response IDs, or text
 * content. When neither `sessionId` nor `nonce` is given, scans the entire
 * non-truncated turn window cleanly without requiring a synthetic barrier RPC.
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
  const matched = barrier
    ? sawBarrier
    : stepTimes.length > 0 || transientRetries > 0 || anchorMs !== undefined;
  if (!matched && stepTimes.length === 0 && transientRetries === 0) {
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
    barrier_matched: matched,
    backend_steps: matched ? stepTimes.length : null,
    transient_retries: matched ? transientRetries : null,
    first_step_offset_ms: matched ? firstStepOffsetMs : null,
    step_span_ms: matched ? stepSpanMs : null,
    max_inter_step_ms: matched ? maxInterStepMs : null,
  };
}

/**
 * Parse log text into backend records: `success`, `step_failure`,
 * `request_failure`, `prompt`, `dispatch`, and optional `barrier`.
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
    return { status: undefined, code: undefined, reasons: [] };
  }
}

/**
 * The explicit Google refusal class a backend failure record proves, or `null`
 * when the failure is not a known account-level Google refusal.
 */
export function refusalClass({ status, code, reasons = [] }) {
  if (reasons.includes("VALIDATION_REQUIRED")) return "verification_required";
  if (reasons.includes("UNSUPPORTED_LOCATION")) return "location_unsupported";
  if (status === "RESOURCE_EXHAUSTED" || code === 429) return "rate_limited";
  return null;
}

/**
 * Classify a turn from the records of its window. Only explicit proven Google
 * backend refusals (`verification_required`, `location_unsupported`,
 * `rate_limited`, `suspended`) produce a non-`ok` class; missing barriers,
 * zero backend steps, or unrecognized errors never park the account.
 */
export function classify(records, { prior = [] } = {}) {
  const barrier = records.some((record) => record.kind === "barrier");
  const successes = records.filter((record) => record.kind === "success").length;
  const known = [...prior, ...records].filter((record) => record.kind === "request_failure");
  const refusals = [];
  const classes = new Set();
  for (const record of records) {
    if (record.kind === "request_failure") {
      const klass = refusalClass(record);
      if (klass) {
        refusals.push(record);
        classes.add(klass);
      }
      continue;
    }
    if (transient(record)) continue;
    if (record.kind === "step_failure" || record.kind === "retry") {
      const source = known.findLast(
        (failure) =>
          failure.status === record.status &&
          failure.code === record.code &&
          failure.reasons.length,
      );
      const enriched = { ...record, reasons: source ? source.reasons : [] };
      const klass = refusalClass(enriched);
      if (klass) {
        refusals.push(enriched);
        classes.add(klass);
      }
    }
  }
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
  if (classes.size > 0) {
    return { class: PRECEDENCE.find((name) => classes.has(name)), ...evidence };
  }
  return { class: "ok", ...evidence };
}

const STEP_TIMINGS = new WeakMap();

/** Retrieve content-free step timings associated with a `settle()` result. */
export function stepTimingsFor(classified) {
  return classified && typeof classified === "object" ? STEP_TIMINGS.get(classified) : undefined;
}

/**
 * Non-blocking reader for one `agy acp` process log (`acp-server-*-<pid>.log`).
 * Performs one bounded read at turn end strictly for explicit Google refusals
 * and numeric step timing diagnostics; never polls or sends synthetic RPCs.
 */
export class AgyLog {
  constructor(directory, pid) {
    this.directory = directory;
    this.pid = pid;
    this.path = undefined;
    this.childExited = false;
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

  /** The cursor of a turn about to start (best-effort; `undefined` if no log yet). */
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

  markExited() {
    this.childExited = true;
  }

  /**
   * Settle a finished turn with a single bounded log read (plus one re-read if
   * the child exited mid-read). Missing or unreadable logs cleanly return `ok`.
   */
  async settle(cursor, _sessionId, _stopReason) {
    if (cursor === undefined) return classify([]);
    try {
      let windowText;
      let truncated = false;
      for (let pass = 0; pass < 2; pass++) {
        const exitedBefore = this.childExited;
        const size = (await stat(await this.locate())).size;
        const span = Math.max(0, size - cursor);
        truncated = span > MAX_WINDOW;
        const from = truncated ? size - MAX_WINDOW : cursor;
        windowText = await this.read(from, size);
        if (exitedBefore || !this.childExited) break;
      }
      const records = parseRecords(windowText);
      const needsPrior = records.some(
        (record) =>
          (record.kind === "step_failure" || record.kind === "retry") && !transient(record),
      );
      const prior = needsPrior
        ? parseRecords(await this.read(Math.max(0, cursor - MAX_WINDOW), cursor))
        : [];
      const result = classify(records, { prior });
      if (!truncated) {
        const timings = parseStepTimings(windowText);
        if (timings) STEP_TIMINGS.set(result, timings);
      }
      return result;
    } catch {
      return classify([]);
    }
  }
}
