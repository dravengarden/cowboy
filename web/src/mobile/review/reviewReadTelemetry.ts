import { reportClientLog } from "../../observability";

/** Reads a reader waits on; background polls only report failures. */
const REPORTED_COMPLETIONS = new Set([
  "changes",
  "commit",
  "commit-diff",
  "diff",
  "file",
  "repository",
  "search",
  "tree",
]);

/** The Code API resource of a session read, without ids or query values. */
export function codeReadEndpoint(url: string): string {
  const path = new URL(url, "http://cowboy.invalid").pathname;
  const match = /^\/api\/code\/sessions\/[^/]+\/([a-z-]+)$/u.exec(path);
  return match?.[1] ?? "other";
}

/**
 * Record how long a Review read took and how it ended, so Review latency and
 * failures are visible in client telemetry. Cancelled reads are not reported.
 */
export function reportCodeRead(
  url: string,
  startedAt: number,
  status: number | "network" | "version",
): void {
  const endpoint = codeReadEndpoint(url);
  const duration_ms = Math.round(performance.now() - startedAt);
  if (status === 200) {
    if (!REPORTED_COMPLETIONS.has(endpoint)) return;
    reportClientLog("info", "review_read_completed", "Review read completed", {
      endpoint,
      duration_ms,
    });
    return;
  }
  reportClientLog("warn", "review_read_failed", "Review read failed", {
    endpoint,
    duration_ms,
    status: String(status),
  });
}
