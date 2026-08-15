import { assertEquals } from "jsr:@std/assert";
import { groupProviderAuthentications } from "./providerAuthenticationGroups.ts";

Deno.test("credential families remain one group when a future Provider joins", () => {
  const groups = groupProviderAuthentications([
    {
      provider_id: "claude-deepseek",
      authentication_scope: "deepseek-api-key-v1",
    },
    {
      provider_id: "codex-deepseek",
      authentication_scope: "deepseek-api-key-v1",
    },
    {
      provider_id: "deepseek-harness",
      authentication_scope: "deepseek-api-key-v1",
    },
    { provider_id: "gemini", authentication_scope: "gemini-auth-v1" },
  ]);

  assertEquals(groups, [
    {
      authenticationScope: "deepseek-api-key-v1",
      entries: [
        {
          provider_id: "claude-deepseek",
          authentication_scope: "deepseek-api-key-v1",
        },
        {
          provider_id: "codex-deepseek",
          authentication_scope: "deepseek-api-key-v1",
        },
        {
          provider_id: "deepseek-harness",
          authentication_scope: "deepseek-api-key-v1",
        },
      ],
    },
    {
      authenticationScope: "gemini-auth-v1",
      entries: [
        { provider_id: "gemini", authentication_scope: "gemini-auth-v1" },
      ],
    },
  ]);
});
