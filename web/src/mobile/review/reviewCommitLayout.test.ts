import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";

const appSource = await readFile(
  new URL("./ReviewApp.tsx", import.meta.url), "utf8",
);
const repositorySource = await readFile(
  new URL("./ReviewRepository.tsx", import.meta.url), "utf8",
);
const commitSource = await readFile(
  new URL("./ReviewCommit.tsx", import.meta.url), "utf8",
);
const codeViewerSource = await readFile(
  new URL("./CodeViewer.tsx", import.meta.url), "utf8",
);

test("repository history pages older commits instead of a 128-commit wall", () => {
  if (repositorySource.includes("Showing the newest 128 commits")) {
    throw new Error(
      "History should lazy-load instead of advertising a hard cap",
    );
  }
  if (!repositorySource.includes("HistoryCommitSkeleton")) {
    throw new Error("History needs a transcript-like loading skeleton");
  }
});

test("a commit patch has no inner back chrome and lists files in the strip", () => {
  if (commitSource.includes("Back to commit files")) {
    throw new Error("Commit patch should not add a second back control");
  }
  if (!appSource.includes("commitFileTabs(commitPaths)")) {
    throw new Error("Commit view must put involved files in the tab strip");
  }
  if (!appSource.includes('allowPin={mode === "files"}')) {
    throw new Error("Commit file tabs must not offer close or pin");
  }
});

test("repository history opens commit content on the main review surface", () => {
  assertStringIncludes(repositorySource, "onOpenCommit(commit);");
  assertStringIncludes(repositorySource, "onClose();");
  assertStringIncludes(appSource, "<ReviewCommit");
  assertStringIncludes(appSource, 'mode === "git" && commitTarget');
});

test("repository tabs use the shared segmented tabs", () => {
  // SegmentedTabs.test.ts pins the iOS selected-pill invariants.
  const start = repositorySource.indexOf("<SegmentedTabs");
  const tabs = repositorySource.slice(
    start,
    repositorySource.indexOf("/>", start),
  );
  assertStringIncludes(tabs, "data-mobile-repository-tabs");
  assertStringIncludes(tabs, 'aria-label="Repository views"');
  assertEquals(repositorySource.includes('role="tab"'), false);
});

test("repository header uses a machine chip and stable project path", () => {
  assertStringIncludes(repositorySource, "label={machineLabel}");
  assertStringIncludes(repositorySource, "data-repository-project-path");
  assertStringIncludes(appSource, "{ projectPath: currentProjectPath }");
  assertStringIncludes(appSource, "currentRegisteredWorkspace?.canonical_path");
});

test("repository footer keeps Settings and close in one capsule", () => {
  assertStringIncludes(repositorySource, 'key: "settings"');
  assertStringIncludes(repositorySource, 'key: "close"');
  assertStringIncludes(repositorySource, 'justifyContent: "flex-start"');
  if (repositorySource.includes("MobileSheetDismiss")) {
    throw new Error("Repository close belongs in the Settings capsule");
  }
});

test("commit patches are not rendered inside the repository drawer", () => {
  assertStringIncludes(commitSource, "data-review-commit-patch");
  assertStringIncludes(commitSource, 'component="main"');
  if (repositorySource.includes("fetchGitCommitDiff")) {
    throw new Error("Repository drawer must not own commit patch rendering");
  }
});

test("review tabs retain independent fail-safe scroll surfaces", () => {
  assertStringIncludes(appSource, "tabScrollPositions");
  assertStringIncludes(appSource, "outerScrollKey");
  assertStringIncludes(appSource, "editorScrollKey");
  assertStringIncludes(codeViewerSource, "restoreReviewScrollTop");
  assertStringIncludes(codeViewerSource, "scrollRestoreKey");
  assertStringIncludes(commitSource, "overviewScrollKey");
  assertStringIncludes(commitSource, 'data-mobile-overflow-layer="true"');
});

test("tab close confirmation uses the medium Cowboy corner radius", () => {
  const content = appSource.indexOf("data-review-tab-close-confirm");
  const start = appSource.lastIndexOf("<Popover", content);
  const end = appSource.indexOf("</Popover>", content);
  const confirmation = appSource.slice(start, end);

  assertEquals(start >= 0 && content > start && end > content, true);
  assertStringIncludes(confirmation, 'borderRadius: "12px"');
  assertEquals(confirmation.includes("borderRadius: 2.5"), false);
});
