import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
);
const workspaceSource = await Deno.readTextFile(
  new URL("./DesktopWorkspace.tsx", import.meta.url),
);
const commandsSource = await Deno.readTextFile(
  new URL("./commands/DesktopCommandProvider.tsx", import.meta.url),
);
const statusSource = await Deno.readTextFile(
  new URL("./DesktopStatusLine.tsx", import.meta.url),
);

Deno.test("every Desktop vertical boundary exposes the shared splitter contract", () => {
  assert(appSource.includes('data-desktop-splitter="sessions-prompt"'));
  assert(workspaceSource.includes('data-desktop-splitter="prompt-conversation"'));
  assert(workspaceSource.includes('data-desktop-splitter="questions-page"'));
  assertEquals(
    (appSource + workspaceSource).match(/<DesktopSplitterHint \/>/gu)?.length,
    3,
  );
});

Deno.test("Ctrl-W R owns an exclusive H/L splitter mode", () => {
  assert(commandsSource.includes('["h", "j", "k", "l", "r", "w"]'));
  assert(commandsSource.includes('key.toLowerCase() === "r"'));
  assert(commandsSource.includes('lower === "h" || lower === "l"'));
  assert(commandsSource.includes("DESKTOP_SPLITTER_LARGE_STEP"));
  assert(commandsSource.includes("adjacentDesktopSplitter("));
  assert(commandsSource.includes("Resize mode is exclusive"));
  assert(statusSource.includes('{ keys: "Ctrl+W R", label: "Resize" }'));
  assert(statusSource.includes('{ keys: "H/L", label: "Resize" }'));
});

Deno.test("Reading Questions width is adjustable and persisted", () => {
  assert(workspaceSource.includes("readingQuestionsWidthStore.get"));
  assert(workspaceSource.includes("startQuestionsResize"));
  assert(workspaceSource.includes("readingQuestionsWidthStore.set(next)"));
  assert(workspaceSource.includes("width={questionsWidth}"));
});

Deno.test("Prompt divider is governed by the Conversation floor, not a hidden percentage cap", () => {
  assertEquals(workspaceSource.includes("46%"), false);
  assert(workspaceSource.includes("calc(100% -"));
});
