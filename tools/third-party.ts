// Third-party upstream + delta-series workflow. See docs/third-party.md.
//
//   bun tools/third-party.ts list
//   bun tools/third-party.ts check [<name>...] [--apply]
//   bun tools/third-party.ts status [<name>...]
//   bun tools/third-party.ts checkout <name> [--force]
//   bun tools/third-party.ts export <name>
//   bun tools/third-party.ts drop <name> <delta-id>
//   bun tools/third-party.ts upgrade <name> <tag>
//   bun tools/third-party.ts diff <name> [--base <git-rev>]
//   bun tools/third-party.ts prepare <name> <empty-destination> <archive-cache>
//
// `prepare` is the build interface: it writes the patched source tree and
// prints {upstream, series} as JSON. Plugin builders run it as a process
// because a Plugin package may not import repository modules.
//
// Run from the repository root through the pinned shell.

import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  archiveUrl,
  checkPackage,
  listPackages,
  MANIFEST,
  mirror,
  PATCHES,
  packageRoot,
  parsePatch,
  prepareSource,
  readSeries,
  readUpstream,
  REPOSITORY,
  run,
  sha256File,
  type Upstream,
  validateUpstream,
} from "./lib/third-party.ts";

const BASE_REF = "refs/cowboy/base";
const BRANCH = "cowboy";
const STATE = "cowboy-third-party.json";
const COMMITTER = {
  GIT_COMMITTER_NAME: "Cowboy third-party",
  GIT_COMMITTER_EMAIL: "third-party@cowboy.invalid",
};

interface WorkState {
  name: string;
  base: string;
  tag: string;
  /** Present between `upgrade` and the `export` that records it. */
  upgrade?: { from: { tag: string; commit: string }; to: { tag: string; commit: string } };
}

export function workTree(name: string): string {
  packageRoot(name);
  return join(REPOSITORY, "dist", "third-party", name);
}

const git = (work: string, ...args: string[]) =>
  run("git", ["-C", work, ...args], undefined, COMMITTER);

async function readState(work: string): Promise<WorkState> {
  const path = join(work, ".git", STATE);
  if (!existsSync(path)) throw new Error(`no work tree at ${work}; run checkout first`);
  return JSON.parse(await readFile(path, "utf8"));
}

/** The upstream commit the work tree's series currently sits on. */
function seriesBase(state: WorkState): string {
  return state.upgrade?.to.commit ?? state.base;
}

async function writeState(work: string, state: WorkState): Promise<void> {
  await writeFile(join(work, ".git", STATE), JSON.stringify(state, null, 2) + "\n");
}

async function requireClean(work: string): Promise<void> {
  for (const marker of ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD"]) {
    if (existsSync(join(work, ".git", marker))) {
      throw new Error(`${work} has a ${marker} in progress; finish or abort it first`);
    }
  }
  if ((await git(work, "status", "--porcelain", "--untracked-files=normal")).trim() !== "") {
    throw new Error(`${work} has uncommitted changes; commit them as a delta or a fixup first`);
  }
}

/** Expand a series onto `base` in a work tree and return the resulting head. */
async function applySeries(work: string, base: string, patchFiles: string[]): Promise<string> {
  await git(work, "switch", "--quiet", "--force-create", BRANCH, base);
  await git(work, "update-ref", BASE_REF, base);
  if (patchFiles.length > 0) {
    await git(work, "am", "--quiet", "--committer-date-is-author-date", ...patchFiles);
  }
  return (await git(work, "rev-parse", "HEAD")).trim();
}

