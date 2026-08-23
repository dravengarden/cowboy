import { assertEquals } from "jsr:@std/assert";
import { discoverProviderIds } from "./audit-dependencies.ts";

Deno.test("all audit discovers only directories with Provider manifests", async () => {
  const root = await Deno.makeTempDir({ prefix: "cowboy-provider-audit-" });
  try {
    for (const name of ["grok", "claude-code", "runtime-packages"]) {
      await Deno.mkdir(`${root}/${name}`);
    }
    await Deno.writeTextFile(`${root}/grok/provider.json`, "{}");
    await Deno.writeTextFile(`${root}/claude-code/provider.json`, "{}");
    await Deno.writeTextFile(`${root}/README.md`, "not a Provider");

    assertEquals(discoverProviderIds(new URL(`file://${root}/`)), [
      "claude-code",
      "grok",
    ]);
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});
