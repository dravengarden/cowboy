import { assertStringIncludes } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);

Deno.test("previous-session navigation stays a compact fixed-height row", () => {
  const start = source.indexOf("function ContextPreviousSessionRow");
  const end = source.indexOf("type ReviewTarget", start);
  const component = source.slice(start, end);

  assertStringIncludes(component, 'height: 52');
  assertStringIncludes(component, 'flex: "0 0 52px"');
  assertStringIncludes(component, 'bgcolor: "transparent"');
});
