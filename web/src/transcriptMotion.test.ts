import { assert, assertEquals } from "jsr:@std/assert";

const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Codex activity copy fades without moving the text baseline", () => {
  const animation = transcriptSource.match(
    /const codexPhraseFade = keyframes`([\s\S]*?)`;/,
  )?.[1];
  assert(animation);
  assertEquals(animation.includes("translateY"), false);
});
