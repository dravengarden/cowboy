import { assertEquals } from "jsr:@std/assert";
import {
  LITERAL_SLASH_GUARD,
  prepareUserPrompt,
} from "./slashCommandIntent.ts";

Deno.test("explicitly selected slash commands remain executable", () => {
  assertEquals(prepareUserPrompt("/compact now", "compact"), "/compact now");
  assertEquals(prepareUserPrompt("/review", "review"), "/review");
});

Deno.test("typed leading slashes remain literal", () => {
  assertEquals(prepareUserPrompt("/", null), `${LITERAL_SLASH_GUARD}/`);
  assertEquals(prepareUserPrompt("/dir", null), `${LITERAL_SLASH_GUARD}/dir`);
  assertEquals(
    prepareUserPrompt("/home/draven/project", null),
    `${LITERAL_SLASH_GUARD}/home/draven/project`,
  );
  assertEquals(
    prepareUserPrompt("  /tmp/result", null),
    `  ${LITERAL_SLASH_GUARD}/tmp/result`,
  );
});

Deno.test("stale completion intent cannot authorize different text", () => {
  assertEquals(
    prepareUserPrompt("/home", "compact"),
    `${LITERAL_SLASH_GUARD}/home`,
  );
});

Deno.test("non-leading slashes and an existing guard are unchanged", () => {
  assertEquals(prepareUserPrompt("open /tmp/result", null), "open /tmp/result");
  const guarded = `${LITERAL_SLASH_GUARD}/dir`;
  assertEquals(prepareUserPrompt(guarded, null), guarded);
});
