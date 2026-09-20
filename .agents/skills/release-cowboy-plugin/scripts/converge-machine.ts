#!/usr/bin/env -S deno run --allow-run
/**
 * Bring one or more Machines' installed Plugins up to the Catalog's newest
 * ready releases, one signed upgrade at a time.
 *
 * The planning, ordering, verification and receipts all live in the shipped
 * Controller CLI (`cowboy operator converge`), so a release agent and an
 * unattended timer run the exact same implementation; this script only shapes
 * the invocation. Keeping one planner matters because the digest is the whole
 * trust claim: two planners are two chances for a fleet to be converged to
 * different bytes.
 *
 * It fails closed there, not here: a dry run is the default, a Plugin holding
 * an active session lease is reported rather than recycled under a live
 * worker, a release that is not `ready` or that does not declare the Machine's
 * platform is not a target, an installed version ahead of the Catalog is never
 * downgraded, and a Machine that does not converge stops the rollout.
 *
 * It does NOT publish, sign, or decide what "latest" should be: it converges
 * to what the Catalog already advertises. Publication stays the separate,
 * explicitly authorized act described in SKILL.md.
 */

export interface ConvergeInvocation {
  readonly machines: readonly string[];
  readonly plugins: readonly string[];
  readonly apply: boolean;
}

/** The exact `cowboy` argument vector for one convergence run. The requested
 *  Machine order is preserved: it is the rollout order, and its first entry is
 *  the canary. */
export function convergeArguments(
  invocation: ConvergeInvocation,
): string[] {
  const args = ["operator", "converge"];
  for (const machine of invocation.machines) args.push("--machine", machine);
  for (const plugin of invocation.plugins) args.push("--plugin", plugin);
  if (invocation.apply) args.push("--apply");
  return args;
}

/** Parse this script's own arguments. Unknown flags are refused rather than
 *  forwarded: a typo must not silently become a wider run. */
export function parseInvocation(
  argv: readonly string[],
): { invocation: ConvergeInvocation; cowboy: string } | { error: string } {
  const machines: string[] = [];
  const plugins: string[] = [];
  let apply = false;
  let cowboy = "cowboy";
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--apply") {
      apply = true;
      continue;
    }
    if (
      argument === "--plugin" || argument === "--machine" ||
      argument === "--cowboy"
    ) {
      const value = argv[index + 1];
      if (value === undefined || value.startsWith("--")) {
        return { error: `${argument} needs a value` };
      }
      index += 1;
      if (argument === "--plugin") plugins.push(value);
      else if (argument === "--machine") machines.push(value);
      else cowboy = value;
      continue;
    }
    if (argument !== undefined && argument.startsWith("--")) {
      return { error: `unknown option ${argument}` };
    }
    if (argument !== undefined) machines.push(argument);
  }
  return { invocation: { machines, plugins, apply }, cowboy };
}

if (import.meta.main) {
  const parsed = parseInvocation(Deno.args);
  if ("error" in parsed) {
    console.error(`${parsed.error}
usage: converge-machine.ts [<machine>...] [--apply] [--plugin id]... [--cowboy path]`);
    Deno.exit(2);
  }
  const { code } = await new Deno.Command(parsed.cowboy, {
    args: convergeArguments(parsed.invocation),
    stdout: "inherit",
    stderr: "inherit",
  }).output();
  Deno.exit(code);
}
