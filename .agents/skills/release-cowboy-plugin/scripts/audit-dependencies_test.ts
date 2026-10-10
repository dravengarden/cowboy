import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { discoverProviderIds } from "./audit-dependencies.ts";

test("all audit discovers only directories with Provider manifests", async () => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-provider-audit-"));
  try {
    for (const name of ["grok", "claude-code", "runtime-packages"]) {
      await mkdir(`${root}/${name}`);
    }
    await writeFile(`${root}/grok/provider.json`, "{}");
    await writeFile(`${root}/claude-code/provider.json`, "{}");
    await writeFile(`${root}/README.md`, "not a Provider");

    assertEquals(discoverProviderIds(new URL(`file://${root}/`)), [
      "claude-code",
      "grok",
    ]);
  } finally {
    await rm(root, { recursive: true });
  }
});
