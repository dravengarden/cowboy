// Reproducible rebuild of the runtime's display server. Ubuntu's own
// tigervnc source package, with `native-touch.patch` appended to its quilt
// series, is built by its own `debian/rules build-arch` and stripped the way
// dh_strip strips it, inside the build root Ubuntu's official build used
// (the Launchpad .buildinfo's Installed-Build-Depends, every .deb by
// snapshot path, size and SHA-256 from `display-server.lock.json`), at that
// build's path, in an unprivileged bubblewrap sandbox without network. The
// result must hash exactly to the digest the lock records; `--record` writes
// that digest after a first build, and `--verify-stock` proves the build
// root by rebuilding without the patch to Ubuntu's shipped binary byte for
// byte. No Docker and no prebuilt binary are involved.
//
// Usage (build.ts calls buildDisplayServer):
//   bun plugins/machine-browser/runtime/display-server.ts [--record | --rebuild | --verify-stock]
import { createHash } from "node:crypto";
import { copyFile, mkdir, readdir, readFile, rm, stat, symlink, writeFile } from "node:fs/promises";
import { cpus } from "node:os";
import { join, resolve } from "node:path";
import { extractDeb } from "./extract.ts";
import { type Lock, type LockedPackage, downloadAll, run, sha256File } from "./lock.ts";

const RUNTIME_DIR = "plugins/machine-browser/runtime";
const LOCK_PATH = `${RUNTIME_DIR}/display-server.lock.json`;
const PATCH_PATH = `${RUNTIME_DIR}/native-touch.patch`;
const WORK_ROOT = "dist/plugins/machine-browser/display-server";
const BINARY_PACKAGE = "tigervnc-standalone-server";
/// debhelper's staging directory of the binary package, relative to the
/// source tree; the shipped Xtigervnc is the result inside it.
const PACKAGE_DIR = `debian/${BINARY_PACKAGE}`;
const BUILT_XVNC = `${PACKAGE_DIR}/usr/bin/Xtigervnc`;
/// Generic names maintainer scripts register with update-alternatives; each
/// must resolve to exactly one candidate in the build root.
const ALTERNATIVES: Array<[string, RegExp]> = [
  ["cc", /^gcc$/],
  ["c++", /^g\+\+$/],
  ["awk", /^mawk$/],
  ["automake", /^automake-[0-9.]+$/],
  ["aclocal", /^aclocal-[0-9.]+$/],
  ["yacc", /^bison\.yacc$/],
  ["which", /^which\.debianutils$/],
];

interface SourceFile {
  name: string;
  filename: string;
  size: number;
  sha256: string;
}

interface BuildRootPackage extends SourceFile {
  version: string;
  architecture: string;
}

export interface DisplayServerLock {
  schema_version: number;
  snapshot: string;
  archive: string;
  source: { package: string; version: string; archive: string; files: SourceFile[] };
  buildinfo: { url: string; size: number; sha256: string };
  patch: { file: string; sha256: string } | null;
  build_root: BuildRootPackage[];
  output: { path: string; size: number; sha256: string } | null;
}

export interface DisplayServer {
  /// The verified binary to install as `usr/bin/Xtigervnc`.
  path: string;
  sha256: string;
  size: number;
  rebuilt: boolean;
  source_version: string;
  patch_sha256: string;
  snapshot: string;
  buildinfo_sha256: string;
}

interface BuildInfo {
  path: string;
  version: string;
  environment: Map<string, string>;
  installed: Map<string, string>;
}

/// The version line the patch was reviewed for; an Ubuntu tigervnc bump
/// refuses to build until the patch is reviewed again and this line moves.
export function reviewedVersion(patch: string): string {
  const match = /^Cowboy-Reviewed-For: tigervnc (\S+)$/m.exec(patch);
  if (!match) throw new Error("native-touch.patch has no Cowboy-Reviewed-For header");
  return match[1];
}

