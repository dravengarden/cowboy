// Third-party sources: a pinned upstream release plus an ordered series of
// Cowboy-owned deltas, one patch file per feature. See docs/third-party.md.
//
// The series is the storage format. Development happens in a real Git work
// tree that `tools/third-party.ts checkout` expands from it, and `export`
// writes the work tree back. Build scripts call `prepareSource` to obtain the
// exact patched upstream tree from the pinned archive.

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readdir, readFile, rename, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { Command } from "./command.ts";

export const REPOSITORY = resolve(import.meta.dir, "../..");
export const MANIFEST = "upstream.json";
export const PATCHES = "patches";

export type ConsumerMode = "source" | "registry";

export interface Consumer {
  /** Repository path of the Plugin directory whose provider.json pins the package. */
  path: string;
  /** `source` builds the patched tree; `registry` uses the unmodified upstream release. */
  mode: ConsumerMode;
}

export interface Upstream {
  schema_version: 1;
  name: string;
  /** GitHub repository, `https://github.com/<owner>/<repo>`. */
  repository: string;
  /** The release version consumers pin, without a tag prefix. */
  version: string;
  tag: string;
  commit: string;
  /** sha256 of the GitHub source archive of `commit`. */
  archive_sha256: string;
  /** Files inside the source tree whose exact bytes the build relies on. */
  pinned_files: Record<string, string>;
  /** Regular expression selecting upstream release tags for `status`. */
  release_tag_pattern: string;
  consumers: Consumer[];
}

/** Upstream state of a delta. A merged delta must be dropped at the next upgrade. */
export const UPSTREAM_STATES = /^(not-proposed|cowboy-specific|(proposed|merged) https:\/\/\S+)$/;
const DELTA_ID = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const PATCH_FILE = /^(\d{4})-([a-z0-9-]+)\.patch$/;
const TRAILERS = ["Cowboy-Delta", "Upstream", "Drop-When", "Tests"] as const;

export interface Delta {
  file: string;
  index: number;
  id: string;
  subject: string;
  upstream: string;
  dropWhen: string;
  tests: string[];
  /** Paths the patch creates or modifies. */
  paths: string[];
  sha256: string;
}

export function packageRoot(name: string, root = REPOSITORY): string {
  if (!DELTA_ID.test(name)) throw new Error(`invalid third-party name: ${name}`);
  return join(root, "third_party", name);
}

export async function listPackages(root = REPOSITORY): Promise<string[]> {
  const entries = await readdir(join(root, "third_party"), { withFileTypes: true });
  return entries
    .filter(
      (entry) => entry.isDirectory() && existsSync(join(root, "third_party", entry.name, MANIFEST)),
    )
    .map((entry) => entry.name)
    .sort();
}

export async function readUpstream(name: string, root = REPOSITORY): Promise<Upstream> {
  const value = JSON.parse(await readFile(join(packageRoot(name, root), MANIFEST), "utf8"));
  const problems = validateUpstream(name, value);
  if (problems.length) throw new Error(`${name}/${MANIFEST}: ${problems.join("; ")}`);
  return value as Upstream;
}

export function validateUpstream(name: string, value: Record<string, unknown>): string[] {
  const problems: string[] = [];
  const text = (key: string, pattern: RegExp) => {
    if (typeof value[key] !== "string" || !pattern.test(value[key] as string)) {
      problems.push(`${key} must match ${pattern}`);
    }
  };
  if (value.schema_version !== 1) problems.push("schema_version must be 1");
  if (value.name !== name) problems.push(`name must be ${name}`);
  text("repository", /^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/);
  text("version", /^\d+\.\d+\.\d+([-.+][0-9A-Za-z.-]+)?$/);
  text("tag", /^[\w.+-]+$/);
  text("commit", /^[a-f0-9]{40}$/);
  text("archive_sha256", /^[a-f0-9]{64}$/);
  text("release_tag_pattern", /^\^.+\$$/);
  const pinned = value.pinned_files;
  if (pinned === null || typeof pinned !== "object" || Array.isArray(pinned)) {
    problems.push("pinned_files must be an object");
  } else {
    for (const [path, digest] of Object.entries(pinned)) {
      if (path.startsWith("/") || path.split("/").includes(".."))
        problems.push(`invalid pinned path ${path}`);
      if (typeof digest !== "string" || !/^[a-f0-9]{64}$/.test(digest))
        problems.push(`invalid digest for ${path}`);
    }
  }
  const consumers = value.consumers;
  if (!Array.isArray(consumers) || consumers.length === 0) {
    problems.push("consumers must list at least one Plugin");
  } else {
    for (const consumer of consumers as Consumer[]) {
      if (typeof consumer?.path !== "string" || !["source", "registry"].includes(consumer?.mode)) {
        problems.push(`invalid consumer ${JSON.stringify(consumer)}`);
      }
    }
  }
  return problems;
}

