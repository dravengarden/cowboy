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
  const root = await Deno.realPath(repositoryRoot);
  let repaired = false;
  const nodeModules = resolve(root, "web", "node_modules");
  const nodeModulesInfo = await lstatOrNull(nodeModules);
  if (nodeModulesInfo?.isSymlink) {
    await Deno.remove(nodeModules);
    console.log(
      "removed borrowed web/node_modules symlink; installing a checkout-local dependency view",
    );
    repaired = true;
  }

  if ((await lstatOrNull(nodeModules))?.isDirectory) {
    for (const { name, expectedReal } of await localDependencyTargets(root)) {
      const installed = resolve(nodeModules, name);
      const installedInfo = await lstatOrNull(installed);
      if (installedInfo === null) continue;
      const installedReal = await Deno.realPath(installed);
      if (installedReal === expectedReal) continue;

      await Deno.remove(installed, {
        recursive: installedInfo.isDirectory && !installedInfo.isSymlink,
      });
      console.log(
        `removed borrowed ${name} entry; reinstalling this worktree's local package`,
      );
      repaired = true;
    }
  }

  const legacyShell = resolve(root, "web", "src", "_shell");
  if ((await lstatOrNull(legacyShell))?.isSymlink) {
    await Deno.remove(legacyShell);
    console.log("removed obsolete external web/src/_shell source link");
    repaired = true;
  }
  return repaired;
}

export async function verifyWorktreeDependencyView(
  repositoryRoot: string,
  options: VerifyOptions = {},
): Promise<void> {
  const root = await Deno.realPath(repositoryRoot);
  const webRoot = resolve(root, "web");
  const legacyShell = resolve(webRoot, "src", "_shell");
  if ((await lstatOrNull(legacyShell))?.isSymlink) {
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

  if (nodeModulesInfo.isSymlink) {
    throw new Error(
      "web/node_modules must be a checkout-local directory, not a symlink; remove the link and run `just install` in this worktree",
    );
  }
  if (!nodeModulesInfo.isDirectory) {
    throw new Error("web/node_modules exists but is not a directory");
  }

  const nodeModulesReal = await Deno.realPath(nodeModules);
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

    const installedReal = await Deno.realPath(installed);
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
    await Deno.readTextFile(resolve(webRoot, "package.json")),
  ) as PackageManifest;
  const dependencies = {
    ...manifest.dependencies,
    ...manifest.devDependencies,
  };
  const targets: Array<{ name: string; expectedReal: string }> = [];

  for (const [name, specifier] of Object.entries(dependencies).sort()) {
    if (!specifier.startsWith("file:")) continue;
    const expected = resolve(webRoot, specifier.slice("file:".length));
    assertInside(
      root,
      expected,
      `${name} points outside this checkout in package.json`,
    );
    const expectedReal = await Deno.realPath(expected);
    assertInside(
      root,
      expectedReal,
      `${name} package source resolves outside this checkout`,
    );
    targets.push({ name, expectedReal });
  }
  return targets;
}

function assertInside(root: string, candidate: string, message: string): void {
  const path = relative(root, candidate);
  if (
    path === ".." || path.startsWith(`..${sep}`) || isAbsolute(path)
  ) {
    throw new Error(`${message}: ${candidate}`);
  }
}

async function lstatOrNull(path: string): Promise<Deno.FileInfo | null> {
  try {
    return await Deno.lstat(path);
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return null;
    throw error;
  }
}

if (import.meta.main) {
  const repositoryRoot = dirname(dirname(fileURLToPath(import.meta.url)));
  if (Deno.args.includes("--repair-borrowed-state")) {
    await repairBorrowedWorktreeState(repositoryRoot);
  }
  await verifyWorktreeDependencyView(repositoryRoot, {
    requireInstalled: Deno.args.includes("--require-installed"),
  });
}