export function parseBuildInfo(text: string): BuildInfo {
  const field = (name: string) => new RegExp(`^${name}: (.*)$`, "m").exec(text)?.[1];
  const block = (name: string) =>
    (new RegExp(`^${name}:\\n((?: .*\\n?)*)`, "m").exec(text)?.[1] ?? "")
      .split("\n")
      .map((line) => line.trim().replace(/,$/, ""))
      .filter(Boolean);
  const installed = new Map<string, string>();
  for (const entry of block("Installed-Build-Depends")) {
    const match = /^(\S+) \(= (\S+)\)$/.exec(entry);
    if (!match) throw new Error(`unexpected Installed-Build-Depends entry ${entry}`);
    installed.set(match[1], match[2]);
  }
  const environment = new Map<string, string>();
  for (const entry of block("Environment")) {
    const match = /^([A-Z_]+)="(.*)"$/.exec(entry);
    if (match) environment.set(match[1], match[2]);
  }
  const path = field("Build-Path");
  const version = field("Version");
  if (!path?.startsWith("/build/") || path.includes("..") || !version) {
    throw new Error("the .buildinfo names no usable Build-Path or Version");
  }
  return { path, version, environment, installed };
}

export function checkDisplayServerLock(
  lock: DisplayServerLock,
  runtime: Lock,
  patch: string,
  buildinfo?: BuildInfo,
) {
  if (lock.schema_version !== 1 || lock.source.package !== "tigervnc") {
    throw new Error("Unsupported display-server.lock.json");
  }
  for (const [archive, snapshot] of [
    [lock.archive, lock.snapshot],
    [lock.source.archive, lock.source.archive.split("/").pop() ?? ""],
  ]) {
    if (
      !/^[0-9]{8}T[0-9]{6}Z$/.test(snapshot) ||
      archive !== `https://snapshot.ubuntu.com/ubuntu/${snapshot}`
    ) {
      throw new Error("every input must come from a snapshot.ubuntu.com instant");
    }
  }
  const binary = runtime.packages.find((entry) => entry.name === BINARY_PACKAGE);
  if (!binary || binary.version !== lock.source.version) {
    throw new Error(
      `display-server.lock.json builds tigervnc ${lock.source.version} but the runtime ships ${binary?.version}; rerun display-server-lock.sh`,
    );
  }
  const reviewed = reviewedVersion(patch);
  if (reviewed !== lock.source.version) {
    throw new Error(
      `native-touch.patch was reviewed for tigervnc ${reviewed}, not ${lock.source.version}: review the patch against the new source, then move its Cowboy-Reviewed-For line`,
    );
  }
  const seen = new Set<string>();
  for (const entry of [...lock.source.files, ...lock.build_root]) {
    if (!/^[0-9a-f]{64}$/.test(entry.sha256)) throw new Error(`${entry.name}: invalid sha256`);
    if (!/^pool\/[A-Za-z0-9+._~/-]+$/.test(entry.filename) || entry.filename.includes("..")) {
      throw new Error(`${entry.name}: invalid pool path`);
    }
    if (seen.has(entry.sha256)) throw new Error(`${entry.name}: duplicate digest`);
    seen.add(entry.sha256);
  }
  if (buildinfo) {
    if (buildinfo.version !== lock.source.version) {
      throw new Error(`the .buildinfo is for ${buildinfo.version}, not ${lock.source.version}`);
    }
    const locked = new Map(lock.build_root.map((entry) => [entry.name, entry.version]));
    const differs =
      locked.size !== buildinfo.installed.size ||
      [...buildinfo.installed].some(([name, version]) => locked.get(name) !== version);
    if (differs) throw new Error("the build root is not the .buildinfo's Installed-Build-Depends");
  }
}

