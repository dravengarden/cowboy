import { createHash } from "node:crypto";
import { lstat, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * A `node_npm` recipe whose program is Cowboy-owned source rather than one
 * registry package: its dependency source is
 * `cowboy:components/provider-runtime/<package_dir>`, every npm input stays
 * exactly pinned in its own lock, and the owned files below are copied into
 * the archive's `app/` beside the launcher.
 */
export interface OwnedNodeRecipe {
  package_dir: string;
  launcher?: string;
  /** Plain file names in `package_dir`, shipped as `app/<name>`. */
  files?: string[];
  /** Plain file names in `components/provider-runtime`, shipped as `app/<name>`. */
  shared_files?: string[];
}

const PLAIN = /^[a-z0-9][a-z0-9._-]*$/;

export function ownedNodeSource(recipe: OwnedNodeRecipe): string {
  return `cowboy:components/provider-runtime/${recipe.package_dir}`;
}

/** Runtime-root-relative sources in integrity order, validated. */
export function ownedNodeInputs(recipe: OwnedNodeRecipe): string[] {
  const files = recipe.files ?? [];
  const shared = recipe.shared_files ?? [];
  const names = [...files, ...shared];
  if (
    !/^packages\/[a-z0-9-]+$/.test(recipe.package_dir) ||
    recipe.launcher !== `${recipe.package_dir}/launch.mjs` ||
    !names.every((name) => PLAIN.test(name) && name !== "launch.mjs") ||
    new Set(names).size !== names.length
  ) {
    throw new Error(`owned node package ${recipe.package_dir} has unsafe files`);
  }
  return [
    `${recipe.package_dir}/package.json`,
    `${recipe.package_dir}/package-lock.json`,
    recipe.launcher,
    ...files.map((name) => `${recipe.package_dir}/${name}`),
    ...shared,
  ];
}

/** SHA-256 over each owned input's relative path and SHA-256, in order. */
export async function ownedNodeIntegrity(
  runtimeRoot: string,
  recipe: OwnedNodeRecipe,
): Promise<string> {
  let lines = "";
  for (const input of ownedNodeInputs(recipe)) {
    const path = join(runtimeRoot, input);
    if (!(await lstat(path)).isFile()) {
      throw new Error(`owned node input ${input} is not a regular file`);
    }
    const digest = createHash("sha256")
      .update(await readFile(path))
      .digest("hex");
    lines += `${input} ${digest}\n`;
  }
  return `sha256:${createHash("sha256").update(lines).digest("hex")}`;
}

/**
 * Owned modules import the shared provider-runtime modules by their source
 * path (`../../<name>`); in the archive they sit beside them in `app/`.
 */
export function rewriteSharedImports(source: string, shared: string[]): string {
  let rewritten = source;
  for (const name of shared) {
    rewritten = rewritten.replaceAll(`"../../${name}"`, `"./${name}"`);
  }
  if (/["']\.\.\//.test(rewritten)) {
    throw new Error("owned node module imports outside its archive");
  }
  return rewritten;
}

/**
 * Copy a `node_npm` launcher into `app/cowboy-launch.mjs`, and for a
 * Cowboy-owned package (verified against its integrity) its files and the
 * shared modules it imports.
 */
export async function copyOwnedNode(
  runtimeRoot: string,
  recipe: OwnedNodeRecipe,
  dependency: { id: string; source: string; integrity: string },
  app: string,
): Promise<void> {
  const owned = dependency.source === ownedNodeSource(recipe);
  if (owned && dependency.integrity !== (await ownedNodeIntegrity(runtimeRoot, recipe))) {
    throw new Error(`owned node source mismatch for ${dependency.id}`);
  }
  const shared = owned ? (recipe.shared_files ?? []) : [];
  const copies: Array<[string, string]> = [
    [recipe.launcher ?? "", "cowboy-launch.mjs"],
    ...(owned ? (recipe.files ?? []) : []).map((name): [string, string] => [
      `${recipe.package_dir}/${name}`,
      name,
    ]),
    ...shared.map((name): [string, string] => [name, name]),
  ];
  for (const [source, name] of copies) {
    const text = await readFile(join(runtimeRoot, source), "utf8");
    await writeFile(
      join(app, name),
      owned && name.endsWith(".mjs") ? rewriteSharedImports(text, shared) : text,
    );
  }
}

/** SHA-256 over each owned archive file's name and SHA-256, in order. */
export async function ownedArchiveIntegrity(
  sourceDir: string,
  launcher: string,
  files: string[],
): Promise<string> {
  const names = [launcher, ...files];
  if (!names.every((name) => PLAIN.test(name)) || new Set(names).size !== names.length) {
    throw new Error("owned archive files must be distinct plain names");
  }
  let lines = "";
  for (const name of names) {
    const path = join(sourceDir, name);
    if (!(await lstat(path)).isFile()) {
      throw new Error(`owned archive file ${name} is not a regular file`);
    }
    const digest = createHash("sha256")
      .update(await readFile(path))
      .digest("hex");
    lines += `${name} ${digest}\n`;
  }
  return `sha256:${createHash("sha256").update(lines).digest("hex")}`;
}
