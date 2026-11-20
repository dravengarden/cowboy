// `agy-proxy --cowboy-usage`: the account's quota report for the
// account-sandbox daemon's `usage` unit (packaging/account-sandbox, "usage").
//
// It first runs the pinned `agy` in print mode with the fixed `/usage`
// command, which answers locally from Cloud Code's quota summary, and passes
// that report through unchanged. Print mode refuses some paid accounts
// before it reaches the command: its eligibility check treats an ineligible
// free tier (`UNSUPPORTED_LOCATION`) as an ineligible account even when a
// paid tier serves it, while ACP sessions on that account still run.
// Only for that refusal it asks Cloud Code's quota summary itself, the same
// request `agy` makes, and prints a report in print mode's own shape.
//
// Credentials: the helper runs as the account's own identity over its own
// home, exactly like `agy`. It reads the access token `agy` just used (print
// mode refreshes and persists it before its eligibility check), sends it only
// to Cloud Code, and never refreshes, writes or prints it. Without a usable
// token it keeps `agy`'s own report, so the daemon's answer is unchanged.
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { request } from "node:https";
import { join } from "node:path";

export const AGY_USAGE_ARGUMENTS = Object.freeze([
  "-p",
  "/usage",
  "--output-format",
  "json",
  "--log-file",
  "/tmp/agy-usage.log",
]);
export const QUOTA_URL =
  "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
/** Cloud Code refuses this endpoint without a client user agent. */
const USER_AGENT = "antigravity";
const TOKEN_PATH = [".gemini", "antigravity-cli", "antigravity-oauth-token"];
/** The daemon accepts at most 256 KiB of report; stay well below. */
const MAX_OUTPUT = 192 * 1024;
const MAX_TOKEN_FILE = 64 * 1024;
/** The daemon's bound is 25 s: print mode, then one request. */
const AGY_TIMEOUT_MS = 15_000;
const REQUEST_TIMEOUT_MS = 8_000;

/** `agy`'s report: exit code and bounded stdout (stderr is dropped). */
export function runAgy(agy, { timeoutMs = AGY_TIMEOUT_MS } = {}) {
  return new Promise((done) => {
    const child = spawn(agy, AGY_USAGE_ARGUMENTS, { stdio: ["ignore", "pipe", "ignore"] });
    const chunks = [];
    let size = 0;
    let overflow = false;
    child.stdout.on("data", (chunk) => {
      size += chunk.length;
      if (size > MAX_OUTPUT) overflow = true;
      else chunks.push(chunk);
    });
    const timer = setTimeout(() => child.kill("SIGKILL"), timeoutMs);
    child.once("error", () => {
      clearTimeout(timer);
      done({ code: null, output: Buffer.alloc(0) });
    });
    child.once("close", (code) => {
      clearTimeout(timer);
      done({
        code: overflow ? null : code,
        output: overflow ? Buffer.alloc(0) : Buffer.concat(chunks),
      });
    });
  });
}

/** The last JSON object line of print mode's output, if any. */
export function envelope(output) {
  const lines = output.toString("utf8").split("\n").reverse();
  for (const line of lines) {
    const text = line.trim();
    if (!text.startsWith("{")) continue;
    try {
      const value = JSON.parse(text);
      if (value && typeof value === "object" && !Array.isArray(value)) return value;
    } catch {}
  }
  return undefined;
}

/** Whether print mode stopped at its eligibility check for the location. */
export function refusedEligibility(code, report) {
  if (code === 0 && (report?.status === undefined || report.status === "SUCCESS")) return false;
  const error = typeof report?.error === "string" ? report.error.toLowerCase() : "";
  return error.includes("eligib") && error.includes("location");
}

/** The access token `agy` keeps in the account home, or `undefined`. */
export async function accessToken(home) {
  try {
    const text = await readFile(join(home, ...TOKEN_PATH), "utf8");
    if (text.length > MAX_TOKEN_FILE) return undefined;
    const token = JSON.parse(text)?.token?.access_token;
    return typeof token === "string" && /^[\x21-\x7e]{16,4096}$/.test(token) ? token : undefined;
  } catch {
    return undefined;
  }
}

/** POST `{}` to Cloud Code's quota summary; the parsed body on 200. */
export function fetchQuota(token, { url = QUOTA_URL, timeoutMs = REQUEST_TIMEOUT_MS } = {}) {
  return new Promise((done) => {
    const body = "{}";
    const req = request(
      url,
      {
        method: "POST",
        // The account unit has no AF_INET6.
        family: 4,
        timeout: timeoutMs,
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
          "User-Agent": USER_AGENT,
        },
      },
      (response) => {
        const chunks = [];
        let size = 0;
        response.on("data", (chunk) => {
          size += chunk.length;
          if (size > MAX_OUTPUT) response.destroy();
          else chunks.push(chunk);
        });
        response.on("error", () => done(undefined));
        response.on("end", () => {
          if (response.statusCode !== 200 || size > MAX_OUTPUT) return done(undefined);
          try {
            done(JSON.parse(Buffer.concat(chunks).toString("utf8")));
          } catch {
            done(undefined);
          }
        });
      },
    );
    req.on("timeout", () => req.destroy());
    req.on("error", () => done(undefined));
    req.end(body);
  });
}

function text(value) {
  return typeof value === "string" && value.trim() ? value : undefined;
}

/**
 * Cloud Code's summary in print mode's quota document shape. Proto3 omits a
 * zero `remainingFraction`, so a bucket with a reset time and no fraction is
 * exhausted. `undefined` when the summary is not the expected shape; the
 * daemon validates the result again.
 */
export function quotaDocument(summary) {
  if (!Array.isArray(summary?.groups)) return undefined;
  const groups = [];
  for (const group of summary.groups) {
    const name = text(group?.displayName);
    if (!name) return undefined;
    const buckets = [];
    for (const bucket of Array.isArray(group.buckets) ? group.buckets : []) {
      const bucketName = text(bucket?.displayName);
      if (!bucketName) return undefined;
      const out = { name: bucketName };
      if (text(bucket.bucketId)) out.id = bucket.bucketId;
      if (text(bucket.window)) out.window = bucket.window;
      if (bucket.disabled === true) out.disabled = true;
      if (typeof bucket.remainingFraction === "number") {
        out.remaining_fraction = bucket.remainingFraction;
      } else if (text(bucket.resetTime) && bucket.disabled !== true) {
        out.remaining_fraction = 0;
      }
      if (text(bucket.resetTime)) out.reset_time = bucket.resetTime;
      buckets.push(out);
    }
    groups.push({ name, buckets });
  }
  return { groups };
}

/** One report: `{ code, output }` for the daemon to read from stdout. */
export async function usageReport({ agy, home, run = runAgy, quota = fetchQuota }) {
  const native = await run(agy);
  if (!refusedEligibility(native.code, envelope(native.output))) return native;
  const token = await accessToken(home);
  const document = token ? quotaDocument(await quota(token)) : undefined;
  if (!document) return native;
  const report = {
    status: "SUCCESS",
    num_turns: 0,
    command: { name: "/usage", data: document },
  };
  return { code: 0, output: Buffer.from(`${JSON.stringify(report)}\n`) };
}

/** The `--cowboy-usage` entry: write the report and return its exit code. */
export async function runUsage({ agy, home = process.env.HOME } = {}) {
  if (typeof home !== "string" || !home.startsWith("/")) return 1;
  const { code, output } = await usageReport({ agy, home });
  await new Promise((done) => process.stdout.write(output, done));
  return code ?? 1;
}
