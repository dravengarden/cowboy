import { assertEquals } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const providerConfigSource = await Deno.readTextFile(
  new URL("./providerConfigOptions.ts", import.meta.url),
);
const claudeDeepSeekSource = await Deno.readTextFile(
  new URL("../../providers/claude-deepseek/provider.json", import.meta.url),
);

Deno.test("large DeepSeek sessions expose bounded cache-protection status", () => {
  assertEquals(composerSource.includes('"deepseek_cache_protection"'), false);
  assertEquals(
    claudeDeepSeekSource.includes('"id": "deepseek_cache_protection"'),
    true,
  );
  assertEquals(
    claudeDeepSeekSource.includes('"availability": "idle_or_stopped"'),
    true,
  );
  assertEquals(
    providerConfigSource.includes("providerConfigOptionDisabled"),
    true,
  );
  assertEquals(
    composerSource.includes("contextUsed >= DEEPSEEK_CACHE_MIN_HIT_TOKENS"),
    true,
  );
  assertEquals(composerSource.includes("Base ${cacheBaseInterval}"), true);
  assertEquals(composerSource.includes("adaptive learning"), true);
  assertEquals(composerSource.includes("/cache-protection"), true);
  assertEquals(
    composerSource.includes("globalThis.setInterval(refresh, 60_000)"),
    true,
  );
  assertEquals(composerSource.includes("Cache protected"), true);
  assertEquals(composerSource.includes("Cache status unavailable"), true);
});
