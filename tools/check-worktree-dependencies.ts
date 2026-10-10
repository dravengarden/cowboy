import { type Stats } from "node:fs";
import { lstat, readFile, realpath, rm } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

interface PackageManifest {
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
}

interface VerifyOptions {
  requireInstalled?: boolean;
}

export async function repairBorrowedWorktreeState(
  repositoryRoot: string,
): Promise<boolean> {
  const root = await realpath(repositoryRoot);
  let repaired = false;
  const nodeModules = resolve(root, "web", "node_modules");
  const nodeModulesInfo = await lstatOrNull(nodeModules);
  if (nodeModulesInfo?.isSymbolicLink()) {
    await rm(nodeModules);
    console.log(
      "removed borrowed web/node_modules symlink; installing a checkout-local dependency view",
    );
    repaired = true;
  }

  if ((await lstatOrNull(nodeModules))?.isDirectory()) {
    for (const { name, expectedReal } of await localDependencyTargets(root)) {
      const installed = resolve(nodeModules, name);
      const installedInfo = await lstatOrNull(installed);
      if (installedInfo === null) continue;
      const installedReal = await realpath(installed);
      if (installedReal === expectedReal) continue;

      await rm(installed, {
        recursive: installedInfo.isDirectory() && !installedInfo.isSymbolicLink(),
      });
      console.log(
        `removed borrowed ${name} entry; reinstalling this worktree's local package`,
      );
      repaired = true;
    }
  }

  const legacyShell = resolve(root, "web", "src", "_shell");
  if ((await lstatOrNull(legacyShell))?.isSymbolicLink()) {
    await rm(legacyShell);
    console.log("removed obsolete external web/src/_shell source link");
    repaired = true;
  }
  return repaired;
}

export async function verifyWorktreeDependencyView(
  repositoryRoot: string,
  options: VerifyOptions = {},
): Promise<void> {
  const root = await realpath(repositoryRoot);
  const webRoot = resolve(root, "web");
  const legacyShell = resolve(webRoot, "src", "_shell");
  if ((await lstatOrNull(legacyShell))?.isSymbolicLink()) {
    throw new Error(
      "web/src/_shell is an obsolete external source link; run `just install` to remove it",
    );
  }
  const nodeModules = resolve(webRoot, "node_modules");
  const nodeModulesInfo = await lstatOrNull(nodeModules);

  if (nodeModulesInfo === null) {
    if (options.requireInstalled) {
      throw new Error(
        "web/node_modules is missing; run `just install` in this worktree",
      );
    }
    return;
  }

  if (nodeModulesInfo.isSymbolicLink()) {
    throw new Error(
      "web/node_modules must be a checkout-local directory, not a symlink; remove the link and run `just install` in this worktree",
    );
  }
  if (!nodeModulesInfo.isDirectory()) {
    throw new Error("web/node_modules exists but is not a directory");
  }

  const nodeModulesReal = await realpath(nodeModules);
  assertInside(
    root,
    nodeModulesReal,
    "web/node_modules resolves outside this checkout",
  );

  for (const { name, expectedReal } of await localDependencyTargets(root)) {
    const installed = resolve(nodeModules, name);
    const installedInfo = await lstatOrNull(installed);
    if (installedInfo === null) {
      if (options.requireInstalled) {
        throw new Error(`${name} is missing from web/node_modules`);
      }
      continue;
    }

    const installedReal = await realpath(installed);
    if (installedReal !== expectedReal) {
      throw new Error(
        `${name} resolves to ${installedReal}, expected this worktree's ${expectedReal}`,
      );
    }
  }
}

async function localDependencyTargets(
  root: string,
): Promise<Array<{ name: string; expectedReal: string }>> {
  const webRoot = resolve(root, "web");
  const manifest = JSON.parse(
    await readFile(resolve(webRoot, "package.json"), "utf8"),
  ) as PackageManifest;
  const dependencies = {
    ...manifest.dependencies,
    ...manifest.devDependencies,
  };
  const targets: Array<{ name: string; expectedReal: string }> = [];
  const members = await workspaceMembers(root);

  for (const [name, specifier] of Object.entries(dependencies).sort()) {
    if (!specifier.startsWith("workspace:")) continue;
    const expected = members.get(name);
    if (expected === undefined) {
      throw new Error(`${name} is not a workspace member of this checkout`);
    }
    assertInside(
      root,
      expected,
      `${name} points outside this checkout in package.json`,
    );
    const expectedReal = await realpath(expected);
    assertInside(
      root,
      expectedReal,
      `${name} package source resolves outside this checkout`,
    );
    targets.push({ name, expectedReal });
  }
  return targets;
}

/** Workspace member directories by package name, from the root manifest. */
async function workspaceMembers(root: string): Promise<Map<string, string>> {
  const manifest = JSON.parse(
    await readFile(resolve(root, "package.json"), "utf8"),
  ) as { workspaces?: string[] };
  const members = new Map<string, string>();
  for (const member of manifest.workspaces ?? []) {
    const directory = resolve(root, member);
    assertInside(
      root,
      directory,
      `workspace ${member} points outside this checkout`,
    );
    const { name } = JSON.parse(
      await readFile(resolve(directory, "package.json"), "utf8"),
    ) as { name?: string };
    if (name !== undefined) members.set(name, directory);
  }
  return members;
}

function assertInside(root: string, candidate: string, message: string): void {
  const path = relative(root, candidate);
  if (
    path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path)
  ) {
    throw new Error(`${message}: ${candidate}`);
  }
}

async function lstatOrNull(path: string): Promise<Stats | null> {
  try {
    return await lstat(path);
  } catch (error) {
    if ((error as { code?: string }).code === "ENOENT") return null;
    throw error;
  }
}

if (import.meta.main) {
  const repositoryRoot = dirname(dirname(fileURLToPath(import.meta.url)));
  if (process.argv.slice(2).includes("--repair-borrowed-state")) {
    await repairBorrowedWorktreeState(repositoryRoot);
  }
  await verifyWorktreeDependencyView(repositoryRoot, {
    requireInstalled: process.argv.slice(2).includes("--require-installed"),
  });
}
