import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const appSource = await readFile(
  new URL("../App.tsx", import.meta.url), "utf8",
);
const workspaceSource = await readFile(
  new URL("./DesktopWorkspace.tsx", import.meta.url), "utf8",
);
const commandsSource = await readFile(
  new URL("./commands/DesktopCommandProvider.tsx", import.meta.url), "utf8",
);
const statusSource = await readFile(
  new URL("./DesktopStatusLine.tsx", import.meta.url), "utf8",
);

test("every Desktop vertical boundary exposes the shared splitter contract", () => {
  assert(appSource.includes('data-desktop-splitter="sessions-prompt"'));
  assert(workspaceSource.includes('data-desktop-splitter="prompt-conversation"'));
  assertEquals(
    (appSource + workspaceSource).match(/<DesktopSplitterHint \/>/gu)?.length,
    2,
  );
});

test("the workspace prefix enters an exclusive H/L Resize mode", () => {
  assert(commandsSource.includes("leaderArmed.current"));
  assert(commandsSource.includes("DESKTOP_WORKSPACE_COMMANDS"));
  assert(commandsSource.includes("matchesDesktopWorkspacePrefix(event)"));
  assert(commandsSource.includes("key !== null && isModifierKey(key)"));
  assert(!commandsSource.includes('key.toLowerCase() === "r"'));
  assert(commandsSource.includes("delta: left ? -step : step"));
  assert(commandsSource.includes('lower === "h" || lower === "l"'));
  assert(commandsSource.includes("DESKTOP_SPLITTER_LARGE_STEP"));
  assert(commandsSource.includes("adjacentDesktopSplitter("));
  assert(commandsSource.includes("Resize mode is exclusive"));
  assert(statusSource.includes("DESKTOP_RESIZE_HINT"));
  assert(statusSource.includes('{ keys: "H/L", label: "Resize" }'));
});

test("Prompt divider is governed by the Conversation floor, not a hidden percentage cap", () => {
  assertEquals(workspaceSource.includes("46%"), false);
  assert(workspaceSource.includes("calc(100% -"));
});
