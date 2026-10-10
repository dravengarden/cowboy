import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const runtimeBuilder = await readFile(
  new URL("../components/provider-runtime/build.ts", import.meta.url), "utf8",
);
const runtimeLockChecker = await readFile(
  new URL("../components/provider-runtime/check.ts", import.meta.url), "utf8",
);

for (const provider of ["claude-deepseek", "codex-deepseek"]) {
  test(`${provider} publishes Linux x86_64 and macOS arm64 runtimes`, async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(`../plugins/${provider}/provider.json`, import.meta.url), "utf8",
      ),
    ) as {
      runtime: {
        platforms: Array<{ os: string; architecture: string }>;
      };
    };
    assertEquals(
      manifest.runtime.platforms.map(({ os, architecture }) =>
        `${os}-${architecture}`
      ),
      ["linux-x86_64", "macos-aarch64"],
    );
  });
}

test("static Go gateways map typed Provider targets to Go targets", () => {
  assertEquals(runtimeBuilder.includes('? "linux"'), true);
  assertEquals(runtimeBuilder.includes('? "darwin"'), true);
  assertEquals(runtimeBuilder.includes("`GOOS=${goOperatingSystem}`"), true);
  assertEquals(
    runtimeLockChecker.includes('target === "macos-aarch64"'),
    true,
  );
});
