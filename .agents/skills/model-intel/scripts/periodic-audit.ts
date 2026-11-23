// Read-only periodic audit for Cowboy model evidence, review routing, live
// ledger costs, and Agent Provider harness dependencies.
//
// Usage (repository root, dev shell or bun):
//   bun .agents/skills/model-intel/scripts/periodic-audit.ts \
//     --repo . --database /var/lib/cowboy/cowboy.sqlite3 --days 14 \
//     --check-upstream --out-dir /var/lib/cowboy/model-intel --logfmt
//
// Strictly read-only on the repository and Controller database: writes only
// latest.json and latest.md under --out-dir (when given) and never mutates
// Git checkouts, plugins, routing.json, or sessions.

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import {
  type AuditReport,
  auditDependencies,
  type FetchFn,
} from "../../release-cowboy-plugin/scripts/audit-dependencies.ts";
import {
  readLedgerCalls,
  summarizeTelemetry,
  type TelemetrySummary,
  writeFileGuarded,
} from "./field.ts";
import {
  agentManifests,
  audit,
  type Catalog,
  digest,
  fieldSnapshotFiles,
  type Inputs,
  type Methodology,
  readObservations,
  type RoutingTable,
  type Target,
} from "./score.ts";

export type HarnessSummary = {
  provider_id: string;
  version: string;
  default_preset: string | null;
  preset_count: number;
  targets_covered: number;
  routed_for_review: boolean;
};

export type PeriodicAuditReport = {
  schema: 1;
  audited_at: string;
  as_of: string;
  action_needed: boolean;
  reasons: string[];
  catalog_audit: {
    as_of: string;
    findings: string[];
  };
  harness_inventory: HarnessSummary[];
  field_telemetry:
    | ({
        available: true;
        window_days: number;
        total_reviews: number;
        total_analyses: number;
      } & TelemetrySummary)
    | {
        available: false;
        reason: string;
      };
  upstream_dependencies:
    | {
        checked: true;
        upgrades_available: number;
        integrity_mismatches: number;
        workspace_drifts: number;
        check_failures: number;
        report: AuditReport;
      }
    | {
        checked: false;
      };
};

export type PeriodicAuditOptions = {
  repoRoot?: string;
  database?: string;
  days?: number;
  asOf?: string;
  localObservations?: string;
  checkUpstream?: boolean;
  driftThreshold?: number;
  driftMinSamples?: number;
  fetchFn?: FetchFn;
  nowMs?: number;
};

