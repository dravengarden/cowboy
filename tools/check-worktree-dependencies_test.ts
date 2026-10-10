import { tmpdir } from "node:os";
import { lstat, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { join, resolve } from "node:path";
import {
  repairBorrowedWorktreeState,
  verifyWorktreeDependencyView,
} from "./check-worktree-dependencies.ts";

test("rejects a node_modules directory borrowed from another checkout", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const otherModules = resolve(otherRoot, "web", "node_modules");
    await mkdir(otherModules, { recursive: true });
    await symlink(otherModules, resolve(root, "web", "node_modules"));

    await assertRejects(
      () => verifyWorktreeDependencyView(root),
      "checkout-local directory, not a symlink",
    );
  });
});

test("repairs only the borrowed node_modules link", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const otherModules = resolve(otherRoot, "web", "node_modules");
    const sentinel = resolve(otherModules, "keep-me");
    await mkdir(otherModules, { recursive: true });
    await writeFile(sentinel, "stable checkout dependency");
    await symlink(otherModules, resolve(root, "web", "node_modules"));

    assert(await repairBorrowedWorktreeState(root), "expected a repair");
    assert(
      (await lstat(sentinel)).isFile(),
      "repair removed the borrowed checkout's dependency",
    );
    await verifyWorktreeDependencyView(root);
  });
});

test("rejects and repairs the obsolete external source seam", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const source = resolve(root, "web", "src");
    await mkdir(source, { recursive: true });
    await symlink(
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

test("rejects a local package link into another checkout", async () => {
  await withFixture(async ({ root, otherRoot }) => {
    const borrowed = resolve(otherRoot, "components", "app-shell");
    await linkInstalledComponent(
      root,
      borrowed,
    );
    const sentinel = resolve(borrowed, "keep-me");
    await writeFile(sentinel, "other checkout source");

    await assertRejects(
      () => verifyWorktreeDependencyView(root, { requireInstalled: true }),
      "expected this worktree",
    );
    assert(await repairBorrowedWorktreeState(root), "expected a repair");
    assert(
      (await lstat(sentinel)).isFile(),
      "repair removed the other checkout's package source",
    );
    await linkInstalledComponent(
      root,
      resolve(root, "components", "app-shell"),
    );
    await verifyWorktreeDependencyView(root, { requireInstalled: true });
  });
});

test("accepts checkout-local node_modules and workspace package links", async () => {
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
  const base = await mkdtemp(join(tmpdir(), "cowboy-dependency-view-"));
  const root = resolve(base, "current");
  const otherRoot = resolve(base, "other");
  try {
    for (const checkout of [root, otherRoot]) {
      await mkdir(resolve(checkout, "web"), { recursive: true });
      await mkdir(resolve(checkout, "components", "app-shell"), {
        recursive: true,
      });
    }
    await writeFile(
      resolve(root, "package.json"),
      JSON.stringify({ workspaces: ["web", "components/app-shell"] }),
    );
    await writeFile(
      resolve(root, "components", "app-shell", "package.json"),
      JSON.stringify({ name: "@cowboy/app-shell" }),
    );
    await writeFile(
      resolve(root, "web", "package.json"),
      JSON.stringify({
        name: "cowboy-web",
        dependencies: { "@cowboy/app-shell": "workspace:*" },
      }),
    );
    await run({ root, otherRoot });
  } finally {
    await rm(base, { recursive: true });
  }
}

async function linkInstalledComponent(
  root: string,
  component: string,
): Promise<void> {
  const scope = resolve(root, "web", "node_modules", "@cowboy");
  await mkdir(scope, { recursive: true });
  await symlink(component, resolve(scope, "app-shell"));
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
