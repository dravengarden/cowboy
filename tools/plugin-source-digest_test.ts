import {
  assertEquals,
  assertNotEquals,
  assertRejects,
} from "jsr:@std/assert@1.0.19";
import {
  filesDigest,
  repositorySourceFiles,
} from "./check-plugin-components.ts";

Deno.test("Plugin source fingerprints exclude build caches but include new and changed source", async () => {
  const root = await Deno.makeTempDir({ prefix: "cowboy-plugin-source-test-" });
  try {
    const git = async (...args: string[]) => {
      const output = await new Deno.Command("git", {
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
    await Deno.mkdir(`${root}/plugin/adapter/target`, { recursive: true });
    await Deno.writeTextFile(`${root}/.gitignore`, "target/\n");
    await Deno.writeTextFile(`${root}/plugin/plugin.json`, '{"id":"test"}');
    await git("add", ".gitignore", "plugin/plugin.json");
    const before = await filesDigest(
      await repositorySourceFiles("plugin", root),
    );
    await Deno.writeTextFile(
      `${root}/plugin/adapter/target/cache`,
      "generated output",
    );
    assertEquals(
      await filesDigest(await repositorySourceFiles("plugin", root)),
      before,
    );
    await Deno.writeTextFile(
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
    await Deno.writeTextFile(
      `${root}/plugin/new.ts`,
      "export const value = 2;",
    );
    assertNotEquals(
      await filesDigest(await repositorySourceFiles("plugin", root)),
      withNew,
    );
    await Deno.symlink(`${root}/plugin/new.ts`, `${root}/plugin/alias.ts`);
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
    await Deno.remove(root, { recursive: true });
  }
});
