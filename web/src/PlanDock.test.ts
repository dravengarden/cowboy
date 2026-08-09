import { assertStringIncludes } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./PlanDock.tsx", import.meta.url),
);

Deno.test("Plan progress stays inside the rounded surface without a duplicate desktop focus ring", () => {
  assertStringIncludes(source, 'overflow: "hidden"');
  assertStringIncludes(source, '"&:focus-within": {');
  assertStringIncludes(source, 'boxShadow: "none"');
});