/** `https://codeload.github.com/<owner>/<repo>/tar.gz/<commit>` */
export function archiveUrl(upstream: Pick<Upstream, "repository" | "commit">): string {
  const path = new URL(upstream.repository).pathname;
  return `https://codeload.github.com${path}/tar.gz/${upstream.commit}`;
}

export async function readSeries(name: string, root = REPOSITORY): Promise<Delta[]> {
  const directory = join(packageRoot(name, root), PATCHES);
  if (!existsSync(directory)) return [];
  const files = (await readdir(directory)).filter((file) => !file.startsWith(".")).sort();
  const deltas: Delta[] = [];
  for (const file of files) {
    deltas.push(parsePatch(file, await readFile(join(directory, file), "utf8")));
  }
  return deltas;
}

/** Parse one `git format-patch` file and its Cowboy trailers. */
export function parsePatch(file: string, text: string): Delta {
  const named = PATCH_FILE.exec(file);
  if (!named) throw new Error(`${file}: patch files are named NNNN-<delta-id>.patch`);
  const headerEnd = text.indexOf("\n\n");
  const messageEnd = Math.min(
    ...["\n---\n", "\ndiff --git "].map((marker) => text.indexOf(marker)).filter((at) => at >= 0),
  );
  if (
    !text.startsWith("From ") ||
    headerEnd < 0 ||
    !Number.isFinite(messageEnd) ||
    messageEnd < headerEnd
  ) {
    throw new Error(`${file}: not a git format-patch file`);
  }
  const subjectMatch = /^Subject: (?:\[PATCH[^\]]*\] )?((?:.*)(?:\n .*)*)$/m.exec(
    text.slice(0, headerEnd),
  );
  const subject = (subjectMatch?.[1] ?? "").replace(/\n /g, " ");
  const message = text.slice(headerEnd + 2, messageEnd);
  const trailers = new Map<string, string>();
  for (const line of message.split("\n")) {
    const match = /^([A-Za-z-]+): (.+)$/.exec(line);
    if (match && (TRAILERS as readonly string[]).includes(match[1])) {
      if (trailers.has(match[1])) throw new Error(`${file}: duplicate ${match[1]} trailer`);
      trailers.set(match[1], match[2].trim());
    }
  }
  for (const trailer of TRAILERS) {
    if (!trailers.get(trailer)) throw new Error(`${file}: missing ${trailer} trailer`);
  }
  const id = trailers.get("Cowboy-Delta")!;
  if (id !== named[2]) throw new Error(`${file}: Cowboy-Delta ${id} does not match the file name`);
  const upstream = trailers.get("Upstream")!;
  if (!UPSTREAM_STATES.test(upstream)) throw new Error(`${file}: invalid Upstream: ${upstream}`);
  const paths = [...text.matchAll(/^\+\+\+ b\/(.+)$/gm)].map((match) => match[1]);
  if (paths.length === 0) throw new Error(`${file}: the patch changes no file`);
  const tests = trailers.get("Tests")!.split(/\s+/);
  for (const test of tests) {
    if (!paths.includes(test))
      throw new Error(`${file}: test ${test} is not changed by this delta`);
  }
  return {
    file,
    index: Number(named[1]),
    id,
    subject,
    upstream,
    dropWhen: trailers.get("Drop-When")!,
    tests,
    paths,
    sha256: createHash("sha256").update(text).digest("hex"),
  };
}

