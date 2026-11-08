// Deterministic packing of the tree: GNU format, sorted names, fixed owner,
// mode and timestamp, gzip without a name or timestamp.
import { rm } from "node:fs/promises";

// Deterministic member timestamp (2000-01-01T00:00:00Z).
const ARCHIVE_MTIME = "2000-01-01 00:00:00 UTC";

export async function packTree(tree: string, archive: string) {
  await rm(archive, { force: true });
  const tar = Bun.spawn({
    cmd: [
      "tar",
      "--format=gnu",
      "--sort=name",
      "--owner=0",
      "--group=0",
      "--numeric-owner",
      `--mtime=${ARCHIVE_MTIME}`,
      "--mode=u+rw,go+r,go-w,a-st",
      "-C",
      tree,
      "-cf",
      "-",
      "usr",
      "opt",
    ],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "inherit",
  });
  const gzip = Bun.spawn({
    cmd: ["gzip", "-n", "-6"],
    stdin: tar.stdout,
    stdout: Bun.file(archive),
    stderr: "inherit",
  });
  const [tarCode, gzipCode] = await Promise.all([tar.exited, gzip.exited]);
  if (tarCode !== 0) throw new Error(`tar exited ${tarCode}`);
  if (gzipCode !== 0) throw new Error(`gzip exited ${gzipCode}`);
}
