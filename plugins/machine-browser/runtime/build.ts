// Package-owned, credential-free builder of the Machine browser runtime
// artifact: the complete Ubuntu userland of one browser instance, assembled
// deterministically from `packages.lock.json` (every .deb by URL, version and
// SHA-256), with the static launcher, the managed policy and `nsswitch.conf`
// added, every symlink and hard link replaced by a copy, every ELF object
// audited against the tree alone, and the whole packed as one tar.gz within
// the installer's limits. It never runs anything from the tree except the
// launcher's own `--probe`, and it needs no Docker: the lock already holds
// what the distribution's resolver decided (`lock.sh`).
//
// dpkg-deb is not part of the development shell, so the builder reads the
// `ar` container itself (`extract.ts`) and hands the `data.tar.*` member to
// GNU tar (xz and zstd come from the shell).
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { packTree } from "./archive.ts";
import { PROBE_TIMEOUT_MS, auditTree, probeLauncher, resolveLauncher } from "./audit.ts";
import { breakHardLinks, extractDeb, measureTree, replaceSymlinks } from "./extract.ts";
import { type Contract, type Lock, checkContract, downloadAll, run, sha256File } from "./lock.ts";

const PLUGIN_DIR = "plugins/machine-browser";
const OUTPUT_ROOT = "dist/plugins/machine-browser";
const ARCHIVE_NAME = "runtime-linux-x86_64.tar.gz";
const MAX_ENTRIES = 100_000;
const MAX_EXPANDED_BYTES = 4 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES = 1024 * 1024 * 1024;
const NSSWITCH_CONF =
  "passwd: files\ngroup: files\nshadow: files\nhosts: files dns\nservices: files\nprotocols: files\n";
// Not runtime files: documentation and shell integration shipped by packages.
const PRUNED_DIRECTORIES = [
  "usr/share/doc",
  "usr/share/doc-base",
  "usr/share/man",
  "usr/share/info",
  "usr/share/lintian",
  "usr/share/bug",
  "usr/share/menu",
  "usr/share/apport",
  "usr/share/bash-completion",
  "usr/share/zsh",
  "usr/share/fish",
  "usr/share/gdb",
  "usr/share/pkgconfig",
  "usr/include",
  "opt/google/chrome/cron",
];
// The setuid sandbox helper (Chromium uses its namespace sandbox) and the Qt
// theme shims, whose Qt libraries are deliberately not shipped.
const REMOVED_FILES = [
  "opt/google/chrome/chrome-sandbox",
  "opt/google/chrome/libqt5_shim.so",
  "opt/google/chrome/libqt6_shim.so",
];

interface Options {
  baseUrl: string;
  launcher?: string;
  allowDirty: boolean;
  keepTree: boolean;
  cacheDir?: string;
}

interface Inputs {
  options: Options;
  base: URL;
  revision: string;
  contract: Contract;
  lock: Lock;
  policy: string;
  treeRoot: string;
  cacheDir: string;
}

process.exitCode = await main(process.argv.slice(2));

async function main(argv: string[]): Promise<number> {
  const inputs = await readInputs(parseOptions(argv));
  const assembled = await assembleTree(inputs);
  const verified = await verifyTree(inputs);
  const receipt = await packAndRecord(inputs, assembled, verified);
  if (!inputs.options.keepTree) {
    await rm(inputs.treeRoot, { recursive: true, force: true });
  }
  console.log(JSON.stringify(receipt));
  return 0;
}

function parseOptions(argv: string[]): Options {
  const parsed: Options = { baseUrl: "", allowDirty: false, keepTree: false };
  const positional: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    switch (argument) {
      case "--launcher":
        parsed.launcher = argv[++index];
        break;
      case "--cache":
        parsed.cacheDir = argv[++index];
        break;
      case "--allow-dirty":
        parsed.allowDirty = true;
        break;
      case "--keep-tree":
        parsed.keepTree = true;
        break;
      default:
        if (argument.startsWith("--")) throw new Error(`unknown option ${argument}`);
        positional.push(argument);
    }
  }
  if (positional.length !== 1) {
    throw new Error(
      "usage: bun plugins/machine-browser/runtime/build.ts <artifact-base-url> [--launcher PATH] [--allow-dirty] [--keep-tree] [--cache DIR]",
    );
  }
  parsed.baseUrl = positional[0];
  return parsed;
}

