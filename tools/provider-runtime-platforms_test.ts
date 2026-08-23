import { assertEquals } from "jsr:@std/assert";

const runtimeBuilder = await Deno.readTextFile(
  new URL("./build-provider-runtime.ts", import.meta.url),
);
const runtimeLockChecker = await Deno.readTextFile(
  new URL("./check-provider-runtime-lock.ts", import.meta.url),
);

for (const provider of ["claude-deepseek", "codex-deepseek"]) {
  Deno.test(`${provider} publishes Linux x86_64 and macOS arm64 runtimes`, async () => {
    const manifest = JSON.parse(
      await Deno.readTextFile(
        new URL(`../plugins/${provider}/provider.json`, import.meta.url),
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

Deno.test("static Go gateways map typed Provider targets to Go targets", () => {
  assertEquals(runtimeBuilder.includes('? "linux"'), true);
  assertEquals(runtimeBuilder.includes('? "darwin"'), true);
  assertEquals(runtimeBuilder.includes("`GOOS=${goOperatingSystem}`"), true);
  assertEquals(
    runtimeLockChecker.includes('target === "macos-aarch64"'),
    true,
  );
});