/** Render `base..HEAD` as named series files. Every commit must carry the Cowboy trailers. */
async function renderSeries(work: string, base: string): Promise<Map<string, string>> {
  const output = await mkdtemp(join(tmpdir(), "third-party-export-"));
  try {
    // Ignore user and system Git configuration (format.thread, coverLetter,
    // diff.algorithm, renames...) so the same commits always render the same bytes.
    await run(
      "git",
      [
        "-C",
        work,
        "format-patch",
        "--quiet",
        "--no-thread",
        "--no-renames",
        "--diff-algorithm=myers",
        "--zero-commit",
        "--no-signature",
        "--no-stat",
        "--full-index",
        "--no-numbered",
        "-o",
        output,
        `${base}..HEAD`,
      ],
      undefined,
      { ...COMMITTER, GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1" },
    );
    const rendered = new Map<string, string>();
    const files = (await readdir(output)).sort();
    for (const [position, file] of files.entries()) {
      const text = await readFile(join(output, file), "utf8");
      const id = /^Cowboy-Delta: (.+)$/m.exec(text)?.[1]?.trim();
      if (!id) throw new Error(`commit ${position + 1} (${file}) has no Cowboy-Delta trailer`);
      const name = `${String(position + 1).padStart(4, "0")}-${id}.patch`;
      parsePatch(name, text);
      if ([...rendered.keys()].some((existing) => existing.slice(5) === name.slice(5))) {
        throw new Error(`delta ${id} appears twice; squash it or use a fixup! commit`);
      }
      rendered.set(name, text);
    }
    return rendered;
  } finally {
    await rm(output, { recursive: true, force: true });
  }
}

async function committedSeriesMatches(name: string, work: string, base: string): Promise<boolean> {
  const rendered = await renderSeries(work, base);
  const directory = join(packageRoot(name), PATCHES);
  const committed = existsSync(directory) ? (await readdir(directory)).sort() : [];
  if (committed.length !== rendered.size) return false;
  for (const file of committed) {
    if (rendered.get(file) !== (await readFile(join(directory, file), "utf8"))) return false;
  }
  return true;
}

async function checkout(name: string, force: boolean): Promise<void> {
  const upstream = await readUpstream(name);
  const work = workTree(name);
  if (existsSync(work)) {
    if (!force) throw new Error(`${work} exists; export it, or pass --force to discard it`);
    await rm(work, { recursive: true, force: true });
  }
  const source = await mirror(upstream);
  await mkdir(join(work, ".."), { recursive: true });
  await run("git", ["clone", "--quiet", "--no-checkout", source, work]);
  const series = (await readSeries(name)).map((delta) =>
    join(packageRoot(name), PATCHES, delta.file),
  );
  await applySeries(work, upstream.commit, series);
  await writeState(work, { name, base: upstream.commit, tag: upstream.tag });
  console.log(`${work}: ${upstream.tag} + ${series.length} deltas on branch ${BRANCH}`);
  console.log("Commit a new delta with a Cowboy-Delta/Upstream/Drop-When/Tests trailer block,");
  console.log("amend an existing one with `git commit --fixup`, then run export.");
}

async function exportSeries(name: string): Promise<void> {
  const work = workTree(name);
  const state = await readState(work);
  await requireClean(work);
  await requireSeriesShape(work, state);
  const pending = (await git(work, "log", "--format=%s", `${seriesBase(state)}..HEAD`))
    .split("\n")
    .some((subject) => /^(fixup|squash|amend)! /.test(subject));
  if (pending) {
    await run(
      "git",
      ["-C", work, "rebase", "--quiet", "--interactive", "--autosquash", seriesBase(state)],
      undefined,
      { ...COMMITTER, GIT_SEQUENCE_EDITOR: "true", GIT_EDITOR: "true" },
    );
  }
  const rendered = await renderSeries(work, seriesBase(state));
  if (state.upgrade) await recordUpgrade(name, work, state);
  const directory = join(packageRoot(name), PATCHES);
  await rm(directory, { recursive: true, force: true });
  if (rendered.size > 0) await mkdir(directory);
  for (const [file, text] of rendered) await writeFile(join(directory, file), text);
  if (state.upgrade) {
    delete state.upgrade;
    await writeState(work, state);
  }
  const problems = await checkPackage(name);
  for (const problem of problems) console.error(`warning: ${problem}`);
  console.log(`${name}: exported ${rendered.size} deltas to third_party/${name}/${PATCHES}`);
}

/**
 * The work tree must be on its branch with the series base as an ancestor. An
 * aborted upgrade rebase leaves the old base in history: reject it instead of
 * recording the new upstream pin for commits that were never rebased.
 */
async function requireSeriesShape(work: string, state: WorkState): Promise<void> {
  const branch = (
    await git(work, "symbolic-ref", "--quiet", "--short", "HEAD").catch(() => "")
  ).trim();
  if (branch !== BRANCH) throw new Error(`${work} must be on branch ${BRANCH}`);
  const base = seriesBase(state);
  const onBase = await git(work, "merge-base", "--is-ancestor", base, "HEAD").then(
    () => true,
    () => false,
  );
  if (!onBase) {
    throw new Error(
      state.upgrade
        ? `the upgrade to ${state.upgrade.to.tag} was not completed (rebase aborted?); rerun checkout --force and upgrade`
        : `${work} no longer contains its base ${base}; rerun checkout --force`,
    );
  }
}

async function drop(name: string, id: string): Promise<void> {
  const work = workTree(name);
  const state = await readState(work);
  await requireClean(work);
  await requireSeriesShape(work, state);
  const commits = (
    await git(
      work,
      "log",
      "--reverse",
      "--format=%H %(trailers:key=Cowboy-Delta,valueonly,separator=)",
      `${seriesBase(state)}..HEAD`,
    )
  )
    .trim()
    .split("\n");
  const target = commits.find((line) => line.split(" ")[1] === id)?.split(" ")[0];
  if (!target) throw new Error(`no delta ${id} in ${work}`);
  await git(work, "rebase", "--quiet", "--onto", `${target}^`, target, BRANCH);
  console.log(`${name}: dropped ${id}; run export to write the series`);
}

async function upgrade(name: string, tag: string): Promise<void> {
  const upstream = await readUpstream(name);
  const work = workTree(name);
  const state = await readState(work);
  await requireClean(work);
  if (
    state.upgrade ||
    state.base !== upstream.commit ||
    !(await committedSeriesMatches(name, work, state.base))
  ) {
    throw new Error(`${work} differs from the committed series; export or checkout --force first`);
  }
  const source = await mirror(upstream);
  await git(work, "fetch", "--quiet", "--tags", source, "+refs/heads/*:refs/remotes/origin/*");
  const commit = (await git(work, "rev-parse", "--verify", `refs/tags/${tag}^{commit}`)).trim();
  await git(work, "tag", "--force", "cowboy/previous", "HEAD");
  state.upgrade = { from: { tag: upstream.tag, commit: upstream.commit }, to: { tag, commit } };
  await writeState(work, state);
  await writeFile(
    upgradeReportPath(name, tag),
    await upstreamChanges(work, upstream.commit, commit),
  );
  try {
    await git(work, "rebase", "--quiet", "--onto", commit, state.base, BRANCH);
  } catch (error) {
    console.error((error as Error).message);
    console.error(`Conflicts: resolve in ${work}, then \`git rebase --continue\`.`);
    console.error(
      "Upstream-first: `git rebase --skip` drops the delta being applied when upstream now covers it.",
    );
    console.error(`Then run: bun tools/third-party.ts export ${name}`);
    process.exitCode = 1;
    return;
  }
  console.log(
    `${name}: rebased onto ${tag}. Review ${upgradeReportPath(name, tag)} and decide each delta`,
  );
  console.log(
    `(drop covered ones with \`bun tools/third-party.ts drop ${name} <id>\`), then export.`,
  );
}

function upgradeReportPath(name: string, tag: string): string {
  return join(workTree(name), "..", `${name}-upgrade-${tag}.md`);
}

/** Upstream history of the range, and the part of it touching delta paths. */
async function upstreamChanges(work: string, from: string, to: string): Promise<string> {
  const paths = [
    ...new Set((await git(work, "diff", "--name-only", `${from}..HEAD`)).trim().split("\n")),
  ].filter((path) => path !== "");
  const all = await git(work, "log", "--no-merges", "--format=- %h %s", `${from}..${to}`);
  const touching = paths.length
    ? await git(work, "log", "--no-merges", "--format=- %h %s", `${from}..${to}`, "--", ...paths)
    : "";
  return [
    `# Upstream ${from.slice(0, 12)}..${to.slice(0, 12)}`,
    "",
    "## Changes touching files the deltas change (check each delta against these first)",
    "",
    touching || "(none)",
    "## All upstream changes",
    "",
    all || "(none)",
  ].join("\n");
}

async function recordUpgrade(name: string, work: string, state: WorkState): Promise<void> {
  const { from, to } = state.upgrade!;
  const upstream = await readUpstream(name);
  const next: Upstream = { ...upstream, tag: to.tag, commit: to.commit };
  const match = /^v?(\d+\.\d+\.\d+.*)$/.exec(to.tag);
  if (!match) throw new Error(`cannot derive a version from tag ${to.tag}; edit ${MANIFEST}`);
  next.version = match[1];
  const cache = await mkdtemp(join(tmpdir(), "third-party-archive-"));
  try {
    const archive = join(cache, "source.tar.gz");
    await run("curl", [
      "--fail",
      "--silent",
      "--show-error",
      "--location",
      "--output",
      archive,
      archiveUrl(next),
    ]);
    next.archive_sha256 = await sha256File(archive);
    const tree = join(cache, "tree");
    await mkdir(tree);
    await run("tar", ["-xzf", archive, "--strip-components=1", "-C", tree]);
    for (const path of Object.keys(upstream.pinned_files)) {
      next.pinned_files[path] = await sha256File(join(tree, path));
    }
  } finally {
    await rm(cache, { recursive: true, force: true });
  }
  // Everything that can fail runs before the manifest, base ref and state change.
  const head = (await git(work, "rev-parse", "HEAD")).trim();
  const previous = (await git(work, "rev-parse", "cowboy/previous")).trim();
  const rangeDiff = await compareSeries(work, from.commit, previous, to.commit, head);
  const report = upgradeReportPath(name, to.tag);
  const prior = existsSync(report) ? await readFile(report, "utf8") : "";
  await writeFile(report, `${prior}\n## Delta range-diff\n\n${rangeDiff}`);
  await writeFile(join(packageRoot(name), MANIFEST), JSON.stringify(next, null, 2) + "\n");
  await git(work, "update-ref", BASE_REF, to.commit);
  state.base = to.commit;
  state.tag = to.tag;
  console.log(
    `${name}: ${from.tag} -> ${to.tag}; consumers must now pin ${next.version}. Report: ${report}`,
  );
}

/** range-diff of two series; git refuses an empty range, so report additions or removals instead. */
async function compareSeries(
  work: string,
  oldBase: string,
  oldHead: string,
  newBase: string,
  newHead: string,
): Promise<string> {
  const oldEmpty = oldHead === oldBase;
  const newEmpty = newHead === newBase;
  if (oldEmpty && newEmpty) return "(no deltas on either side)\n";
  if (oldEmpty || newEmpty) {
    const [label, from, to] = oldEmpty
      ? ["added", newBase, newHead]
      : ["removed", oldBase, oldHead];
    return `# every delta ${label}\n${await git(work, "log", "--reverse", "--no-color", "--patch", `${from}..${to}`)}`;
  }
  return git(work, "range-diff", "--no-color", `${oldBase}..${oldHead}`, `${newBase}..${newHead}`);
}

async function status(names: string[]): Promise<void> {
  for (const name of names) {
    const upstream = await readUpstream(name);
    const source = await mirror(upstream);
    const pattern = new RegExp(upstream.release_tag_pattern);
    const tags = (await run("git", ["-C", source, "tag", "--sort=-v:refname"]))
      .trim()
      .split("\n")
      .filter((tag) => pattern.test(tag));
    const position = tags.indexOf(upstream.tag);
    const newer = position < 0 ? tags.slice(0, 5) : tags.slice(0, position);
    const ahead = (
      await run("git", ["-C", source, "rev-list", "--count", `${upstream.commit}..HEAD`])
    ).trim();
    console.log(
      `${name} ${upstream.tag}: ${newer.length ? `newer releases ${newer.join(", ")}` : "current"}; default branch +${ahead}`,
    );
    for (const delta of await readSeries(name)) {
      let note = delta.upstream;
      const url = /^(proposed|merged) (https:\/\/github\.com\/\S+\/pull\/\d+)$/.exec(
        delta.upstream,
      )?.[2];
      if (url && Bun.which("gh")) {
        note += ` [${(await run("gh", ["pr", "view", url, "--json", "state", "--jq", ".state"]).catch(() => "unknown")).trim()}]`;
      }
      console.log(`  ${delta.file}: ${note}`);
    }
  }
}

/** Code-level review of a series change: range-diff between the series at `base` and the working tree. */
async function diff(name: string, base: string): Promise<void> {
  const current = await readUpstream(name);
  const commit = (
    await run("git", [
      "-C",
      REPOSITORY,
      "rev-parse",
      "--verify",
      "--quiet",
      `${base}^{commit}`,
    ]).catch(() => {
      throw new Error(`--base ${base} is not a commit`);
    })
  ).trim();
  // A package absent at a valid `base` compares against an empty series on the current pin.
  const present = (
    await run("git", [
      "-C",
      REPOSITORY,
      "ls-tree",
      "--name-only",
      commit,
      `third_party/${name}/${MANIFEST}`,
    ])
  ).trim();
  const recorded = present
    ? await run("git", ["-C", REPOSITORY, "show", `${commit}:third_party/${name}/${MANIFEST}`])
    : null;
  const previous: Upstream = recorded === null ? current : JSON.parse(recorded);
  if (validateUpstream(name, previous as unknown as Record<string, unknown>).length) {
    throw new Error(`${base}:third_party/${name}/${MANIFEST} is invalid`);
  }
  const scratch = await mkdtemp(join(tmpdir(), "third-party-diff-"));
  try {
    const old = join(scratch, "old");
    await mkdir(old);
    const listed = !present
      ? ""
      : (
            await run("git", [
              "-C",
              REPOSITORY,
              "ls-tree",
              "--name-only",
              `${commit}:third_party/${name}/`,
            ])
          ).includes(`${PATCHES}\n`)
        ? await run("git", [
            "-C",
            REPOSITORY,
            "ls-tree",
            "--name-only",
            `${commit}:third_party/${name}/${PATCHES}`,
          ])
        : "";
    const oldFiles: string[] = [];
    for (const file of listed.trim().split("\n").filter(Boolean)) {
      const path = join(old, file);
      await writeFile(
        path,
        await run("git", [
          "-C",
          REPOSITORY,
          "show",
          `${commit}:third_party/${name}/${PATCHES}/${file}`,
        ]),
      );
      oldFiles.push(path);
    }
    const work = join(scratch, "work");
    await run("git", ["clone", "--quiet", "--no-checkout", await mirror(current), work]);
    const oldHead = await applySeries(work, previous.commit, oldFiles);
    const newFiles = (await readSeries(name)).map((delta) =>
      join(packageRoot(name), PATCHES, delta.file),
    );
    const newHead = await applySeries(work, current.commit, newFiles);
    process.stdout.write(
      await compareSeries(work, previous.commit, oldHead, current.commit, newHead),
    );
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

async function check(names: string[], apply: boolean): Promise<void> {
  let failed = false;
  for (const name of names) {
    const problems = await checkPackage(name);
    if (problems.length === 0 && apply) {
      const scratch = await mkdtemp(join(tmpdir(), "third-party-check-"));
      try {
        await prepareSource(
          name,
          join(scratch, "source"),
          join(REPOSITORY, "dist", "plugins", ".runtime-cache"),
        );
      } catch (error) {
        problems.push((error as Error).message);
      } finally {
        await rm(scratch, { recursive: true, force: true });
      }
    }
    for (const problem of problems) console.error(`${name}: ${problem}`);
    failed ||= problems.length > 0;
    if (problems.length === 0)
      console.log(`${name}: ok (${(await readSeries(name)).length} deltas)`);
  }
  if (failed) process.exit(1);
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  const flags = new Set(rest.filter((value) => value.startsWith("--")));
  const positional = rest.filter(
    (value, index) => !value.startsWith("--") && rest[index - 1] !== "--base",
  );
  const all = async () => (positional.length ? positional : await listPackages());
  const one = () => {
    if (!positional[0]) throw new Error(`${command} needs a package name`);
    return positional[0];
  };
  switch (command) {
    case "list":
      for (const name of await listPackages()) {
        const upstream = await readUpstream(name);
        console.log(`${name} ${upstream.tag} ${(await readSeries(name)).length} deltas`);
      }
      break;
    case "check":
      await check(await all(), flags.has("--apply"));
      break;
    case "status":
      await status(await all());
      break;
    case "checkout":
      await checkout(one(), flags.has("--force"));
      break;
    case "export":
      await exportSeries(one());
      break;
    case "drop":
      if (!positional[1]) throw new Error("drop needs a delta ID");
      await drop(one(), positional[1]);
      break;
    case "upgrade":
      if (!positional[1]) throw new Error("upgrade needs an upstream tag");
      await upgrade(one(), positional[1]);
      break;
    case "prepare": {
      if (!positional[1] || !positional[2])
        throw new Error("prepare needs a destination and a cache");
      console.log(JSON.stringify(await prepareSource(one(), positional[1], positional[2])));
      break;
    }
    case "diff": {
      const at = rest.indexOf("--base");
      await diff(one(), at >= 0 && rest[at + 1] ? rest[at + 1] : "HEAD");
      break;
    }
    default:
      console.error(
        "usage: bun tools/third-party.ts list|check|status|checkout|export|drop|upgrade|diff (see docs/third-party.md)",
      );
      process.exit(2);
  }
}
