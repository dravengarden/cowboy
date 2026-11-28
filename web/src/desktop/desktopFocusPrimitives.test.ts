import { readFile } from "node:fs/promises";
import { expect, test } from "bun:test";
import { createTheme } from "@mui/material";
import {
  desktopAuxRegionFocusBg,
  desktopControlFocusRing,
  desktopFocusEdgeColor,
  desktopFocusHaloColor,
  desktopFocusRootSx,
  desktopRegionFocusBoxShadow,
  PROMOTABLE_DESKTOP_PANE_SELECTOR,
  PROMOTED_DESKTOP_PANE_SELECTOR,
} from "./desktopFocusPrimitives.ts";
import {
  isDesktopRegionVisible,
  resolvePaneEntryRegion,
  resolveVisiblePointerRegion,
  resolveVisibleRegionElement,
} from "./desktopFocusResolution.ts";

test("desktopFocusPrimitives applies one paint-only perimeter frame to every Desktop region with zero exclusions", () => {
  const light = createTheme({ palette: { mode: "light", primary: { main: "#6d4aff" } } });
  const dark = createTheme({ palette: { mode: "dark", primary: { main: "#b99af3" } } });

  expect(desktopFocusEdgeColor(light)).toContain("color-mix(in srgb");
  expect(desktopFocusEdgeColor(light)).toContain("38%");
  expect(desktopFocusEdgeColor(dark)).toContain("color-mix(in srgb");
  expect(desktopFocusEdgeColor(dark)).toContain("44%");
  expect(desktopRegionFocusBoxShadow(light)).toContain("inset 0 0 6px 0");
  expect(desktopRegionFocusBoxShadow(dark)).toContain("inset 0 0 6px 0");
  expect(desktopFocusHaloColor(light)).toContain("rgba");
  expect(desktopAuxRegionFocusBg(dark)).toContain("rgba");
  expect(desktopControlFocusRing(light)).toContain("0 0 0 2px");

  const root = desktopFocusRootSx();
  const keys = Object.keys(root);
  // Universal selector must cover ALL regions with zero `:not(...)` exclusions
  expect(keys.some((k) => k.includes(":not([data-desktop-region="))).toBe(false);
  expect(root["& [data-desktop-region]"]).toBeDefined();
  expect(root["& [data-desktop-region][data-desktop-focused='true']::after"]).toBeDefined();

  // Base `::after` overlay must be strictly paint-only with hardware-snapped 1px border (zero layout shift)
  const regionRule = root["& [data-desktop-region]"] as Record<string, unknown>;
  const afterRule = regionRule["&::after"] as Record<string, unknown>;
  expect(afterRule.position).toBe("absolute");
  expect(afterRule.inset).toBe(0);
  expect(afterRule.boxSizing).toBe("border-box");
  expect(afterRule.border).toBe("1px solid transparent");
  expect(afterRule.pointerEvents).toBe("none");
  expect(afterRule.borderRadius).toBe("inherit");
  const focusedAfter = root["& [data-desktop-region][data-desktop-focused='true']::after"] as Record<string, unknown>;
  expect(focusedAfter.borderColor).toBe(desktopFocusEdgeColor);
  expect(focusedAfter.boxShadow).toBe(desktopRegionFocusBoxShadow);

  // Single-region panes promote the identical paint-only perimeter frame onto
  // the outer pane (enclosing PaneHeader) and suppress the inner region frame.
  const promotablePane = root[PROMOTABLE_DESKTOP_PANE_SELECTOR] as Record<string, unknown>;
  expect(promotablePane.position).toBe("relative");
  expect(promotablePane["&::after"]).toEqual(afterRule);
  const promotedAfter = root[`${PROMOTED_DESKTOP_PANE_SELECTOR}::after`] as Record<string, unknown>;
  expect(promotedAfter.borderColor).toBe(desktopFocusEdgeColor);
  expect(promotedAfter.boxShadow).toBe(desktopRegionFocusBoxShadow);
  const suppressedInner = root[
    `${PROMOTED_DESKTOP_PANE_SELECTOR} [data-desktop-region][data-desktop-focused='true']::after`
  ] as Record<string, unknown>;
  expect(suppressedInner.borderColor).toBe("transparent");
  expect(suppressedInner.boxShadow).toBe("none");
});

