import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import {
  checkPackage,
  parsePatch,
  prepareSource,
  run,
  validateUpstream,
} from "./lib/third-party.ts";
import { assert, assertEquals, assertMatch } from "./test-assert.ts";

function patch(id: string, trailers: Record<string, string> = {}, path = "src/a.ts"): string {
  const values = {
    "Cowboy-Delta": id,
    Upstream: "not-proposed",
    "Drop-When": "upstream ships it",
    Tests: "src/a.test.ts",
    ...trailers,
  };
  return [
    "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001",
    "From: Test <test@example.invalid>",
    "Date: Wed, 18 Nov 2026 00:00:00 +0000",
    "Subject: [PATCH] Change a",
    "",
    "Why.",
    "",
    ...Object.entries(values).map(([key, value]) => `${key}: ${value}`),
    "",
    `diff --git a/${path} b/${path}`,
    "--- a/src/a.ts",
    `+++ b/${path}`,
    "@@ -1 +1 @@",
    "-old",
    "+new",
    "diff --git a/src/a.test.ts b/src/a.test.ts",
    "new file mode 100644",
    "--- /dev/null",
    "+++ b/src/a.test.ts",
    "@@ -0,0 +1 @@",
    "+test",
    "",
  ].join("\n");
}

function rejects(action: () => unknown, message: string): void {
  try {
    action();
  } catch (error) {
    assertMatch((error as Error).message, new RegExp(message));
    return;
  }
  throw new Error(`expected a failure matching ${message}`);
}

test("parses the delta ledger from the patch trailers", () => {
  const delta = parsePatch("0001-a-change.patch", patch("a-change"));
  assertEquals(delta.id, "a-change");
  assertEquals(delta.subject, "Change a");
  assertEquals(delta.tests, ["src/a.test.ts"]);
  assertEquals(delta.paths, ["src/a.ts", "src/a.test.ts"]);
});

test("rejects an incomplete or inconsistent ledger", () => {
  rejects(() => parsePatch("0001-a-change.patch", patch("other")), "does not match the file name");
  rejects(
    () => parsePatch("0001-a-change.patch", patch("a-change", { "Drop-When": "" })),
    "missing Drop-When",
  );
  rejects(
    () => parsePatch("0001-a-change.patch", patch("a-change", { Upstream: "maybe" })),
    "invalid Upstream",
  );
  rejects(
    () => parsePatch("0001-a-change.patch", patch("a-change", { Tests: "src/b.test.ts" })),
    "not changed by this delta",
  );
  rejects(() => parsePatch("a-change.patch", patch("a-change")), "NNNN-");
  assertEquals(
    parsePatch(
      "0001-a-change.patch",
      patch("a-change", { Upstream: "proposed https://github.com/o/r/pull/1" }),
    ).upstream,
    "proposed https://github.com/o/r/pull/1",
  );
});

async function fixture(): Promise<{ root: string; upstream: Record<string, unknown> }> {
  const root = await mkdtemp(join(tmpdir(), "third-party-test-"));
  await mkdir(join(root, "third_party", "lib", "patches"), { recursive: true });
  for (const [plugin, version] of [
    ["source-user", "1.0.0"],
    ["registry-user", "1.0.0"],
  ]) {
    await mkdir(join(root, "plugins", plugin), { recursive: true });
    await writeFile(
      join(root, "plugins", plugin, "provider.json"),
      JSON.stringify({ runtime: { dependencies: [{ id: "lib", version }] } }),
    );
  }
  const upstream = {
    schema_version: 1,
    name: "lib",
    repository: "https://github.com/example/lib",
    version: "1.0.0",
    tag: "v1.0.0",
    commit: "a".repeat(40),
    archive_sha256: "b".repeat(64),
    pinned_files: {},
    release_tag_pattern: "^v\\d+\\.\\d+\\.\\d+$",
    consumers: [
      { path: "plugins/source-user", mode: "source" },
      { path: "plugins/registry-user", mode: "registry" },
    ],
  };
  await writeFile(join(root, "third_party", "lib", "upstream.json"), JSON.stringify(upstream));
  return { root, upstream };
}

