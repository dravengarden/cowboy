import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);
const treeSource = await Deno.readTextFile(
  new URL("./ReviewFileTree.tsx", import.meta.url),
);
const storeSource = await Deno.readTextFile(
  new URL("../../store.ts", import.meta.url),
);

Deno.test("review sheets portal off the product-pager containing block", () => {
  const symbolsAt = source.indexOf('? `Symbols · ${inspectCandidates.length}`');
  assertEquals(symbolsAt >= 0, true);
  const symbolsSheet = source.slice(symbolsAt, symbolsAt + 240);
  assertStringIncludes(symbolsSheet, "portal");
  assertStringIncludes(symbolsSheet, 'mobileDismiss="footer"');
  const contextAt = source.indexOf(
    'title={selectedContextProject?.label ?? "Context"}',
  );
  assertEquals(contextAt >= 0, true);
  assertStringIncludes(source.slice(contextAt, contextAt + 180), "portal");
});

Deno.test("symbol navigation disables actions without destinations", () => {
  for (
    const kind of [
      "definition",
      "declaration",
      "typeDefinition",
      "implementation",
      "references",
    ]
  ) {
    assertStringIncludes(source, `navigationButton("${kind}"`);
  }
  assertStringIncludes(
    source,
    'result.locations.length > 0 ? "available" : "unavailable"',
  );
  assertStringIncludes(source, 'disabled={availability !== "available"}');
  assertStringIncludes(source, "data-navigation-status={availability}");
});

Deno.test("mobile symbol choices separate the current symbol from a tidy responsive grid", () => {
  assertStringIncludes(source, "data-mobile-symbol-current");
  assertStringIncludes(source, "data-mobile-symbol-grid");
  assertStringIncludes(
    source,
    'gridTemplateColumns: "repeat(auto-fit, minmax(7rem, 1fr))"',
  );
  assertStringIncludes(source, 'minHeight: 44');
  assertEquals(
    source.includes(
      'flexWrap={inspectCandidatesExpanded ? "wrap" : "nowrap"}',
    ),
    false,
  );
});

Deno.test("session context leads with the active session without a history row", () => {
  assertEquals(source.includes("function ContextPreviousSessionRow"), false);
  assertEquals(source.includes('label = "Previous session"'), false);
  assertStringIncludes(
    source,
    'useState<"sessions" | "projects">(\n    "sessions",',
  );
  assertStringIncludes(source, "Current session");
  assertStringIncludes(source, 'currentSession ? "Other sessions" : "Sessions"');
  assertStringIncludes(
    source,
    "sessions.filter((session) => session.id !== activeSessionId)",
  );
});

Deno.test("session context list consumes duplicate floating-footer clearance", () => {
  assertStringIncludes(
    source,
    '"calc(100dvh - 148px + 76px + env(safe-area-inset-bottom, 0px))"',
  );
  assertStringIncludes(
    source,
    'mb: "calc(-76px - env(safe-area-inset-bottom, 0px))"',
  );
  assertStringIncludes(
    source,
    'pb: "calc(88px + env(safe-area-inset-bottom, 0px))"',
  );
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
