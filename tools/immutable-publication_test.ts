import { copyImmutable, copyImmutableText } from "./immutable-publication.ts";

async function fixture(run: (root: string) => Promise<void>) {
  const root = await Deno.makeTempDir({ prefix: "cowboy-publication-test-" });
  try {
    await run(root);
    for await (const entry of Deno.readDir(root)) {
      if (entry.name.startsWith(".cowboy-publication-")) {
        throw new Error("publication leaked a temporary file");
      }
    }
  } finally {
    await Deno.remove(root, { recursive: true });
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

Deno.test("immutable files and text allow identical retries only", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    const target = `${root}/target`;
    await Deno.writeTextFile(source, "original");
    await copyImmutable(source, target);
    await copyImmutable(source, target);
    await copyImmutableText("original", target);
    await Deno.writeTextFile(source, "replacement");
    await rejects(() => copyImmutable(source, target));
    await rejects(() => copyImmutableText("replacement", target));
    if (await Deno.readTextFile(target) !== "original") {
      throw new Error("retry replaced immutable bytes");
    }
  }));

Deno.test("simultaneous different publications cannot overwrite the winner", () =>
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
    if (await Deno.readTextFile(target) !== `candidate-${winners[0]}`) {
      throw new Error("publication race replaced the winner");
    }
  }));

Deno.test("simultaneous identical file publications are idempotent", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    await Deno.writeTextFile(source, "same release");
    await Promise.all(
      Array.from({ length: 12 }, () => copyImmutable(source, `${root}/target`)),
    );
    if (await Deno.readTextFile(`${root}/target`) !== "same release") {
      throw new Error("publication race corrupted the release");
    }
  }));

Deno.test("publication rejects source and target symlinks even for identical bytes", () =>
  fixture(async (root) => {
    const source = `${root}/source`;
    const link = `${root}/link`;
    await Deno.writeTextFile(source, "original");
    await Deno.symlink(source, link);
    await rejects(() => copyImmutable(source, link));
    await rejects(() => copyImmutableText("original", link));
    await rejects(() => copyImmutable(link, `${root}/target`));
    await rejects(() => copyImmutableText("data", source + "/child"));
    if (await Deno.readTextFile(source) !== "original") {
      throw new Error("publication followed a symlink");
    }
  }));
