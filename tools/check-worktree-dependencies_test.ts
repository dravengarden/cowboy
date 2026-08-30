import { resolve } from "node:path";
import {
  repairBorrowedWorktreeState,
  verifyWorktreeDependencyView,
} from "./check-worktree-dependencies.ts";

Deno.test("rejects a node_modules directory borrowed from another checkout", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const otherModules = resolve(otherRoot, "web", "node_modules");
    await Deno.mkdir(otherModules, { recursive: true });
    await Deno.symlink(otherModules, resolve(root, "web", "node_modules"));

    await assertRejects(
      () => verifyWorktreeDependencyView(root),
      "checkout-local directory, not a symlink",
    );
  });
});

Deno.test("repairs only the borrowed node_modules link", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const otherModules = resolve(otherRoot, "web", "node_modules");
    const sentinel = resolve(otherModules, "keep-me");
    await Deno.mkdir(otherModules, { recursive: true });
    await Deno.writeTextFile(sentinel, "stable checkout dependency");
    await Deno.symlink(otherModules, resolve(root, "web", "node_modules"));

    assert(await repairBorrowedWorktreeState(root), "expected a repair");
    assert(
      (await Deno.lstat(sentinel)).isFile,
      "repair removed the borrowed checkout's dependency",
    );
    await verifyWorktreeDependencyView(root);
  });
});

Deno.test("rejects and repairs the obsolete external source seam", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const source = resolve(root, "web", "src");
    await Deno.mkdir(source, { recursive: true });
    await Deno.symlink(
      resolve(otherRoot, "components"),
      resolve(source, "_shell"),
    );

    await assertRejects(
      () => verifyWorktreeDependencyView(root),
      "obsolete external source link",
    );
    assert(await repairBorrowedWorktreeState(root), "expected a repair");
    await verifyWorktreeDependencyView(root);
  });
});

Deno.test("rejects a local package link into another checkout", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const borrowed = resolve(otherRoot, "components", "app-shell");
    await linkInstalledComponent(
      root,
      borrowed,
    );
    const sentinel = resolve(borrowed, "keep-me");
    await Deno.writeTextFile(sentinel, "other checkout source");

    await assertRejects(
      () => verifyWorktreeDependencyView(root, { requireInstalled: true }),
      "expected this worktree",
    );
    assert(await repairBorrowedWorktreeState(root), "expected a repair");
    assert(
      (await Deno.lstat(sentinel)).isFile,
      "repair removed the other checkout's package source",
    );
    await linkInstalledComponent(
      root,
      resolve(root, "components", "app-shell"),
    );
    await verifyWorktreeDependencyView(root, { requireInstalled: true });
  });
});

Deno.test("accepts checkout-local node_modules and file package links", async () => {
  await withFixture(async ({ root }) => {
    await linkInstalledComponent(
      root,
      resolve(root, "components", "app-shell"),
    );
    await verifyWorktreeDependencyView(root, { requireInstalled: true });
  });
});

async function withFixture(
  run: (fixture: { root: string; otherRoot: string }) => Promise<void>,
): Promise<void> {
  const base = await Deno.makeTempDir({ prefix: "cowboy-dependency-view-" });
  const root = resolve(base, "current");
  const otherRoot = resolve(base, "other");
  try {
    for (const checkout of [root, otherRoot]) {
      await Deno.mkdir(resolve(checkout, "web"), { recursive: true });
      await Deno.mkdir(resolve(checkout, "components", "app-shell"), {
        recursive: true,
      });
    }
    await Deno.writeTextFile(
      resolve(root, "web", "package.json"),
      JSON.stringify({
        dependencies: { "@cowboy/app-shell": "file:../components/app-shell" },
      }),
    );
    await run({ root, otherRoot });
  } finally {
    await Deno.remove(base, { recursive: true });
  }
}

async function linkInstalledComponent(
  root: string,
  component: string,
): Promise<void> {
  const scope = resolve(root, "web", "node_modules", "@cowboy");
  await Deno.mkdir(scope, { recursive: true });
  await Deno.symlink(component, resolve(scope, "app-shell"));
}

async function assertRejects(
  run: () => Promise<void>,
  expectedMessage: string,
): Promise<void> {
  try {
    await run();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes(expectedMessage)) return;
    throw new Error(
      `expected error containing ${JSON.stringify(expectedMessage)}, got ${
        JSON.stringify(message)
      }`,
    );
  }
  throw new Error(
    `expected error containing ${JSON.stringify(expectedMessage)}`,
  );
}

function assert(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
