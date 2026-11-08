// A managed review's structured findings, anchored for Desktop Review. A
// finding is `current` only while its file is unchanged since the review was
// asked (same change fingerprint); then its lines can be drawn. A changed
// file makes it `stale` and a file outside the delegation `unmapped`: both
// are listed, never drawn. An incomplete or truncated review yields no
// findings to anchor at all.

import type { DelegatedFile } from "./reviewDelegations";

export type FindingSeverity = "critical" | "high" | "medium" | "low";
export type FindingAnchor = "current" | "stale" | "unmapped";

export interface ReviewFinding {
  readonly key: string;
  readonly severity: FindingSeverity;
  readonly title: string;
  readonly body: string;
  readonly path: string;
  readonly lineStart: number;
  readonly lineEnd: number;
  readonly anchor: FindingAnchor;
}

export interface ReviewOutcome {
  readonly verdict: string | undefined;
  readonly complete: boolean;
  readonly findings: readonly ReviewFinding[];
}

const SEVERITIES: ReadonlySet<string> = new Set(["critical", "high", "medium", "low"]);

function samePath(a: string, b: string): boolean {
  const normal = (path: string) => path.replace(/^(\.\/)+/, "");
  return normal(a) === normal(b);
}

/**
 * Read a review call's result. `structured` follows the review output schema
 * (components/wire/src/managed_calls/review.rs); `fingerprintOf` is the file's
 * current change fingerprint, or undefined when it is not in the change list.
 */
export function reviewOutcome(
  callId: string,
  result: { structured?: unknown; truncated?: unknown } | null,
  files: readonly DelegatedFile[],
  fingerprintOf: (path: string) => string | undefined,
): ReviewOutcome {
  const root = result?.structured && typeof result.structured === "object"
    ? result.structured as Record<string, unknown>
    : undefined;
  const verdict = typeof root?.verdict === "string" ? root.verdict : undefined;
  const complete = root?.review_complete === true && verdict !== "incomplete" && result?.truncated !== true &&
    Array.isArray(root?.findings);
  if (!complete) return { verdict, complete: false, findings: [] };
  const findings = (root!.findings as unknown[]).flatMap((value, index): ReviewFinding[] => {
    if (!value || typeof value !== "object") return [];
    const item = value as Record<string, unknown>;
    if (typeof item.file !== "string" || typeof item.line_start !== "number" || !SEVERITIES.has(String(item.severity))) return [];
    const delegated = files.find((file) => samePath(file.path, item.file as string));
    const now = delegated ? fingerprintOf(delegated.path) : undefined;
    const anchor: FindingAnchor = !delegated
      ? "unmapped"
      : delegated.fingerprint !== undefined && now === delegated.fingerprint
      ? "current"
      : "stale";
    return [{
      key: `${callId}:${String(index)}`,
      severity: item.severity as FindingSeverity,
      title: typeof item.title === "string" ? item.title : "Finding",
      body: typeof item.body === "string" ? item.body : "",
      path: delegated?.path ?? item.file,
      lineStart: Math.max(1, item.line_start),
      lineEnd: Math.max(1, typeof item.line_end === "number" ? item.line_end : item.line_start),
      anchor,
    }];
  });
  return { verdict, complete: true, findings };
}

const RANK: Readonly<Record<FindingSeverity, number>> = { critical: 0, high: 1, medium: 2, low: 3 };

/** The Navigator badge of a file: its current findings and the worst one. */
export function findingBadge(
  findings: readonly ReviewFinding[],
  path: string,
): { count: number; severity: FindingSeverity } | undefined {
  const here = findings.filter((finding) => finding.anchor === "current" && samePath(finding.path, path));
  if (here.length === 0) return undefined;
  const severity = here.reduce((worst, finding) => RANK[finding.severity] < RANK[worst] ? finding.severity : worst, "low" as FindingSeverity);
  return { count: here.length, severity };
}