test("checks series order, consumers and their pins", async () => {
  const { root, upstream } = await fixture();
  try {
    const patches = join(root, "third_party", "lib", "patches");
    await writeFile(join(patches, "0001-one.patch"), patch("one"));
    assertEquals(await checkPackage("lib", root), []);

    await writeFile(join(patches, "0003-two.patch"), patch("two"));
    assertMatch((await checkPackage("lib", root)).join("\n"), /contiguous/);
    await rm(join(patches, "0003-two.patch"));

    await writeFile(
      join(root, "plugins", "registry-user", "provider.json"),
      JSON.stringify({ runtime: { dependencies: [{ id: "lib", version: "0.9.0" }] } }),
    );
    assertMatch((await checkPackage("lib", root)).join("\n"), /pins lib 0\.9\.0/);

    await mkdir(join(root, "plugins", "undeclared"));
    await writeFile(
      join(root, "plugins", "undeclared", "provider.json"),
      JSON.stringify({ runtime: { dependencies: [{ id: "lib", version: "1.0.0" }] } }),
    );
    assertMatch((await checkPackage("lib", root)).join("\n"), /not a declared consumer/);

    const registryOnly = {
      ...upstream,
      consumers: [{ path: "plugins/registry-user", mode: "registry" }],
    };
    await writeFile(
      join(root, "third_party", "lib", "upstream.json"),
      JSON.stringify(registryOnly),
    );
    assertMatch((await checkPackage("lib", root)).join("\n"), /needs a source consumer/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("validates the upstream manifest", () => {
  const problems = validateUpstream("lib", {
    schema_version: 1,
    name: "other",
    repository: "https://gitlab.com/x/y",
  });
  assert(problems.some((problem) => problem.startsWith("name")));
  assert(problems.some((problem) => problem.startsWith("repository")));
  assert(problems.some((problem) => problem.startsWith("consumers")));
});

test("prepares the pinned source and applies the series in order at fuzz 0", async () => {
  const { root, upstream } = await fixture();
  try {
    const tree = join(root, "upstream", "lib-1.0.0");
    await mkdir(join(tree, "src"), { recursive: true });
    await writeFile(join(tree, "src", "a.ts"), "old\n");
    const cache = join(root, "cache");
    await mkdir(cache);
    const staged = join(cache, "staged.tar.gz");
    await run("tar", ["-czf", staged, "-C", join(root, "upstream"), "lib-1.0.0"]);
    const digest = createHash("sha256")
      .update(await readFile(staged))
      .digest("hex");
    await run("mv", [staged, join(cache, `lib-${digest}.tar.gz`)]);
    const pinned = createHash("sha256").update("old\n").digest("hex");
    await writeFile(
      join(root, "third_party", "lib", "upstream.json"),
      JSON.stringify({ ...upstream, archive_sha256: digest, pinned_files: { "src/a.ts": pinned } }),
    );
    await writeFile(join(root, "third_party", "lib", "patches", "0001-one.patch"), patch("one"));

    const destination = join(root, "source");
    const { series } = await prepareSource("lib", destination, cache, root);
    assertEquals(
      series.map((delta) => delta.id),
      ["one"],
    );
    assertEquals(await readFile(join(destination, "src", "a.ts"), "utf8"), "new\n");
    assertEquals(await readFile(join(destination, "src", "a.test.ts"), "utf8"), "test\n");

    // A delta whose context no longer matches fails instead of applying with fuzz.
    await writeFile(join(root, "third_party", "lib", "patches", "0002-two.patch"), patch("two"));
    let failed = false;
    await prepareSource("lib", join(root, "again"), cache, root).catch(() => (failed = true));
    assert(failed);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
