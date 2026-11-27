import { readFile } from "node:fs/promises";
import { expect, test } from "bun:test";

test("FloatingActionIsland uses unified Cowboy frosted-matte glass with no radial glare or double blur", async () => {
  const bottomSheet = await readFile(new URL("../../components/app-shell/bottom-sheet.tsx", import.meta.url), "utf8");
  const bottomEdge = await readFile(new URL("../../components/app-shell/reader-bottom-edge.ts", import.meta.url), "utf8");
  const contextSummary = await readFile(new URL("../../components/app-shell/sheet-context-summary.tsx", import.meta.url), "utf8");
  const tasksSheet = await readFile(new URL("./tasks/MobileTasksSheet.tsx", import.meta.url), "utf8");

  expect(bottomSheet).toContain("floatingIslandSurfaceSx");
  expect(bottomSheet).not.toContain("radial-gradient");
  expect(bottomSheet).not.toContain("filter: \"blur(7px)\"");
  expect(bottomEdge).toContain("blur(14px) saturate(150%)");
  expect(bottomEdge).toContain("[data-detent-moving] &");
  expect(bottomEdge).toContain("prefers-reduced-transparency: reduce");
  expect(contextSummary).toContain("floatingIslandSurfaceSx");
  expect(tasksSheet).toContain("floatingIslandSurfaceSx");
});

test("non-fullscreen Mobile sheets render a visible grabber pill while full-screen cover sheets omit it", async () => {
  const detentSheet = await readFile(new URL("../../components/app-shell/detent-sheet.tsx", import.meta.url), "utf8");
  const obsidianSheet = await readFile(new URL("./ObsidianSheet.tsx", import.meta.url), "utf8");

  expect(detentSheet).toContain('data-sheet-grabber-pill="true"');
  expect(detentSheet).toContain("isCover ? <Box sx={{ height: 12 }} /> : <SheetGrabberPill />");
  expect(obsidianSheet).toContain('data-sheet-grabber-pill="true"');
  expect(obsidianSheet).toContain("readerBottomEdge(t, t.palette.background.paper)");
  expect(obsidianSheet).toContain("FLOATING_CONTROLS_BOTTOM");
});

test("ContinuityDivider uses structured two-row Mobile card without squished Restore oval", async () => {
  const divider = await readFile(new URL("./ContinuityDivider.tsx", import.meta.url), "utf8");
  expect(divider).toContain('data-continuity-card="mobile"');
  expect(divider).toContain('data-continuity-card="desktop"');
  expect(divider).toContain('whiteSpace: "nowrap"');
  expect(divider).toContain('borderRadius: "14px"');
});
