// The ELF audit (every object's NEEDED entries resolved against the tree
// alone, the loader's default directories and RUNPATH/RPATH only) and the
// launcher checks: a standalone static executable whose probe answers for
// the contract.
import { mkdtemp, open, lstat, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { run, type Contract } from "./lock.ts";
import { walk } from "./extract.ts";

export const PROBE_TIMEOUT_MS = 5_000;
// Default dynamic-loader search directories of the artifact's glibc (merged
// usr; no ld.so.cache exists inside an instance, so ld.so.conf.d is not
// consulted).
const DEFAULT_LIBRARY_DIRS = ["usr/lib/x86_64-linux-gnu", "usr/lib64", "usr/lib"];
const INTERPRETER = "/lib64/ld-linux-x86-64.so.2";

export interface ElfInfo {
  needed: string[];
  runpaths: string[];
  interpreter?: string;
}

export async function readElf(path: string): Promise<ElfInfo | null> {
  const handle = await open(path, "r");
  try {
    const header = Buffer.alloc(64);
    const { bytesRead } = await handle.read(header, 0, 64, 0);
    if (bytesRead < 64 || header.readUInt32BE(0) !== 0x7f454c46) return null;
    if (header[4] !== 2 || header[5] !== 1) {
      throw new Error(`${path}: only little-endian ELF64 objects are supported`);
    }
    const phoff = Number(header.readBigUInt64LE(32));
    const phentsize = header.readUInt16LE(54);
    const phnum = header.readUInt16LE(56);
    const table = Buffer.alloc(phentsize * phnum);
    await handle.read(table, 0, table.length, phoff);
    const loads: Array<{ offset: number; vaddr: number; filesz: number }> = [];
    let dynamic: { offset: number; filesz: number } | undefined;
    let interpreter: string | undefined;
    for (let index = 0; index < phnum; index += 1) {
      const entry = table.subarray(index * phentsize, (index + 1) * phentsize);
      const type = entry.readUInt32LE(0);
      const offset = Number(entry.readBigUInt64LE(8));
      const vaddr = Number(entry.readBigUInt64LE(16));
      const filesz = Number(entry.readBigUInt64LE(32));
      if (type === 1) loads.push({ offset, vaddr, filesz });
      if (type === 2) dynamic = { offset, filesz };
      if (type === 3) {
        const text = Buffer.alloc(filesz);
        await handle.read(text, 0, filesz, offset);
        interpreter = text.toString("latin1").replace(/\0.*$/s, "");
      }
    }
    const info: ElfInfo = { needed: [], runpaths: [], interpreter };
    if (!dynamic) return info;
    const entries = Buffer.alloc(dynamic.filesz);
    await handle.read(entries, 0, dynamic.filesz, dynamic.offset);
    const needed: number[] = [];
    const runpaths: number[] = [];
    let strtab: number | undefined;
    let strsz = 0;
    for (let offset = 0; offset + 16 <= entries.length; offset += 16) {
      const tag = Number(entries.readBigInt64LE(offset));
      const value = Number(entries.readBigUInt64LE(offset + 8));
      if (tag === 0) break;
      if (tag === 1) needed.push(value);
      if (tag === 5) strtab = value;
      if (tag === 10) strsz = value;
      if (tag === 15 || tag === 29) runpaths.push(value);
    }
    if (strtab === undefined || (!needed.length && !runpaths.length)) return info;
    const load = loads.find(
      (segment) => strtab! >= segment.vaddr && strtab! < segment.vaddr + segment.filesz,
    );
    if (!load) throw new Error(`${path}: dynamic string table outside every loadable segment`);
    const strings = Buffer.alloc(strsz);
    await handle.read(strings, 0, strsz, strtab - load.vaddr + load.offset);
    const read = (at: number) => strings.subarray(at).toString("latin1").replace(/\0.*$/s, "");
    info.needed = needed.map(read);
    info.runpaths = runpaths.flatMap((at) => read(at).split(":").filter(Boolean));
    return info;
  } finally {
    await handle.close();
  }
}

export async function auditTree(tree: string): Promise<{ objects: number; missing: string[] }> {
  const missing: string[] = [];
  let objects = 0;
  const files: string[] = [];
  await walk(tree, async (path, info) => {
    if (info.isFile()) files.push(path);
  });
  const loaderPath = join(tree, "usr/lib64/ld-linux-x86-64.so.2");
  if (!(await lstat(loaderPath).catch(() => null))?.isFile()) {
    missing.push("usr/lib64/ld-linux-x86-64.so.2 (the dynamic loader) is missing");
  }
  for (const path of files) {
    const elf = await readElf(path);
    if (!elf) continue;
    objects += 1;
    const shown = relative(tree, path);
    if (elf.interpreter && elf.interpreter !== INTERPRETER) {
      missing.push(`${shown}: interpreter ${elf.interpreter} is not ${INTERPRETER}`);
    }
    const origin = dirname(path);
    const searchDirs = [
      ...elf.runpaths.map((entry) => {
        if (entry.includes("$ORIGIN") || entry.includes("${ORIGIN}")) {
          return entry.replaceAll("$ORIGIN", origin).replaceAll("${ORIGIN}", origin);
        }
        return entry.startsWith("/") ? join(tree, entry) : join(origin, entry);
      }),
      ...DEFAULT_LIBRARY_DIRS.map((directory) => join(tree, directory)),
    ];
    for (const library of elf.needed) {
      let found = false;
      for (const directory of searchDirs) {
        const candidate = join(directory, library);
        if (!candidate.startsWith(`${tree}/`)) continue;
        if ((await lstat(candidate).catch(() => null))?.isFile()) {
          found = true;
          break;
        }
      }
      if (!found) missing.push(`${shown}: ${library} not found in the tree`);
    }
  }
  return { objects, missing };
}

export async function resolveLauncher(
  explicit: string | undefined,
  version: string,
): Promise<{ path: string; source: string }> {
  let path: string;
  let source: string;
  if (explicit) {
    path = await realpath(explicit);
    source = path;
  } else {
    const build = JSON.parse(
      await run("nix", ["build", "--no-link", "--json", ".#cowboy-browser-launcher-portable"]),
    );
    if (build.length !== 1 || typeof build[0].outputs.out !== "string") {
      throw new Error("Unexpected Nix build result");
    }
    const immutable = build[0].outputs.out as string;
    if (!immutable.endsWith(`-${version}`)) {
      throw new Error("Nix output does not match the Plugin version");
    }
    path = `${immutable}/bin/cowboy-browser-launcher`;
    source = immutable;
  }
  const elf = await readElf(path);
  if (!elf) throw new Error(`${path} is not an ELF executable`);
  if (elf.interpreter || elf.needed.length) {
    throw new Error("cowboy-browser-launcher is not a standalone static executable");
  }
  return { path, source };
}

export async function probeLauncher(path: string, contract: Contract) {
  const home = await mkdtemp(join(tmpdir(), "cowboy-browser-launcher-probe-"));
  try {
    const child = Bun.spawn({
      cmd: [path, "--probe"],
      cwd: home,
      env: { HOME: home },
      stdin: "ignore",
      stdout: "pipe",
      stderr: "inherit",
    });
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {
        // already exited
      }
    }, PROBE_TIMEOUT_MS);
    try {
      const output = await new Response(child.stdout).text();
      if ((await child.exited) !== 0) throw new Error("launcher probe failed");
      const probe = JSON.parse(output.trim());
      if (probe.protocol !== contract.launch.protocol || probe.version !== contract.version) {
        throw new Error(
          `launcher probe reported ${output.trim()}, not ${contract.launch.protocol} ${contract.version}`,
        );
      }
    } finally {
      clearTimeout(timer);
    }
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}
