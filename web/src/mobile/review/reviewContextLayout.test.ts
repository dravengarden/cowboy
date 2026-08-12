import { assertStringIncludes } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);
const treeSource = await Deno.readTextFile(
  new URL("./ReviewFileTree.tsx", import.meta.url),
);
const storeSource = await Deno.readTextFile(
  new URL("../../store.ts", import.meta.url),
);

Deno.test("previous-session navigation stays a compact fixed-height row", () => {
  const start = source.indexOf("function ContextPreviousSessionRow");
  const end = source.indexOf("type ReviewTarget", start);
  const component = source.slice(start, end);

  assertStringIncludes(component, 'height: 52');
  assertStringIncludes(component, 'flex: "0 0 52px"');
  assertStringIncludes(component, 'bgcolor: "transparent"');
});

Deno.test("project targets do not inherit worktree-only labels", () => {
  assertStringIncludes(source, 'contextLabel={projectCodeContext ? "Project code" : "Worktree"}');
  assertStringIncludes(source, 'Open context. Current project code');
  assertStringIncludes(source, '`Project code · ${projectCodeContext.project}`');
  assertStringIncludes(treeSource, 'contextLabel = "Worktree"');
  assertStringIncludes(treeSource, "{contextLabel}");
});

Deno.test("registered project code does not enter the session review sync channel", () => {
  assertStringIncludes(storeSource, 'if (sessionId.startsWith("workspace::")) {');
  assertStringIncludes(storeSource, "const mutate = mobileReviewMutators[name]");
  assertStringIncludes(storeSource, "const value = mutate(current, args)");
});
