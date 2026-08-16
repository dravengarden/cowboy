import { assertEquals } from "jsr:@std/assert";

for (
  const [provider, expectedLabel] of [
    ["codex", "ChatGPT subscription"],
    ["claude-code", "Claude subscription"],
    ["grok", "Grok subscription"],
  ] as const
) {
  Deno.test(`${provider} account auth names the subscription product`, async () => {
    const manifest = JSON.parse(
      await Deno.readTextFile(
        new URL(`../../providers/${provider}/provider.json`, import.meta.url),
      ),
    ) as { authentication: { methods: Array<{ flow: string; label: string }> } };
    const account = manifest.authentication.methods.find((method) =>
      method.flow !== "secret_input"
    );
    assertEquals(account?.label, expectedLabel);
  });
}