/** Structural problems of one package; empty when it is consistent. Offline. */
export async function checkPackage(name: string, root = REPOSITORY): Promise<string[]> {
  const problems: string[] = [];
  let upstream: Upstream;
  try {
    upstream = await readUpstream(name, root);
  } catch (error) {
    return [(error as Error).message];
  }
  let series: Delta[] = [];
  try {
    series = await readSeries(name, root);
  } catch (error) {
    problems.push((error as Error).message);
  }
  series.forEach((delta, position) => {
    if (delta.index !== position + 1)
      problems.push(`${delta.file}: series numbering must be contiguous from 0001`);
  });
  const ids = series.map((delta) => delta.id);
  if (new Set(ids).size !== ids.length) problems.push("delta IDs must be unique");
  if (series.length > 0 && !upstream.consumers.some((consumer) => consumer.mode === "source")) {
    problems.push("a package with deltas needs a source consumer");
  }
  const declared = new Set(upstream.consumers.map((consumer) => consumer.path));
  for (const plugin of await readdir(join(root, "plugins"))) {
    const manifest = join(root, "plugins", plugin, "provider.json");
    if (!existsSync(manifest)) continue;
    const dependencies: Array<{ id: string; version: string }> =
      JSON.parse(await readFile(manifest, "utf8")).runtime?.dependencies ?? [];
    const dependency = dependencies.find((value) => value.id === name);
    const path = `plugins/${plugin}`;
    if (!dependency) {
      if (declared.has(path)) problems.push(`${path} does not pin ${name}`);
      continue;
    }
    if (!declared.has(path)) problems.push(`${path} pins ${name} but is not a declared consumer`);
    if (dependency.version !== upstream.version) {
      problems.push(
        `${path} pins ${name} ${dependency.version}, upstream.json is ${upstream.version}`,
      );
    }
  }
  return problems;
}

export async function sha256File(path: string): Promise<string> {
  return createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
}

export async function run(
  command: string,
  args: string[],
  cwd?: string,
  env?: Record<string, string>,
): Promise<string> {
  const output = await new Command(command, {
    args,
    ...(cwd === undefined ? {} : { cwd }),
    ...(env === undefined ? {} : { env }),
  }).output();
  if (!output.success) {
    const stderr = new TextDecoder().decode(output.stderr).trim();
    throw new Error(`${command} ${args.join(" ")} failed (${output.code}): ${stderr}`);
  }
  return new TextDecoder().decode(output.stdout);
}

/** Download (once) and verify the pinned source archive. */
export async function fetchArchive(upstream: Upstream, cache: string): Promise<string> {
  await mkdir(cache, { recursive: true });
  const archive = join(cache, `${upstream.name}-${upstream.archive_sha256}.tar.gz`);
  if (!existsSync(archive)) {
    const partial = `${archive}.${process.pid}.partial`;
    await run("curl", [
      "--fail",
      "--silent",
      "--show-error",
      "--location",
      "--output",
      partial,
      archiveUrl(upstream),
    ]);
    if ((await sha256File(partial)) !== upstream.archive_sha256) {
      await rm(partial, { force: true });
      throw new Error(`${upstream.name}: source archive digest mismatch`);
    }
    await rename(partial, archive);
  }
  if ((await sha256File(archive)) !== upstream.archive_sha256) {
    throw new Error(`${upstream.name}: cached source archive digest mismatch`);
  }
  return archive;
}

/**
 * Extract the pinned upstream source into the empty directory `destination`,
 * verify its pinned files and apply every delta at fuzz 0, in order.
 */
export async function prepareSource(
  name: string,
  destination: string,
  cache: string,
  root = REPOSITORY,
): Promise<{ upstream: Upstream; series: Delta[] }> {
  const upstream = await readUpstream(name, root);
  const series = await readSeries(name, root);
  const archive = await fetchArchive(upstream, cache);
  await mkdir(destination, { recursive: true });
  await run("tar", ["-xzf", archive, "--strip-components=1", "-C", destination]);
  for (const [path, digest] of Object.entries(upstream.pinned_files)) {
    if ((await sha256File(join(destination, path))) !== digest) {
      throw new Error(`${name}: pinned file ${path} does not match upstream.json`);
    }
  }
  const patches = join(packageRoot(name, root), PATCHES);
  for (const delta of series) {
    // --batch alone silently reverses a patch that looks already applied;
    // --forward makes that a failure, like any hunk that misses at fuzz 0.
    await run(
      "patch",
      [
        "--forward",
        "--batch",
        "--fuzz=0",
        "--no-backup-if-mismatch",
        "--reject-file=-",
        "-p1",
        "-i",
        join(patches, delta.file),
      ],
      destination,
    );
  }
  return { upstream, series };
}

/** A shared bare mirror of the upstream repository, outside every checkout. */
export async function mirror(upstream: Upstream): Promise<string> {
  const cache = process.env.XDG_CACHE_HOME || join(homedir(), ".cache");
  const path = join(cache, "cowboy", "third-party", `${upstream.name}.git`);
  if (!existsSync(path)) {
    await mkdir(join(path, ".."), { recursive: true });
    await run("git", ["clone", "--quiet", "--mirror", `${upstream.repository}.git`, path]);
  } else {
    await run("git", ["-C", path, "remote", "update", "--prune"]);
  }
  return path;
}
