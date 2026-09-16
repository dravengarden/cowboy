#!/usr/bin/env -S deno run --allow-run
/**
 * Bring one Machine's installed Plugin inventory up to the Catalog's newest
 * ready release, one signed upgrade at a time.
 *
 * Why this exists: every individual step already had a command, but converging
 * a Machine meant an operator (human or agent) reading the Catalog, comparing
 * versions by eye, and hand-writing an exact version+digest per Plugin. That is
 * where a release stops being reproducible — the digest is the whole trust
 * claim, and typing it is the one place to get it wrong. This resolves the
 * target from the Catalog it just read, so the digest is never authored.
 *
 * It fails closed. A dry run is the default; `--apply` is required to submit,
 * a Plugin holding an active session lease is refused rather than recycled
 * under a live worker, and a target that is not `ready` in the Catalog is
 * skipped. Each upgrade gets a deterministic operation identity, so a retry
 * after a lost response reuses the same one and the Controller can return the
 * saved result instead of installing twice.
 *
 * It does NOT publish, sign, or decide what "latest" should be: it converges to
 * what the Catalog already advertises. Publication stays the separate,
 * explicitly authorized act described in SKILL.md.
 */

export interface CatalogRelease {
  readonly plugin_id: string;
  readonly plugin_version: string;
  readonly artifact_digest: string;
  readonly release_state: string;
}

export interface InstalledPlugin {
  readonly plugin_id: string;
  readonly plugin_version: string;
  readonly state: string;
  readonly active_session_leases: number;
}

export interface ConvergeStep {
  readonly plugin: string;
  readonly from: string;
  readonly to: string;
  readonly digest: string;
  readonly operationId: string;
}

export interface ConvergePlan {
  readonly machine: string;
  readonly steps: ConvergeStep[];
  /** Everything deliberately not attempted, with the reason. Never silent. */
  readonly skipped: { plugin: string; reason: string }[];
}

/** Ordinal comparison over dot-separated numeric versions. Anything
 *  non-numeric compares as 0 rather than throwing: an unexpected version string
 *  must not be able to crash a release tool mid-convergence. */
export function compareVersions(left: string, right: string): number {
  const a = left.split(".").map((part) => Number.parseInt(part, 10) || 0);
  const b = right.split(".").map((part) => Number.parseInt(part, 10) || 0);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const diff = (a[index] ?? 0) - (b[index] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

/** The newest `ready` release per Plugin. A release in any other state is not a
 *  target: the Catalog is the authority on what may be installed. */
export function latestReadyReleases(
  releases: readonly CatalogRelease[],
): Map<string, CatalogRelease> {
  const latest = new Map<string, CatalogRelease>();
  for (const release of releases) {
    if (release.release_state !== "ready") continue;
    const current = latest.get(release.plugin_id);
    if (
      !current ||
      compareVersions(release.plugin_version, current.plugin_version) > 0
    ) latest.set(release.plugin_id, release);
  }
  return latest;
}

export function operationId(
  machine: string,
  plugin: string,
  version: string,
): string {
  return `${machine}-${plugin}-${version.replaceAll(".", "-")}-converge`;
}

export function planConvergence(
  machine: string,
  releases: readonly CatalogRelease[],
  installed: readonly InstalledPlugin[],
  only?: readonly string[],
): ConvergePlan {
  const latest = latestReadyReleases(releases);
  const steps: ConvergeStep[] = [];
  const skipped: { plugin: string; reason: string }[] = [];
  for (const plugin of installed) {
    if (only && only.length > 0 && !only.includes(plugin.plugin_id)) continue;
    const target = latest.get(plugin.plugin_id);
    if (!target) {
      skipped.push({
        plugin: plugin.plugin_id,
        reason: "no ready release in the Catalog",
      });
      continue;
    }
    const delta = compareVersions(plugin.plugin_version, target.plugin_version);
    if (delta === 0) continue;
    if (delta > 0) {
      // Installed ahead of the Catalog is a real condition (a release was
      // withdrawn, or this Machine was installed from elsewhere). Downgrading
      // is never implied by "converge".
      skipped.push({
        plugin: plugin.plugin_id,
        reason:
          `installed ${plugin.plugin_version} is ahead of Catalog ${target.plugin_version}`,
      });
      continue;
    }
    if (plugin.active_session_leases > 0) {
      skipped.push({
        plugin: plugin.plugin_id,
        reason: `holds ${plugin.active_session_leases} active session lease(s)`,
      });
      continue;
    }
    steps.push({
      plugin: plugin.plugin_id,
      from: plugin.plugin_version,
      to: target.plugin_version,
      digest: target.artifact_digest,
      operationId: operationId(
        machine,
        plugin.plugin_id,
        target.plugin_version,
      ),
    });
  }
  return { machine, steps, skipped };
}

async function operatorJson(
  cowboy: string,
  args: readonly string[],
): Promise<unknown> {
  const output = await new Deno.Command(cowboy, {
    args: [...args],
    stdout: "piped",
    stderr: "piped",
  }).output();
  const text = new TextDecoder().decode(output.stdout);
  const start = text.indexOf("{");
  if (start < 0) {
    throw new Error(
      `${args.join(" ")} produced no JSON: ${
        new TextDecoder().decode(output.stderr).slice(0, 300)
      }`,
    );
  }
  return JSON.parse(text.slice(start));
}

function data(value: unknown): any {
  return (value as { data: unknown }).data;
}

if (import.meta.main) {
  const args = [...Deno.args];
  const apply = args.includes("--apply");
  const cowboyAt = args.indexOf("--cowboy");
  const cowboy = cowboyAt >= 0 ? args[cowboyAt + 1] ?? "cowboy" : "cowboy";
  const only: string[] = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--plugin") {
      const value = args[index + 1];
      if (value) only.push(value);
    }
  }
  const machine = args.find((value) => !value.startsWith("--")) ?? "";
  if (!machine) {
    console.error(
      "usage: converge-machine.ts <machine> [--apply] [--plugin id]... [--cowboy path]",
    );
    Deno.exit(2);
  }
  const catalog = data(await operatorJson(cowboy, ["operator", "catalog"]));
  const inventory = data(
    await operatorJson(cowboy, ["operator", "inspect", "--machine", machine]),
  );
  const plan = planConvergence(
    machine,
    catalog.plugins ?? [],
    inventory ?? [],
    only,
  );
  const results: Record<string, unknown>[] = [];
  for (const step of plan.steps) {
    if (!apply) {
      results.push({ ...step, applied: false, reason: "dry run" });
      continue;
    }
    const response = await operatorJson(cowboy, [
      "operator",
      "upgrade",
      "--machine",
      machine,
      "--plugin",
      step.plugin,
      "--version",
      step.to,
      "--digest",
      step.digest,
      "--operation-id",
      step.operationId,
    ]) as { http_status: number };
    results.push({ ...step, applied: true, http_status: response.http_status });
  }
  // Re-read the inventory rather than trusting the submissions: an applied
  // receipt and an installed generation are different facts.
  const after = apply
    ? data(
      await operatorJson(cowboy, ["operator", "inspect", "--machine", machine]),
    )
    : inventory;
  const pending = planConvergence(
    machine,
    catalog.plugins ?? [],
    after ?? [],
    only,
  );
  console.log(JSON.stringify(
    {
      machine,
      applied: apply,
      steps: results,
      skipped: plan.skipped,
      remaining: pending.steps.map((step) =>
        `${step.plugin} ${step.from}→${step.to}`
      ),
    },
    null,
    1,
  ));
  Deno.exit(apply && pending.steps.length > 0 ? 1 : 0);
}