function pinned(lock: DisplayServerLock, entry: SourceFile | BuildRootPackage): LockedPackage {
  const base = entry.filename.split("/").pop()!;
  const isPackage = "version" in entry;
  return {
    name: entry.name,
    version: isPackage ? entry.version : lock.source.version,
    size: entry.size,
    sha256: entry.sha256,
    urls: [
      `${isPackage ? lock.archive : lock.source.archive}/${entry.filename}`,
      isPackage
        ? `https://launchpad.net/ubuntu/+archive/primary/+files/${base}`
        : `https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/${lock.source.package}/${lock.source.version}/${base}`,
    ],
  };
}

/// The source files the .dsc names must be exactly the locked ones.
function checkDsc(dsc: string, files: SourceFile[]) {
  const section = /^Checksums-Sha256:\n((?: .+\n?)+)/m.exec(dsc);
  if (!section) throw new Error("the .dsc has no Checksums-Sha256");
  for (const line of section[1].trim().split("\n")) {
    const [digest, size, name] = line.trim().split(/\s+/);
    const file = files.find((entry) => entry.name === name);
    if (!file || file.sha256 !== digest || String(file.size) !== size) {
      throw new Error(`${name}: the .dsc and the lock disagree`);
    }
  }
}

async function fetchBuildInfo(lock: DisplayServerLock, cacheDir: string): Promise<BuildInfo> {
  const [path] = (
    await downloadAll(
      [
        {
          name: "buildinfo",
          version: lock.source.version,
          size: lock.buildinfo.size,
          sha256: lock.buildinfo.sha256,
          urls: [lock.buildinfo.url],
        },
      ],
      cacheDir,
    )
  ).values();
  return parseBuildInfo(await readFile(path, "utf8"));
}

function inputsDigest(lock: DisplayServerLock, patch: string): string {
  const hash = createHash("sha256");
  hash.update(JSON.stringify({ ...lock, patch: null, output: null }));
  hash.update("\0");
  hash.update(patch);
  return hash.digest("hex");
}

async function prepareRoot(
  lock: DisplayServerLock,
  root: string,
  debs: Map<string, string>,
  scratch: string,
) {
  await rm(root, { recursive: true, force: true });
  await mkdir(root, { recursive: true });
  for (const entry of lock.build_root) {
    const deb = debs.get(entry.sha256);
    if (!deb) throw new Error(`${entry.name} was not downloaded`);
    // base-files provides the merged-/usr links; keep them.
    await extractDeb(deb, root, scratch, ["--keep-directory-symlink"]);
  }
  // What maintainer scripts would have created: the alternatives the build
  // calls by their generic names and the dpkg vendor (base-files' postinst),
  // which selects Ubuntu's build flags.
  const binaries = await readdir(join(root, "usr/bin"));
  for (const [link, candidate] of ALTERNATIVES) {
    if (binaries.includes(link)) continue;
    const matches = binaries.filter((name) => candidate.test(name));
    if (matches.length !== 1) {
      throw new Error(`build root: ${link} has candidates [${matches.join(", ")}]`);
    }
    await symlink(matches[0], join(root, "usr/bin", link));
  }
  await symlink("ubuntu", join(root, "etc/dpkg/origins/default"));
  await writeFile(
    join(root, "etc/passwd"),
    "root:x:0:0::/root:/bin/sh\nsbuild:x:1000:1000::/nonexistent:/bin/sh\n",
  );
  await writeFile(join(root, "etc/group"), "root:x:0:\nsbuild:x:1000:\n");
}

