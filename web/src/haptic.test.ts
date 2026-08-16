import { assertEquals } from "jsr:@std/assert";
import { hapticStyleForIntent } from "./hapticIntent.ts";

const appSource = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Cowboy haptic intents preserve the product strength hierarchy", () => {
  assertEquals(hapticStyleForIntent("navigation"), "selection");
  assertEquals(hapticStyleForIntent("magnetic"), "selection");
  assertEquals(hapticStyleForIntent("confirmation"), "light");
  assertEquals(hapticStyleForIntent("important"), "heavy");
});

Deno.test("browse surfaces opt into the quietest tap", () => {
  assertEquals(appSource.includes('data-haptic="selection"'), true);
  assertEquals(
    transcriptSource.includes('"data-haptic": "selection"'),
    true,
  );
});
