import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdtemp, readdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { copyImmutable, copyImmutableText } from "./immutable-publication.ts";

async function fixture(run: (root: string) => Promise<void>) {
  const root = await mkdtemp(join(tmpdir(), "cowboy-publication-test-"));
  try {
    await run(root);
    for (const entry of await readdir(root, { withFileTypes: true })) {
      if (entry.name.startsWith(".cowboy-publication-")) {
        throw new Error("publication leaked a temporary file");
      }
    }
  } finally {
    await rm(root, { recursive: true });
  }
}

async function rejects(run: () => Promise<void>) {
  try {
    await run();
  } catch {
    return;
  }
  throw new Error("immutable publication unexpectedly succeeded");
}

test("immutable files and text allow identical retries only", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    const target = `${root}/target`;
    await writeFile(source, "original");
    await copyImmutable(source, target);
    await copyImmutable(source, target);
    await copyImmutableText("original", target);
    await writeFile(source, "replacement");
    await rejects(() => copyImmutable(source, target));
    await rejects(() => copyImmutableText("replacement", target));
    if (await readFile(target, "utf8") !== "original") {
      throw new Error("retry replaced immutable bytes");
    }
  }));

test("simultaneous different publications cannot overwrite the winner", () =>
  fixture(async (root) => {
    const target = `${root}/target`;
    const attempts = await Promise.allSettled(
      Array.from(
        { length: 24 },
        (_, index) => copyImmutableText(`candidate-${index}`, target),
      ),
    );
    const winners = attempts.flatMap((result, index) =>
      result.status === "fulfilled" ? [index] : []
    );
    if (winners.length !== 1) {
      throw new Error("more than one release committed");
    }
    if (await readFile(target, "utf8") !== `candidate-${winners[0]}`) {
      throw new Error("publication race replaced the winner");
    }
  }));

test("simultaneous identical file publications are idempotent", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    await writeFile(source, "same release");
    await Promise.all(
      Array.from({ length: 12 }, () => copyImmutable(source, `${root}/target`)),
    );
    if (await readFile(`${root}/target`, "utf8") !== "same release") {
      throw new Error("publication race corrupted the release");
    }
  }));

test("publication rejects source and target symlinks even for identical bytes", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    const link = `${root}/link`;
    await writeFile(source, "original");
    await symlink(source, link);
    await rejects(() => copyImmutable(source, link));
    await rejects(() => copyImmutableText("original", link));
    await rejects(() => copyImmutable(link, `${root}/target`));
    await rejects(() => copyImmutableText("data", source + "/child"));
    if (await readFile(source, "utf8") !== "original") {
      throw new Error("publication followed a symlink");
    }
  }));
