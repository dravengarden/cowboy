import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "../../../tools/test-assert.ts";

async function readDesktop(relative: string): Promise<string> {
  return await readFile(new URL(relative, import.meta.url), "utf8");
}

test("Review left-truncated paths use BiDi-isolated ReviewPathText", async () => {
  for (
    const file of [
      "./review/DesktopReviewDocument.tsx",
      "./review/ReviewChangesList.tsx",
      "./review/ReviewHistoryList.tsx",
      "./review/ReviewFilesList.tsx",
      "./review/ReviewPullList.tsx",
    ]
  ) {
    const src = await readDesktop(file);
    assertEquals(src.includes('direction: "rtl"'), false);
    assert(src.includes("ReviewPathText"));
  }
  const helper = await readDesktop("./review/ReviewPathText.tsx");
  assert(helper.includes('component="bdi"'));
  assert(helper.includes('dir="ltr"'));
});

test("DesktopTabStrip and Review components use HintTooltip instead of native title attributes", async () => {
  for (
    const file of [
      "./DesktopTabStrip.tsx",
      "./review/DesktopReviewDocument.tsx",
      "./review/ReviewChangesList.tsx",
      "./review/ReviewHistoryList.tsx",
      "./review/ReviewContextMenu.tsx",
      "./review/ReviewWorktreesList.tsx",
      "./review/ReviewPullList.tsx",
    ]
  ) {
    const src = await readDesktop(file);
    // Strip JSX HintTooltip/Tooltip/Section tags' title props and assert no
    // raw DOM element (Box, Tab, Button, Typography, ListItemButton) carries `title=`.
    const nativeTitle = /<(?:Box|Tab|Button|Typography|ListItemButton)\b[^>]*\btitle=/u;
    assertEquals(nativeTitle.test(src), false);
    assert(src.includes("HintTooltip"));
  }
});

test("ReviewFileTabs prevents middle-click autoscroll and supports wheel scroll", async () => {
  const src = await readDesktop("./review/DesktopReviewDocument.tsx");
  assert(src.includes("if (event.button === 1) event.preventDefault();"));
  assert(src.includes(".MuiTabs-scroller"));
  assertEquals(src.includes("<IconButton"), false);
});

test("Command Palette scrolls selected row into view and supports Ctrl+J/K/N/P and empty state", async () => {
  const src = await readDesktop("./commands/DesktopCommandPalette.tsx");
  assert(src.includes("scrollIntoView({ block: \"nearest\" })"));
  assert(src.includes('event.code === "KeyJ" || event.code === "KeyN"'));
  assert(src.includes('event.code === "KeyK" || event.code === "KeyP"'));
  assert(src.includes("data-palette-empty"));
});

test("Shortcuts dialog covers Code Review, leader sequences, and instant filtering", async () => {
  const src = await readDesktop("./commands/DesktopShortcutsDialog.tsx");
  assert(src.includes("const CODE_REVIEW:"));
  assert(src.includes("command.shortcut || command.sequence?.length"));
  assert(src.includes("Filter shortcuts"));
});

test("StatusLine preserves high-priority hints at narrow widths and keeps button typography consistent", async () => {
  const src = await readDesktop("./DesktopStatusLine.tsx");
  assert(src.includes('data-status-hint-priority={highPriority ? "high" : "normal"}'));
  assert(src.includes('"@media (max-width: 640px)": { display: "none" }'));
  // Neither Commands nor Shortcuts passes unused `mono`.
  const commandsIdx = src.indexOf('Open the Desktop command palette');
  const shortcutsIdx = src.indexOf('Open the Desktop keyboard shortcut guide');
  assert(commandsIdx > 0 && shortcutsIdx > commandsIdx);
  assertEquals(src.slice(commandsIdx, shortcutsIdx).includes("mono"), false);
});

test("inputVim r command refuses to replace newlines or past-end positions", async () => {
  const src = await readDesktop("./vim/inputVim.ts");
  assert(src.includes('value[pos] === "\\n"'));
  assert(src.includes("pos >= value.length"));
});
