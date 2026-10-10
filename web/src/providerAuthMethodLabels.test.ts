import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

for (
  const [provider, expectedLabel] of [
    ["codex", "ChatGPT subscription"],
    ["claude-code", "Claude subscription"],
    ["grok", "Grok subscription"],
  ] as const
) {
  test(`${provider} account auth names the subscription product`, async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(`../../plugins/${provider}/provider.json`, import.meta.url), "utf8",
      ),
    ) as { authentication: { methods: Array<{ flow: string; label: string }> } };
    const account = manifest.authentication.methods.find((method) =>
      method.flow !== "secret_input"
    );
    assertEquals(account?.label, expectedLabel);
  });
}