test("all Desktop workspaces mount desktopFocusRootSx, avoid duplicate DraftEditor regions, and keep scrollable regions wrapped cleanly", async () => {
  const [appSrc, draftWsSrc, draftEditorSrc, tasksSrc, collapseSrc, wsSrc] = await Promise.all([
    readFile(new URL("../App.tsx", import.meta.url), "utf8"),
    readFile(new URL("../documents/DraftWorkspace.tsx", import.meta.url), "utf8"),
    readFile(new URL("../documents/DraftEditor.tsx", import.meta.url), "utf8"),
    readFile(new URL("../tasks/DesktopTasksView.tsx", import.meta.url), "utf8"),
    readFile(new URL("./DesktopPaneCollapse.tsx", import.meta.url), "utf8"),
    readFile(new URL("./DesktopWorkspace.tsx", import.meta.url), "utf8"),
  ]);

  expect(appSrc).toContain("...desktopFocusRootSx(CURRENT_ROW_FILL)");
  expect(draftWsSrc).toContain("desktopFocusRootSx()");
  // DraftEditor must declare only ONE prompt.composer region (no nested double frame)
  const draftRegionMatches = draftEditorSrc.match(/data-desktop-region=/g) ?? [];
  expect(draftRegionMatches.length).toBe(1);
  // tasks.roster and sessions.rail must be non-scrolling hosts (`overflow: "hidden"`)
  expect(tasksSrc).toMatch(/data-desktop-region="tasks\.roster"[\s\S]*?overflow: "hidden"/);
  expect(collapseSrc).toMatch(/data-desktop-region="sessions\.rail"[\s\S]*?overflow: "hidden"/);
  // All 3 panes (Sessions in App.tsx, Prompt & Conversation in DesktopWorkspace.tsx) render data-desktop-pane-dot
  expect(appSrc).toContain("data-desktop-pane-dot");
  expect(wsSrc).toContain("data-desktop-pane-dot");
  // Stable workspace root marker spans all 3 panes across compact->wide resizes
  expect(appSrc).toContain("data-desktop-workspace-root");
  expect(draftWsSrc).toContain("data-desktop-workspace-root");
  // Portaled DesktopModal regions (e.g. Page Index) mount shared focus primitives
  const [modalSrc, reviewSrc, tabStripSrc, controllerSrc, cmdProviderSrc, workViewSrc] = await Promise.all([
    readFile(new URL("./DesktopModal.tsx", import.meta.url), "utf8"),
    readFile(new URL("./review/DesktopReviewView.tsx", import.meta.url), "utf8"),
    readFile(new URL("./DesktopTabStrip.tsx", import.meta.url), "utf8"),
    readFile(new URL("./DesktopWorkspaceController.tsx", import.meta.url), "utf8"),
    readFile(new URL("./commands/DesktopCommandProvider.tsx", import.meta.url), "utf8"),
    readFile(new URL("./review/useDesktopWorkView.ts", import.meta.url), "utf8"),
  ]);
  expect(modalSrc).toContain("desktopFocusRootSx()");
  // Hidden mounted views carry the native `hidden` attribute so pure-CSS
  // single-region pane promotion accurately ignores hidden siblings.
  expect(wsSrc).toMatch(/data-desktop-region="conversation\.transcript"[\s\S]*?hidden=\{!conversationShown\}/);
  expect(reviewSrc).toMatch(/data-desktop-region=\{REVIEW_INSPECTOR_REGION\}[\s\S]*?hidden=\{!panel\.shown\}/);
  // Inset Inspector keeps position:absolute via host attribute without losing ::after frame
  expect(reviewSrc).toContain('data-desktop-region-position={panel.beside ? undefined : "absolute"}');
  const root = desktopFocusRootSx();
  const regionRule = root["& [data-desktop-region]"] as Record<string, unknown>;
  expect(regionRule["&[data-desktop-region-position='absolute']"]).toEqual({ position: "absolute" });
  const focusedAfter = root["& [data-desktop-region][data-desktop-focused='true']::after"] as Record<string, unknown>;
  expect(focusedAfter["@media (forced-colors: active)"]).toBeDefined();
  // Row-1 top strip height is shared across App Sessions bar, collapsed rail head, and DesktopTabStrip
  expect(appSrc).toContain("DESKTOP_TOP_STRIP_HEIGHT");
  expect(collapseSrc).toContain("DESKTOP_TOP_STRIP_HEIGHT");
  expect(tabStripSrc).toContain("DESKTOP_TOP_STRIP_HEIGHT");
  // Controller scopes pane chrome to [data-desktop-workspace-root], Top Bar Ctrl+J routes via focusAdjacentRegion(1)
  expect(controllerSrc).toContain("[data-desktop-workspace-root]");
  expect(cmdProviderSrc).toContain('workspace.focusedRegion === "topbar.controls" && motion === "j"');
  expect(workViewSrc).toContain("isDesktopRegionVisible");
});

