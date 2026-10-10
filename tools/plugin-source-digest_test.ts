import { Command } from "./lib/command.ts";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import {
  assertEquals,
  assertNotEquals,
  assertRejects,
} from "@std/assert";
import {
  filesDigest,
  repositorySourceFiles,
} from "./check-plugin-components.ts";

test("Plugin source fingerprints exclude build caches but include new and changed source", async () => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-plugin-source-test-"));
  try {
    const git = async (...args: string[]) => {
      const output = await new Command("git", {
        args,
        cwd: root,
        stdout: "piped",
        stderr: "piped",
      }).output();
      assertEquals(
        output.success,
        true,
        new TextDecoder().decode(output.stderr),
      );
    };
    await git("init", "--quiet");
    await mkdir(`${root}/plugin/adapter/target`, { recursive: true });
    await writeFile(`${root}/.gitignore`, "target/\n");
    await writeFile(`${root}/plugin/plugin.json`, '{"id":"test"}');
    await git("add", ".gitignore", "plugin/plugin.json");
    const before = await filesDigest(
      await repositorySourceFiles("plugin", root),
    );
    await writeFile(
      `${root}/plugin/adapter/target/cache`,
      "generated output",
    );
    assertEquals(
      await filesDigest(await repositorySourceFiles("plugin", root)),
      before,
    );
    await writeFile(
      `${root}/plugin/new.ts`,
      "export const value = 1;",
    );
    const withNew = await filesDigest(
      await repositorySourceFiles("plugin", root),
    );
    assertNotEquals(withNew, before);
    await git("add", "plugin/new.ts");
    assertEquals(
      await filesDigest(await repositorySourceFiles("plugin", root)),
      withNew,
    );
    await writeFile(
      `${root}/plugin/new.ts`,
      "export const value = 2;",
    );
    assertNotEquals(
      await filesDigest(await repositorySourceFiles("plugin", root)),
      withNew,
    );
    await symlink(`${root}/plugin/new.ts`, `${root}/plugin/alias.ts`);
    await assertRejects(
      () => filesDigest([`${root}/plugin/alias.ts`]),
      Error,
      "regular file",
    );
    await assertRejects(
      () => repositorySourceFiles("missing", root),
      Error,
      "no release sources",
    );
  } finally {
    await rm(root, { recursive: true });
  }
});