async function prepareSource(
  lock: DisplayServerLock,
  tree: string,
  sources: Map<string, string>,
  patch: string | null,
) {
  const file = (suffix: string) => {
    const entry = lock.source.files.find((candidate) => candidate.name.endsWith(suffix));
    if (!entry) throw new Error(`no ${suffix} in the lock`);
    return sources.get(entry.sha256)!;
  };
  checkDsc(await readFile(file(".dsc"), "utf8"), lock.source.files);
  const unpack = `${tree}.unpack`;
  await mkdir(unpack, { recursive: true });
  await run("tar", ["-xf", file(".orig.tar.xz"), "-C", unpack, "--no-same-owner"]);
  const top = await readdir(unpack);
  if (top.length !== 1) throw new Error("the orig tarball must hold one directory");
  await run("mv", [join(unpack, top[0]), tree]);
  await rm(unpack, { recursive: true });
  await run("tar", ["-xf", file(".debian.tar.xz"), "-C", tree, "--no-same-owner"]);
  // Quilt semantics: our patch is the last entry of the package's series.
  const patches = join(tree, "debian/patches");
  if (patch !== null) {
    await writeFile(join(patches, "cowboy-native-touch.patch"), patch);
    const series = await readFile(join(patches, "series"), "utf8");
    await writeFile(join(patches, "series"), `${series.trimEnd()}\ncowboy-native-touch.patch\n`);
  }
  for (const line of (await readFile(join(patches, "series"), "utf8")).split("\n")) {
    const entry = line.replace(/#.*$/, "").trim();
    if (!entry) continue;
    const [name, level = "-p1"] = entry.split(/\s+/);
    await run("patch", [
      "--batch",
      "--forward",
      "--no-backup-if-mismatch",
      "--fuzz=0",
      "--quiet",
      level,
      "-d",
      tree,
      "-i",
      join(patches, name),
    ]);
  }
}

/// `debian/rules build-arch`, then exactly what debhelper does to the
/// binary package's ELF files before it ships them: dh_dwz's multifile pass
/// over the package's two executables (installed as dh_auto_install and
/// dh_install name them), then dh_strip on Xtigervnc with automatic dbgsym:
/// a build id seeded with package and version, the compressed debug file,
/// the strip and the debuglink to that file.
function buildScript(buildinfo: BuildInfo): string {
  const multifile = `usr/lib/debug/.dwz/x86_64-linux-gnu/${BINARY_PACKAGE}.debug`;
  return [
    "set -eu",
    "export SOURCE_DATE_EPOCH=$(dpkg-parsechangelog -STimestamp)",
    "make -f debian/rules build-arch",
    `t=${PACKAGE_DIR}`,
    'rm -rf "$t"',
    'mkdir -p "$t/usr/bin" "$t/usr/sbin" "$t/usr/lib/debug/.dwz/x86_64-linux-gnu"',
    'install -m 755 obj-x86_64-linux-gnu/unix/xserver/hw/vnc/Xvnc "$t/usr/bin/Xtigervnc"',
    'install -m 755 obj-x86_64-linux-gnu/unix/vncserver/vncsession "$t/usr/sbin/tigervncsession"',
    `dwz "-m$t/${multifile}" "-M/${multifile}" -- "$t/usr/bin/Xtigervnc" "$t/usr/sbin/tigervncsession"`,
    'f="$t/usr/bin/Xtigervnc"',
    `debugedit --build-id --build-id-seed=${BINARY_PACKAGE}/${buildinfo.version} "$f"`,
    'id=$(LC_ALL=C readelf -n "$f" | sed -n "s/^ *Build ID: //p")',
    "mkdir -p /tmp/debug",
    'objcopy --only-keep-debug --compress-debug-sections "$f" "/tmp/debug/${id#??}.debug"',
    'strip --remove-section=.comment --remove-section=.note "$f"',
    'objcopy --add-gnu-debuglink "/tmp/debug/${id#??}.debug" "$f"',
  ].join("\n");
}

async function buildInSandbox(root: string, buildinfo: BuildInfo, logPath: string) {
  const jobs = Math.max(1, Math.min(cpus().length, 32));
  const environment = ["LANG", "DEB_BUILD_PROFILES"].flatMap((name) => {
    const value = buildinfo.environment.get(name);
    return value === undefined ? [] : ["--setenv", name, value];
  });
  const child = Bun.spawn({
    cmd: [
      "bwrap",
      "--unshare-all",
      "--die-with-parent",
      "--new-session",
      "--hostname",
      "localhost",
      "--bind",
      root,
      "/",
      "--dev",
      "/dev",
      "--proc",
      "/proc",
      "--tmpfs",
      "/tmp",
      "--chdir",
      buildinfo.path,
      "--clearenv",
      "--setenv",
      "PATH",
      "/usr/bin",
      "--setenv",
      "HOME",
      "/nonexistent",
      ...environment,
      "--setenv",
      "DEB_BUILD_OPTIONS",
      `nocheck parallel=${jobs}`,
      "/bin/sh",
      "-c",
      buildScript(buildinfo),
    ],
    stdin: "ignore",
    stdout: Bun.file(logPath),
    stderr: "pipe",
  });
  const stderr = await new Response(child.stderr).text();
  await writeFile(`${logPath}.stderr`, stderr);
  const code = await child.exited;
  if (code !== 0) {
    throw new Error(
      `the display server build exited ${code}; see ${logPath}\n${stderr.split("\n").slice(-25).join("\n")}`,
    );
  }
}

/// Assemble the build root, unpack the source with the package's quilt
/// series (plus `patch` when given) and build; returns the stripped Xvnc
/// inside the build root, which `cleanup` removes.
async function rebuild(lock: DisplayServerLock, patch: string | null, cacheDir: string) {
  const work = resolve(WORK_ROOT);
  await mkdir(cacheDir, { recursive: true });
  const buildinfo = await fetchBuildInfo(lock, cacheDir);
  console.error(`display server: fetching ${lock.build_root.length} build-root packages`);
  const debs = await downloadAll(
    lock.build_root.map((entry) => pinned(lock, entry)),
    cacheDir,
  );
  const sources = await downloadAll(
    lock.source.files.map((entry) => pinned(lock, entry)),
    cacheDir,
  );
  const root = join(work, "root");
  const scratch = join(work, "scratch");
  await rm(scratch, { recursive: true, force: true });
  await mkdir(scratch, { recursive: true });
  console.error("display server: assembling the build root");
  await prepareRoot(lock, root, debs, scratch);
  const tree = join(root, buildinfo.path.slice(1));
  await prepareSource(lock, tree, sources, patch);
  console.error("display server: building (debian/rules build-arch)");
  await buildInSandbox(root, buildinfo, join(work, "build.log"));
  const built = join(tree, BUILT_XVNC);
  return {
    built,
    digest: await sha256File(built),
    size: (await stat(built)).size,
    cleanup: async () => {
      await rm(root, { recursive: true, force: true });
      await rm(scratch, { recursive: true, force: true });
    },
  };
}

async function readLock(runtime: Lock, cacheDir: string) {
  const lock: DisplayServerLock = JSON.parse(await readFile(LOCK_PATH, "utf8"));
  const patch = await readFile(PATCH_PATH, "utf8");
  checkDisplayServerLock(lock, runtime, patch, await fetchBuildInfo(lock, cacheDir));
  return { lock, patch, patchDigest: createHash("sha256").update(patch).digest("hex") };
}

export async function buildDisplayServer(
  runtime: Lock,
  options: { record?: boolean; rebuild?: boolean; cacheDir: string },
): Promise<DisplayServer> {
  await mkdir(options.cacheDir, { recursive: true });
  const { lock, patch, patchDigest } = await readLock(runtime, options.cacheDir);
  const expected = options.record ? undefined : lock.output;
  if (!options.record) {
    if (!expected)
      throw new Error("display-server.lock.json records no output; build with --record");
    if (lock.patch?.sha256 !== patchDigest) {
      throw new Error(
        "native-touch.patch changed since its output was recorded; build with --record",
      );
    }
  }
  const cached = join(resolve(WORK_ROOT), "verified", inputsDigest(lock, patch), "Xtigervnc");
  if (!options.rebuild && expected) {
    const info = await stat(cached).catch(() => null);
    if (info?.isFile() && (await sha256File(cached)) === expected.sha256) {
      return describe(lock, cached, expected.sha256, info.size, false, patchDigest);
    }
  }
  const result = await rebuild(lock, patch, options.cacheDir);
  const { digest, size } = result;
  if (!expected) {
    lock.patch = { file: "native-touch.patch", sha256: patchDigest };
    lock.output = { path: "usr/bin/Xtigervnc", size, sha256: digest };
    await writeFile(LOCK_PATH, `${JSON.stringify(lock, null, 2)}\n`);
    console.error(`display server: recorded ${digest}`);
  } else if (digest !== expected.sha256 || size !== expected.size) {
    throw new Error(
      `the rebuilt display server is ${size} bytes ${digest}, not the recorded ${expected.size} bytes ${expected.sha256}: the build is not reproducible here`,
    );
  }
  await mkdir(join(cached, ".."), { recursive: true });
  await copyFile(result.built, cached);
  await result.cleanup();
  return describe(lock, cached, digest, size, true, patchDigest);
}

/// Provenance of the build root itself: without our patch the rebuild must
/// equal, byte for byte, the Xtigervnc Ubuntu ships in the locked
/// tigervnc-standalone-server package.
export async function verifyStockEquivalence(runtime: Lock, cacheDir: string) {
  await mkdir(cacheDir, { recursive: true });
  const { lock } = await readLock(runtime, cacheDir);
  const stock = runtime.packages.find((entry) => entry.name === BINARY_PACKAGE)!;
  const [deb] = (await downloadAll([stock], cacheDir)).values();
  const unpacked = join(resolve(WORK_ROOT), "stock");
  await rm(unpacked, { recursive: true, force: true });
  await mkdir(join(unpacked, "tree"), { recursive: true });
  await extractDeb(deb, join(unpacked, "tree"), unpacked);
  const shipped = await sha256File(join(unpacked, "tree/usr/bin/Xtigervnc"));
  await rm(unpacked, { recursive: true, force: true });
  const result = await rebuild(lock, null, cacheDir);
  // Kept for diagnosis when the two differ.
  await copyFile(result.built, join(resolve(WORK_ROOT), "unpatched-Xtigervnc"));
  await result.cleanup();
  const report = {
    stock: shipped,
    unpatched_rebuild: result.digest,
    identical: shipped === result.digest,
  };
  if (!report.identical) {
    throw new Error(
      `the unpatched rebuild differs from Ubuntu's binary: ${JSON.stringify(report)}`,
    );
  }
  return report;
}

function describe(
  lock: DisplayServerLock,
  path: string,
  sha256: string,
  size: number,
  rebuilt: boolean,
  patchDigest: string,
): DisplayServer {
  return {
    path,
    sha256,
    size,
    rebuilt,
    source_version: lock.source.version,
    patch_sha256: patchDigest,
    snapshot: lock.snapshot,
    buildinfo_sha256: lock.buildinfo.sha256,
  };
}

if (import.meta.main) {
  const [mode, ...rest] = process.argv.slice(2);
  if (rest.length || (mode && !["--record", "--rebuild", "--verify-stock"].includes(mode))) {
    throw new Error("usage: display-server.ts [--record | --rebuild | --verify-stock]");
  }
  const runtime: Lock = JSON.parse(await readFile(`${RUNTIME_DIR}/packages.lock.json`, "utf8"));
  const cacheDir = resolve("dist/plugins/machine-browser/debs");
  const result =
    mode === "--verify-stock"
      ? await verifyStockEquivalence(runtime, cacheDir)
      : await buildDisplayServer(runtime, {
          record: mode === "--record",
          rebuild: mode === "--rebuild",
          cacheDir,
        });
  console.log(JSON.stringify(result));
}