test("desktopFocusResolution resolves visible sub-regions and remembers last focused sub-region per pane", () => {
  // Minimal DOM stub matching the Element / HTMLElement API used by resolution helpers
  interface StubEl {
    dataset: Record<string, string | undefined>;
    hidden: boolean;
    isConnected: boolean;
    offsetParent: object | null;
    parentElement: StubEl | null;
    display: string;
    children: StubEl[];
    closest: (sel: string) => StubEl | null;
    querySelectorAll: (sel: string) => StubEl[];
  }
  const makeEl = (
    dataset: Record<string, string>,
    opts: { hidden?: boolean; display?: string; parent?: StubEl | null } = {},
  ): StubEl => {
    const el: StubEl = {
      dataset,
      hidden: opts.hidden ?? false,
      isConnected: true,
      offsetParent: opts.display === "none" || opts.hidden ? null : {},
      parentElement: opts.parent ?? null,
      display: opts.display ?? "flex",
      children: [],
      closest(sel: string) {
        let cur: StubEl | null = el;
        while (cur) {
          if (sel === "[hidden]" && cur.hidden) return cur;
          if (sel === "[data-desktop-region]" && cur.dataset.desktopRegion) return cur;
          if (sel === "[data-desktop-pane]" && cur.dataset.desktopPane) return cur;
          cur = cur.parentElement;
        }
        return null;
      },
      querySelectorAll(sel: string) {
        const out: StubEl[] = [];
        const walk = (node: StubEl) => {
          for (const child of node.children) {
            if (sel === "[data-desktop-region]" && child.dataset.desktopRegion) out.push(child);
            else if (sel.startsWith('[data-desktop-pane="') && child.dataset.desktopPane === sel.slice(20, -2)) out.push(child);
            else if (sel.startsWith('[data-desktop-region="') && child.dataset.desktopRegion === sel.slice(22, -2)) out.push(child);
            walk(child);
          }
        };
        walk(el);
        return out;
      },
    };
    if (opts.parent) opts.parent.children.push(el);
    return el;
  };

  const origGetComputedStyle = globalThis.getComputedStyle;
  const origElement = globalThis.Element;
  const origCSS = globalThis.CSS;
  try {
    Object.defineProperty(globalThis, "CSS", {
      value: { escape: (v: string) => v },
      configurable: true,
    });
    Object.defineProperty(globalThis, "getComputedStyle", {
      value: (el: StubEl) => ({ display: el.display, visibility: "visible", position: "relative" }),
      configurable: true,
    });

    const doc = makeEl({});
    const convPane = makeEl({ desktopPane: "conversation" }, { parent: doc });
    const convHeader = makeEl({}, { parent: convPane });
    const transcript = makeEl({ desktopRegion: "conversation.transcript" }, { parent: convPane, display: "none" });
    const reviewWrap = makeEl({}, { parent: convPane });
    const reviewNav = makeEl({ desktopRegion: "review.navigator" }, { parent: reviewWrap });
    const reviewDoc = makeEl({ desktopRegion: "review.document" }, { parent: reviewWrap });

    expect(isDesktopRegionVisible(transcript as unknown as HTMLElement)).toBe(false);
    expect(isDesktopRegionVisible(reviewDoc as unknown as HTMLElement)).toBe(true);

    // When Review is open (transcript display:none), entering Conversation picks review.document (not hidden transcript!)
    expect(resolvePaneEntryRegion("conversation", null, doc as unknown as ParentNode)).toBe("review.document");

    // When last focused sub-region was review.navigator and it's still visible, restore review.navigator
    expect(resolvePaneEntryRegion("conversation", "review.navigator", doc as unknown as ParentNode)).toBe("review.navigator");

    // If review.navigator becomes hidden, fall back to visible review.document
    reviewNav.display = "none";
    reviewNav.offsetParent = null;
    expect(resolvePaneEntryRegion("conversation", "review.navigator", doc as unknown as ParentNode)).toBe("review.document");

    // resolveVisibleRegionElement skips hidden duplicate and picks visible one
    expect(resolveVisibleRegionElement("review.document", doc as unknown as ParentNode)).toBe(reviewDoc as unknown as HTMLElement);

    // Clicking Conversation pane header while Review is open resolves to visible review.document
    class FakeElement {}
    Object.setPrototypeOf(convHeader, FakeElement.prototype);
    Object.defineProperty(globalThis, "Element", { value: FakeElement, configurable: true });
    expect(
      resolveVisiblePointerRegion(convHeader as unknown as EventTarget, {}, doc as unknown as ParentNode),
    ).toBe("review.document");
  } finally {
    Object.defineProperty(globalThis, "getComputedStyle", { value: origGetComputedStyle, configurable: true });
    Object.defineProperty(globalThis, "Element", { value: origElement, configurable: true });
    Object.defineProperty(globalThis, "CSS", { value: origCSS, configurable: true });
  }
});
