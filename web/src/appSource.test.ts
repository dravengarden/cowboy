import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "../../tools/test-assert.ts";

// Source-text assertions about the large shell modules predate their split
// (docs/structural-refactor-plan-2026-11.md, Phase 3). They read the
// combined source of a module and everything split out of it, so code moved
// unchanged keeps its assertions; behavioural tests replace them over time
// (slice 3.1). Add a file here when it is split out of one of these modules.
export const SPLIT_SOURCES = {
  "App.tsx": [
    "./App.tsx",
    "./DeleteSessionShell.tsx",
    "./SettingsShell.tsx",
    "./MobileSettingsNavigation.tsx",
    "./SessionList.tsx",
    "./sessionFocusLifecycle.ts",
    "./desktopSplitters.tsx",
    "./mobileDrawerSurface.ts",
  ],
  "Transcript.tsx": [
    "./Transcript.tsx",
    "./ToolDetailsBrowser.tsx",
    "./TranscriptItems.tsx",
    "./TranscriptPlaceholders.tsx",
  ],
  "Composer.tsx": [
    "./Composer.tsx",
    "./PendingPanel.tsx",
    "./ComposerSheet.tsx",
    "./composerImageSelection.tsx",
    "./DraftMoveSurfaces.tsx",
  ],
  "mobile/review/ReviewApp.tsx": [
    "./mobile/review/ReviewApp.tsx",
    "./mobile/review/ReviewModeSwitcher.tsx",
    "./mobile/review/ReviewMoreActions.tsx",
    "./mobile/review/ReviewContextButton.tsx",
    "./mobile/review/ReviewProjectCheckoutRow.tsx",
    "./mobile/review/DocumentView.tsx",
    "./mobile/review/reviewMarkdownPreview.ts",
  ],
} as const;

export async function readSplitSource(
  module: keyof typeof SPLIT_SOURCES,
): Promise<string> {
  const parts = await Promise.all(
    SPLIT_SOURCES[module].map((path) =>
      readFile(new URL(path, import.meta.url), "utf8")
    ),
  );
  // A split-out module repeats imports the original had once; keep only the
  // original module's imports so counts over the combined source still hold.
  return parts.map((part, index) =>
    index === 0 ? part : part.replace(/^import\b[\s\S]*?;\n/gm, "")
  ).join("\n");
}

/** One top-level declaration of a combined source: from `function name(` to
 *  the next top-level declaration, wherever the module split placed it. */
export function declarationSource(source: string, name: string): string {
  const start = source.indexOf(`function ${name}(`);
  if (start === -1) return "";
  const rest = source.slice(start + 1);
  const next = rest.search(
    /\n(?:export )?(?:async )?(?:function|const|let|class|type|interface) /,
  );
  return source.slice(start, next === -1 ? source.length : start + 1 + next);
}

export function readAppSource(): Promise<string> {
  return readSplitSource("App.tsx");
}

export function readTranscriptSource(): Promise<string> {
  return readSplitSource("Transcript.tsx");
}

export function readComposerSource(): Promise<string> {
  return readSplitSource("Composer.tsx");
}

export function readReviewAppSource(): Promise<string> {
  return readSplitSource("mobile/review/ReviewApp.tsx");
}

test("each combined source includes every module split out of it", async () => {
  const app = await readAppSource();
  assert(app.includes("export function App("));
  assert(app.includes("export function SettingsShell("));
  assert(app.includes("export function SessionList("));
  const dialog = declarationSource(app, "CreateDialog");
  assert(dialog.length > 0 && !dialog.slice(1).includes("\nexport function "));
  const transcript = await readTranscriptSource();
  assert(transcript.includes("export function Transcript("));
  assert(transcript.includes("export function ToolDetailsBrowser("));
  const composer = await readComposerSource();
  assert(composer.includes("export function PendingPanel("));
  const review = await readReviewAppSource();
  assert(review.includes("export function DocumentView("));
});