/// Validate the environment and read the contract, lock and policy.
async function readInputs(options: Options): Promise<Inputs> {
  const base = new URL(options.baseUrl);
  if (
    base.protocol !== "https:" ||
    base.username ||
    base.password ||
    base.search ||
    base.hash ||
    base.pathname.replace(/\/$/, "") !== "/plugin-artifacts"
  ) {
    throw new Error("The artifact base must be an HTTPS /plugin-artifacts route");
  }
  if (process.platform !== "linux" || process.arch !== "x64") {
    throw new Error("The Machine browser Plugin declares only Linux x86_64");
  }
  if (!options.allowDirty && (await run("git", ["status", "--porcelain"])).trim()) {
    throw new Error("Final runtime builds require a clean committed worktree (or --allow-dirty)");
  }
  if (!/GNU tar/.test(await run("tar", ["--version"]))) {
    throw new Error("GNU tar is required");
  }
  const revisionOutput = await run("git", ["rev-parse", "HEAD"]);
  const contract: Contract = JSON.parse(await readFile(`${PLUGIN_DIR}/contract.json`, "utf8"));
  const lock: Lock = JSON.parse(await readFile(`${PLUGIN_DIR}/runtime/packages.lock.json`, "utf8"));
  const policy = await readFile(`${PLUGIN_DIR}/policy.json`, "utf8");
  JSON.parse(policy);
  checkContract(contract, lock);
  return {
    options,
    base,
    revision: revisionOutput.trim(),
    contract,
    lock,
    policy,
    treeRoot: resolve(`${OUTPUT_ROOT}/tree`),
    cacheDir: resolve(options.cacheDir ?? `${OUTPUT_ROOT}/debs`),
  };
}

interface Assembled {
  launcherSource: string;
  launcherDigest: string;
  symlinksReplaced: number;
  hardLinksBroken: number;
}

