import type { CodeCheckRun, CodeChecks, CodeDiagnostic, CodeLanguage } from "./codeApi";

/** LSP severities, which the code viewer already styles. */
const SEVERITY = { error: 1, warning: 2, note: 3 } as const;
/** Clamped to the line end by the viewer: a finding without a usable span
 *  marks the rest of its line. */
const LINE_END = 1 << 20;

function samePath(a: string, b: string): boolean {
  const normal = (path: string) => path.replace(/^(\.\/)+/, "");
  return normal(a) === normal(b);
}

export function checkTitle(checks: CodeChecks, id: string): string {
  return checks.declared.find((check) => check.id === id)?.title ?? id;
}

/**
 * Findings of the latest runs that name this file, as viewer diagnostics.
 * They are evidence for the file as each check saw it, so a later edit can
 * move them; the status row says when each check ran.
 */
export function checkDiagnosticsForPath(
  checks: CodeChecks | undefined,
  path: string,
): CodeDiagnostic[] {
  if (!checks) return [];
  return checks.runs.flatMap((run) =>
    run.diagnostics
      .filter((diagnostic) => diagnostic.line > 0 && samePath(diagnostic.path, path))
      .map((diagnostic) => {
        const row = diagnostic.line - 1;
        const code = diagnostic.code ? `[${diagnostic.code}] ` : "";
        return {
          start: { row, column: Math.max(0, diagnostic.column - 1) },
          end: { row, column: LINE_END },
          severity: SEVERITY[diagnostic.severity],
          source: checkTitle(checks, run.check),
          message: `${checkTitle(checks, run.check)}: ${code}${diagnostic.message}`,
        };
      }),
  );
}

/** Language data with check findings added; synthesized when the language
 *  engine has nothing for the file. */
export function withCheckDiagnostics(
  language: CodeLanguage | undefined,
  path: string,
  diagnostics: CodeDiagnostic[],
): CodeLanguage | undefined {
  if (diagnostics.length === 0) return language;
  if (!language) {
    return {
      apiVersion: 1,
      path,
      version: [],
      diagnostics,
      inlayHints: [],
      semanticTokens: [],
    };
  }
  return {
    ...language,
    diagnostics: [...language.diagnostics, ...diagnostics],
  };
}

export interface CheckFileCounts {
  errors: number;
  warnings: number;
  notes: number;
}

export function countForPath(run: CodeCheckRun, path: string): CheckFileCounts {
  const counts = { errors: 0, warnings: 0, notes: 0 };
  for (const diagnostic of run.diagnostics) {
    if (!samePath(diagnostic.path, path)) continue;
    if (diagnostic.severity === "error") counts.errors += 1;
    else if (diagnostic.severity === "warning") counts.warnings += 1;
    else counts.notes += 1;
  }
  return counts;
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export function formatCounts(counts: CheckFileCounts): string {
  const parts = [
    counts.errors && plural(counts.errors, "error"),
    counts.warnings && plural(counts.warnings, "warning"),
    counts.notes && plural(counts.notes, "note"),
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(", ") : "no findings";
}

const STATE_LABEL: Record<CodeCheckRun["state"], string> = {
  running: "running",
  passed: "passed",
  failed: "failed",
  timed_out: "timed out",
  error: "could not run",
};

export function relativeAge(ms: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

export interface CheckStatusSummary {
  severity: "error" | "warning" | "info" | "success";
  summary: string;
  detail: string;
}

/** One status line for a file: what the latest check runs found in it. */
export function checkStatusSummary(
  checks: CodeChecks | undefined,
  path: string,
  now: number,
): CheckStatusSummary | undefined {
  if (!checks || checks.runs.length === 0) return undefined;
  const here = { errors: 0, warnings: 0, notes: 0 };
  const lines = checks.runs.map((run) => {
    const counts = countForPath(run, path);
    here.errors += counts.errors;
    here.warnings += counts.warnings;
    here.notes += counts.notes;
    const title = checkTitle(checks, run.check);
    if (run.state === "running") {
      return `${title}: running since ${relativeAge(run.startedAtMs, now)}.`;
    }
    const age = relativeAge(run.finishedAtMs ?? run.startedAtMs, now);
    const total = run.diagnostics.length;
    const more = run.truncated ? "+" : "";
    const reason = run.error ? ` (${run.error})` : "";
    return `${title}: ${STATE_LABEL[run.state]} ${age}${reason} · ${plural(
      total,
      "finding",
    )}${more}, ${formatCounts(counts)} in this file.`;
  });
  const running = checks.runs.filter((run) => run.state === "running");
  const incomplete = checks.runs.some(
    (run) =>
      (run.state !== "passed" && run.state !== "running") || run.truncated || run.outputTruncated,
  );
  const findings = here.errors + here.warnings + here.notes;
  const severity =
    here.errors > 0
      ? "error"
      : here.warnings > 0 || incomplete
        ? "warning"
        : running.length > 0 || findings > 0
          ? "info"
          : "success";
  const summary =
    findings > 0
      ? `Checks · ${formatCounts(here)} in this file`
      : running.length > 0
        ? `Checks running · ${running.map((run) => checkTitle(checks, run.check)).join(", ")}`
        : incomplete
          ? "Checks incomplete or failed · no findings reported in this file"
          : "Checks · no findings in this file";
  return {
    severity,
    summary,
    detail: [
      ...lines,
      "Findings reflect the files as each check saw them; edits since then can move them.",
    ].join(" "),
  };
}
