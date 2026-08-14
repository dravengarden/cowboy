import { assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(new URL("./PromptOriginNote.tsx", import.meta.url));

Deno.test("agent runtime notes sit on the left with the provider mark", () => {
  assertEquals(source.includes('alignSelf: "stretch"'), true);
  assertEquals(source.includes("borderRadius: 1"), true);
  assertEquals(source.includes("<ProviderIcon"), true);
  assertEquals(source.includes('data-prompt-origin-actor="agent"'), true);
  assertEquals(source.includes('alignSelf: "flex-end"'), true);
});