/// Download, extract, prune and normalize the tree, then add the launcher,
/// the policy and nsswitch.conf.
async function assembleTree(inputs: Inputs): Promise<Assembled> {
  const { contract, lock, policy, treeRoot, cacheDir } = inputs;
  await rm(treeRoot, { recursive: true, force: true });
  await mkdir(treeRoot, { recursive: true });
  await mkdir(cacheDir, { recursive: true });
  const everything = [...lock.packages, lock.browser];
  console.error(`downloading ${everything.length} packages`);
  const debs = await downloadAll(everything, cacheDir);
  console.error("extracting");
  const scratch = await mkdtemp(join(tmpdir(), "cowboy-browser-runtime-"));
  try {
    for (const locked of everything) {
      const deb = debs.get(locked.sha256);
      if (!deb) throw new Error(`${locked.name} was not downloaded`);
      await extractDeb(deb, treeRoot, scratch);
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
  console.error("pruning and normalizing");
  for (const entry of await readdir(treeRoot)) {
    if (entry !== "usr" && entry !== "opt") {
      await rm(join(treeRoot, entry), { recursive: true, force: true });
    }
  }
  for (const directory of PRUNED_DIRECTORIES) {
    await rm(join(treeRoot, directory), { recursive: true, force: true });
  }
  for (const file of REMOVED_FILES) {
    await rm(join(treeRoot, file), { force: true });
  }
  const symlinksReplaced = await replaceSymlinks(treeRoot);
  const hardLinksBroken = await breakHardLinks(treeRoot);
  console.error("adding the launcher, the policy and nsswitch.conf");
  const launcher = await resolveLauncher(inputs.options.launcher, contract.version);
  const launcherDestination = join(treeRoot, contract.runtime.launcher);
  await mkdir(dirname(launcherDestination), { recursive: true });
  await copyFile(launcher.path, launcherDestination);
  await chmod(launcherDestination, 0o755);
  await probeLauncher(launcherDestination, contract);
  const policyDestination = join(treeRoot, contract.runtime.policy.file);
  await mkdir(dirname(policyDestination), { recursive: true });
  await writeFile(policyDestination, policy);
  await chmod(policyDestination, 0o644);
  await writeFile(join(dirname(policyDestination), "nsswitch.conf"), NSSWITCH_CONF);
  return {
    launcherSource: launcher.source,
    launcherDigest: await sha256File(launcher.path),
    symlinksReplaced,
    hardLinksBroken,
  };
}

interface Verified {
  entries: number;
  bytes: number;
  elfObjects: number;
}

/// Every contract executable present, every ELF object resolvable, and the
/// tree within the installer's limits.
async function verifyTree(inputs: Inputs): Promise<Verified> {
  const { contract, treeRoot } = inputs;
  for (const required of [
    contract.runtime.launcher,
    contract.runtime.browser,
    contract.runtime.display_server,
    contract.runtime.policy.file,
    ...contract.runtime.helpers,
  ]) {
    const info = await lstat(join(treeRoot, required));
    if (!info.isFile()) throw new Error(`${required} is not a regular file in the tree`);
  }
  console.error("auditing ELF objects");
  const audit = await auditTree(treeRoot);
  if (audit.missing.length) {
    for (const problem of audit.missing) console.error(`  ${problem}`);
    throw new Error(`${audit.missing.length} ELF objects have unresolved dependencies`);
  }
  const measure = await measureTree(treeRoot);
  if (measure.entries > MAX_ENTRIES) {
    throw new Error(`tree has ${measure.entries} entries, more than ${MAX_ENTRIES}`);
  }
  if (measure.bytes > MAX_EXPANDED_BYTES) {
    throw new Error(`tree expands to ${measure.bytes} bytes, more than ${MAX_EXPANDED_BYTES}`);
  }
  return { entries: measure.entries, bytes: measure.bytes, elfObjects: audit.objects };
}

/// Pack the tree and write the runtime manifest and the build receipt.
async function packAndRecord(inputs: Inputs, assembled: Assembled, verified: Verified) {
  const { contract, lock, base, treeRoot } = inputs;
  console.error("packing");
  const archivePath = resolve(`${OUTPUT_ROOT}/${ARCHIVE_NAME}`);
  await packTree(treeRoot, archivePath);
  const archiveInfo = await stat(archivePath);
  if (archiveInfo.size > MAX_ARCHIVE_BYTES) {
    throw new Error(`archive is ${archiveInfo.size} bytes, more than ${MAX_ARCHIVE_BYTES}`);
  }
  const archiveDigest = await sha256File(archivePath);
  const component = {
    kind: "machine_browser_runtime",
    slot: contract.runtime.artifact,
    dependency: "cowboy-browser-runtime",
    version: contract.version,
    command: "cowboy-browser-launcher",
    artifact_url: `${base.href.replace(/\/$/, "")}/${archiveDigest}/${ARCHIVE_NAME}`,
    artifact_digest: `sha256:${archiveDigest}`,
    artifact_format: "tar_gz",
    entrypoint: contract.runtime.launcher,
    probe: { args: ["--probe"], timeout_ms: PROBE_TIMEOUT_MS },
  };
  const matrix = [{ os: "linux", architecture: "x86_64", components: [component] }];
  await writeFile(`${OUTPUT_ROOT}/runtime-artifacts.json`, `${JSON.stringify(matrix, null, 2)}\n`);
  const receipt = {
    schema_version: 1,
    plugin_id: contract.id,
    plugin_version: contract.version,
    source_revision: inputs.revision,
    dirty_worktree: inputs.options.allowDirty,
    lock: {
      generated_at: lock.generated_at,
      distribution: lock.distribution,
      browser_version: lock.browser.browser_version,
      packages: lock.packages.length + 1,
    },
    launcher: { source: assembled.launcherSource, sha256: assembled.launcherDigest },
    tree: {
      entries: verified.entries,
      bytes: verified.bytes,
      elf_objects: verified.elfObjects,
      symlinks_replaced: assembled.symlinksReplaced,
      hard_links_broken: assembled.hardLinksBroken,
    },
    archive: {
      path: `${OUTPUT_ROOT}/${ARCHIVE_NAME}`,
      bytes: archiveInfo.size,
      sha256: archiveDigest,
    },
  };
  await writeFile(`${OUTPUT_ROOT}/build-receipt.json`, `${JSON.stringify(receipt, null, 2)}\n`);
  return receipt;
}
