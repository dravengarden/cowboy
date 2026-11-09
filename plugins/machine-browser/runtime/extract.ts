// Package extraction and tree normalization: the `ar` container of a .deb,
// its data member handed to GNU tar, then the tree walk that replaces every
// symlink and hard link with a copy and measures what remains.
import type { Stats } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readdir,
  readFile,
  readlink,
  rename,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { dirname, join, normalize, relative, resolve } from "node:path";
import { run } from "./lock.ts";

/// Members of a `.deb` (`ar` archive): name and byte range.
export function parseAr(bytes: Buffer): Array<{ name: string; start: number; size: number }> {
  if (bytes.subarray(0, 8).toString("latin1") !== "!<arch>\n") {
    throw new Error("not an ar archive");
  }
  const members = [];
  let offset = 8;
  while (offset + 60 <= bytes.length) {
    const header = bytes.subarray(offset, offset + 60);
    if (header.subarray(58, 60).toString("latin1") !== "`\n") {
      throw new Error("corrupt ar member header");
    }
    const name = header.subarray(0, 16).toString("latin1").trim().replace(/\/$/, "");
    const size = Number.parseInt(header.subarray(48, 58).toString("latin1").trim(), 10);
    if (!Number.isFinite(size) || size < 0 || offset + 60 + size > bytes.length) {
      throw new Error("corrupt ar member size");
    }
    members.push({ name, start: offset + 60, size });
    offset += 60 + size + (size % 2);
  }
  return members;
}

export async function extractDeb(
  debPath: string,
  tree: string,
  scratch: string,
  tarOptions: string[] = [],
) {
  const bytes = await readFile(debPath);
  const members = parseAr(bytes);
  if (members[0]?.name !== "debian-binary") {
    throw new Error(`${debPath}: not a Debian package`);
  }
  const data = members.find((member) => member.name.startsWith("data.tar"));
  if (!data) throw new Error(`${debPath}: no data member`);
  const member = join(scratch, data.name);
  await writeFile(member, bytes.subarray(data.start, data.start + data.size));
  await run("tar", [
    "-xf",
    member,
    "-C",
    tree,
    "--no-same-owner",
    "--no-same-permissions",
    "--no-overwrite-dir",
    "--warning=no-unknown-keyword",
    ...tarOptions,
  ]);
  await rm(member, { force: true });
}

export async function walk(root: string, visit: (path: string, info: Stats) => Promise<void>) {
  const pending = [root];
  while (pending.length) {
    const directory = pending.pop()!;
    const entries = (await readdir(directory)).sort();
    for (const entry of entries) {
      const path = join(directory, entry);
      const info = await lstat(path);
      await visit(path, info);
      if (info.isDirectory()) pending.push(path);
    }
  }
}

/// Replace every symlink by a copy of what it points to, inside the tree
/// only. Links to directories are copied as trees whose own links are
/// resolved on later passes; dangling links and links leaving the tree are
/// removed.
export async function replaceSymlinks(tree: string): Promise<number> {
  let replaced = 0;
  for (let pass = 0; pass < 16; pass += 1) {
    const links: string[] = [];
    await walk(tree, async (path, info) => {
      if (info.isSymbolicLink()) links.push(path);
    });
    if (!links.length) return replaced;
    let progressed = false;
    for (const link of links) {
      const target = normalize(resolveLinkTarget(tree, link, await readlink(link)));
      if (!target.startsWith(`${tree}/`)) {
        console.error(`  removing symlink ${relative(tree, link)} -> outside the tree`);
        await unlink(link);
        progressed = true;
        continue;
      }
      let targetInfo;
      try {
        targetInfo = await lstat(target);
      } catch {
        console.error(`  removing dangling symlink ${relative(tree, link)}`);
        await unlink(link);
        progressed = true;
        continue;
      }
      if (targetInfo.isSymbolicLink()) continue; // resolved on a later pass
      await unlink(link);
      if (targetInfo.isFile()) {
        await copyFile(target, link);
        await chmod(link, targetInfo.mode & 0o777);
      } else if (targetInfo.isDirectory()) {
        console.error(
          `  copying directory ${relative(tree, target)} over symlink ${relative(tree, link)}`,
        );
        await copyDirectory(tree, target, link);
      } else {
        throw new Error(`${relative(tree, link)} points at an unsupported entry`);
      }
      replaced += 1;
      progressed = true;
    }
    if (!progressed) {
      throw new Error(
        `symlink cycle among ${links.map((link) => relative(tree, link)).join(", ")}`,
      );
    }
  }
  throw new Error("symlink replacement did not converge");
}

function resolveLinkTarget(tree: string, link: string, target: string): string {
  if (target.startsWith("/")) return join(tree, target);
  return resolve(dirname(link), target);
}

/// Copy a directory a symlink pointed at. Symlinks inside it are recreated
/// as tree-absolute links to the object they resolved to at the original
/// location, so a later pass replaces them with the right copy.
async function copyDirectory(tree: string, source: string, destination: string) {
  await mkdir(destination, { recursive: true });
  for (const entry of (await readdir(source)).sort()) {
    const from = join(source, entry);
    const to = join(destination, entry);
    const info = await lstat(from);
    if (info.isDirectory()) {
      await copyDirectory(tree, from, to);
    } else if (info.isFile()) {
      await copyFile(from, to);
      await chmod(to, info.mode & 0o777);
    } else if (info.isSymbolicLink()) {
      const target = normalize(resolveLinkTarget(tree, from, await readlink(from)));
      await symlink(target.startsWith(`${tree}/`) ? `/${relative(tree, target)}` : target, to);
    } else {
      throw new Error(`${from} is an unsupported entry`);
    }
  }
}

export async function breakHardLinks(tree: string): Promise<number> {
  let broken = 0;
  await walk(tree, async (path, info) => {
    if (info.isFile() && info.nlink > 1) {
      const temporary = `${path}.cowboy-unlink-${process.pid}`;
      await copyFile(path, temporary);
      await chmod(temporary, info.mode & 0o777);
      await rename(temporary, path);
      broken += 1;
    }
  });
  return broken;
}

export async function measureTree(tree: string): Promise<{ entries: number; bytes: number }> {
  let entries = 0;
  let bytes = 0;
  await walk(tree, async (_path, info) => {
    if (!info.isFile() && !info.isDirectory()) {
      throw new Error("tree still contains something other than files and directories");
    }
    entries += 1;
    if (info.isFile()) bytes += info.size;
  });
  return { entries, bytes };
}