export async function runPeriodicAudit(
  options: PeriodicAuditOptions = {},
): Promise<PeriodicAuditReport> {
  const repoRoot = resolve(options.repoRoot ?? ".");
  const modelsRoot = join(repoRoot, "models");
  const pluginsDir = join(repoRoot, "plugins");
  const nowMs = options.nowMs ?? Date.now();
  const auditedAt = new Date(nowMs).toISOString();
  const asOf = options.asOf ?? auditedAt.slice(0, 10);
  const windowDays = options.days ?? 14;
  const state = process.env.XDG_STATE_HOME ?? join(process.env.HOME ?? "", ".local/state");
  const localDir = options.localObservations ?? join(state, "cowboy/model-intel/observations");

  const methodologyPath = join(modelsRoot, "methodology.toml");
  const targetsPath = join(modelsRoot, "targets.toml");
  const catalogPath = join(modelsRoot, "catalog.json");
  const routingPath = join(modelsRoot, "routing.json");

  const methodologyText = readFileSync(methodologyPath, "utf8");
  const targetsText = readFileSync(targetsPath, "utf8");
  const methodology = Bun.TOML.parse(methodologyText) as unknown as Methodology;
  const targets = (Bun.TOML.parse(targetsText) as { targets: Target[] }).targets;
  const manifests = agentManifests(pluginsDir);
  const routing = existsSync(routingPath)
    ? (JSON.parse(readFileSync(routingPath, "utf8")) as RoutingTable)
    : undefined;

  const repoObs = readObservations(join(modelsRoot, "observations"), false);
  const localObs: Inputs = readObservations(localDir, true);
  const findings = existsSync(catalogPath)
    ? audit(
        JSON.parse(readFileSync(catalogPath, "utf8")) as Catalog,
        targets,
        manifests,
        {
          repository: repoObs.digest,
          local: localObs.present ? localObs.digest : null,
          methodology: digest(methodologyText),
          targets: digest(targetsText),
        },
        asOf,
        methodology.freshness.provisional_days,
        routing ? { routing, fieldSnapshots: fieldSnapshotFiles(modelsRoot) } : undefined,
      )
    : ["no committed models/catalog.json"];

  const harnessInventory: HarnessSummary[] = Object.entries(manifests)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([providerId, manifest]) => {
      const presets = manifest.configuration_presets ?? [];
      const covered = presets.filter((preset) =>
        targets.some((t) => t.provider === providerId && t.preset === preset.id),
      ).length;
      return {
        provider_id: providerId,
        version: manifest.version ?? "unknown",
        default_preset: presets.find((preset) => preset.is_default)?.id ?? null,
        preset_count: presets.length,
        targets_covered: covered,
        routed_for_review: Boolean(routing?.providers[providerId]),
      };
    });

  let fieldTelemetry: PeriodicAuditReport["field_telemetry"];
  const database = options.database;
  if (database && existsSync(database)) {
    try {
      const since = nowMs - windowDays * 86_400_000;
      const calls = readLedgerCalls(database, since, ["review", "design_review", "analysis"]);
      const summary = summarizeTelemetry(calls, routing, {
        minSamples: options.driftMinSamples ?? 5,
        threshold: options.driftThreshold ?? 0.3,
      });
      fieldTelemetry = {
        available: true,
        window_days: windowDays,
        total_reviews: summary.review_groups.reduce((s, g) => s + g.n, 0),
        total_analyses: summary.analysis_groups.reduce((s, g) => s + g.n, 0),
        ...summary,
      };
    } catch (error) {
      fieldTelemetry = {
        available: false,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  } else {
    fieldTelemetry = {
      available: false,
      reason: database ? `database not found at ${database}` : "no database path configured",
    };
  }

  let upstreamDependencies: PeriodicAuditReport["upstream_dependencies"];
  if (options.checkUpstream) {
    const depReport = await auditDependencies({
      repoRoot,
      requested: "all",
      fetchFn: options.fetchFn,
    });
    const countStatus = (status: string) =>
      depReport.dependencies.filter((row) => row.status === status).length;
    upstreamDependencies = {
      checked: true,
      upgrades_available: countStatus("upgrade_available"),
      integrity_mismatches: countStatus("integrity_mismatch"),
      workspace_drifts: countStatus("workspace_drift"),
      check_failures: countStatus("upstream_check_failed"),
      report: depReport,
    };
  } else {
    upstreamDependencies = { checked: false };
  }

  const reasons: string[] = [];
  for (const finding of findings) {
    // Missing local reference_only observations on a machine that does not store
    // them is informational rather than an urgent action by itself.
    if (finding === "local reference-only observations unavailable; local inputs not verified") {
      continue;
    }
    reasons.push(`catalog: ${finding}`);
  }
  if (fieldTelemetry.available) {
    for (const drift of fieldTelemetry.cost_drifts) {
      const pct = `${drift.delta_ratio >= 0 ? "+" : ""}${Math.round(drift.delta_ratio * 100)}%`;
      reasons.push(
        `field-cost-drift: ${drift.target} median ${drift.live_median_tokens} vs recorded ${drift.recorded_median_tokens} (${pct}, n=${drift.n})`,
      );
    }
  }
  if (upstreamDependencies.checked) {
    for (const row of upstreamDependencies.report.dependencies) {
      if (row.status === "upgrade_available") {
        reasons.push(
          `harness-upgrade: ${row.provider_id}/${row.dependency_id} ${row.current_version} -> ${row.candidate_version}`,
        );
      } else if (row.status === "integrity_mismatch") {
        reasons.push(
          `integrity-mismatch: ${row.provider_id}/${row.dependency_id}@${row.current_version}`,
        );
      } else if (row.status === "workspace_drift") {
        reasons.push(
          `workspace-drift: ${row.provider_id}/${row.dependency_id} ${row.current_version} vs local ${row.candidate_version}`,
        );
      }
    }
  }

  return {
    schema: 1,
    audited_at: auditedAt,
    as_of: asOf,
    action_needed: reasons.length > 0,
    reasons,
    catalog_audit: { as_of: asOf, findings },
    harness_inventory: harnessInventory,
    field_telemetry: fieldTelemetry,
    upstream_dependencies: upstreamDependencies,
  };
}

export function renderMarkdown(report: PeriodicAuditReport): string {
  const lines: string[] = [
    `# Cowboy model & harness audit (${report.as_of})`,
    "",
    `- Audited at: ${report.audited_at}`,
    `- Action needed: **${report.action_needed ? "yes" : "no"}**`,
    "",
  ];
  if (report.reasons.length > 0) {
    lines.push("## Action items", ...report.reasons.map((r) => `- ${r}`), "");
  }
  lines.push("## Catalog & routing audit");
  if (report.catalog_audit.findings.length === 0) {
    lines.push("- Clean (0 findings)", "");
  } else {
    lines.push(...report.catalog_audit.findings.map((f) => `- ${f}`), "");
  }
  lines.push(
    "## Harness inventory",
    "| Provider | Version | Default preset | Presets (targeted) | Review routed |",
    "| --- | --- | --- | --- | --- |",
    ...report.harness_inventory.map(
      (h) =>
        `| ${h.provider_id} | ${h.version} | ${h.default_preset ?? "-"} | ${h.preset_count} (${h.targets_covered}) | ${h.routed_for_review ? "yes" : "no"} |`,
    ),
    "",
  );
  lines.push("## Live ledger telemetry");
  if (!report.field_telemetry.available) {
    lines.push(`- Unavailable: ${report.field_telemetry.reason}`, "");
  } else {
    const ft = report.field_telemetry;
    lines.push(
      `- Window: ${ft.window_days} days · completed reviews: ${ft.total_reviews} · analyses: ${ft.total_analyses}`,
      `- Review selection: ${JSON.stringify(ft.review_selection)} · risk: ${JSON.stringify(ft.review_risk)}`,
      "",
    );
    if (ft.review_groups.length > 0) {
      lines.push(
        "| Provider | Purpose | Target | n | Median tokens | Median min |",
        "| --- | --- | --- | --- | --- | --- |",
        ...ft.review_groups.map(
          (g) =>
            `| ${g.provider} | ${g.purpose} | ${g.target} | ${g.n} | ${g.median_tokens ?? "-"} | ${g.median_minutes ?? "-"} |`,
        ),
        "",
      );
    }
    if (ft.analysis_groups.length > 0) {
      lines.push(
        "### Analysis calls",
        "| Provider | Target | n | Median tokens | Median min |",
        "| --- | --- | --- | --- | --- |",
        ...ft.analysis_groups.map(
          (g) =>
            `| ${g.provider} | ${g.target} | ${g.n} | ${g.median_tokens ?? "-"} | ${g.median_minutes ?? "-"} |`,
        ),
        "",
      );
    }
  }
  lines.push("## Upstream harness dependencies");
  if (!report.upstream_dependencies.checked) {
    lines.push("- Not checked (`--check-upstream` omitted)", "");
  } else {
    const ud = report.upstream_dependencies;
    lines.push(
      `- Upgrades available: ${ud.upgrades_available} · integrity mismatches: ${ud.integrity_mismatches} · workspace drift: ${ud.workspace_drifts} · check failures: ${ud.check_failures}`,
      "",
    );
    const notable = ud.report.dependencies.filter(
      (d) =>
        d.status === "upgrade_available" ||
        d.status === "integrity_mismatch" ||
        d.status === "workspace_drift" ||
        d.status === "upstream_check_failed",
    );
    if (notable.length > 0) {
      lines.push(
        "| Provider | Dependency | Current | Candidate | Status | Detail |",
        "| --- | --- | --- | --- | --- | --- |",
        ...notable.map(
          (d) =>
            `| ${d.provider_id} | ${d.dependency_id} | ${d.current_version} | ${d.candidate_version ?? "-"} | ${d.status} | ${d.detail ?? "-"} |`,
        ),
        "",
      );
    }
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

export function formatLogfmt(report: PeriodicAuditReport): string {
  const fields: Array<[string, string | number | boolean]> = [
    ["event_name", "cowboy.model_intel.audit"],
    ["as_of", report.as_of],
    ["action_needed", report.action_needed],
    ["reasons", report.reasons.length],
    ["catalog_findings", report.catalog_audit.findings.length],
    ["harnesses", report.harness_inventory.length],
    ["ledger_available", report.field_telemetry.available],
  ];
  if (report.field_telemetry.available) {
    fields.push(
      ["reviews", report.field_telemetry.total_reviews],
      ["analyses", report.field_telemetry.total_analyses],
      ["cost_drifts", report.field_telemetry.cost_drifts.length],
    );
  }
  if (report.upstream_dependencies.checked) {
    fields.push(
      ["upgrades_available", report.upstream_dependencies.upgrades_available],
      ["integrity_mismatches", report.upstream_dependencies.integrity_mismatches],
      ["workspace_drifts", report.upstream_dependencies.workspace_drifts],
      ["upstream_failures", report.upstream_dependencies.check_failures],
    );
  }
  return fields
    .map(([k, v]) =>
      typeof v === "string" && /\s/.test(v) ? `${k}=${JSON.stringify(v)}` : `${k}=${v}`,
    )
    .join(" ");
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const option = (name: string) => {
    const index = args.indexOf(name);
    return index >= 0 ? args[index + 1] : undefined;
  };
  const repoRoot = option("--repo") ?? ".";
  const database = option("--database") ?? "/var/lib/cowboy/cowboy.sqlite3";
  const days = Number(option("--days") ?? "14");
  const asOf = option("--as-of");
  const localObservations = option("--local-observations");
  const outDir = option("--out-dir");
  const driftThreshold = Number(option("--drift-threshold") ?? "0.30");
  const checkUpstream = args.includes("--check-upstream");
  const logfmt = args.includes("--logfmt");
  const strict = args.includes("--strict");

  if (!Number.isFinite(days) || days <= 0) throw new Error("--days must be a positive number");
  if (!Number.isFinite(driftThreshold) || driftThreshold <= 0) {
    throw new Error("--drift-threshold must be a positive number");
  }

  const report = await runPeriodicAudit({
    repoRoot,
    database,
    days,
    asOf,
    localObservations,
    checkUpstream,
    driftThreshold,
  });

  const jsonText = `${JSON.stringify(report, null, 2)}\n`;
  if (outDir) {
    const protectedDb = existsSync(database) ? database : undefined;
    writeFileGuarded(join(outDir, "latest.json"), jsonText, protectedDb);
    writeFileGuarded(join(outDir, "latest.md"), renderMarkdown(report), protectedDb);
  } else {
    process.stdout.write(jsonText);
  }
  if (logfmt) {
    process.stderr.write(`${formatLogfmt(report)}\n`);
  }
  if (strict && report.action_needed) process.exit(1);
}
