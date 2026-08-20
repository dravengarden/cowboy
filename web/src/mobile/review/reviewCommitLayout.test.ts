import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);
const repositorySource = await Deno.readTextFile(
  new URL("./ReviewRepository.tsx", import.meta.url),
);
const commitSource = await Deno.readTextFile(
  new URL("./ReviewCommit.tsx", import.meta.url),
);
const codeViewerSource = await Deno.readTextFile(
  new URL("./CodeViewer.tsx", import.meta.url),
);

Deno.test("repository history pages older commits instead of a 128-commit wall", () => {
  if (repositorySource.includes("Showing the newest 128 commits")) {
    throw new Error("History should lazy-load instead of advertising a hard cap");
  }
  if (!repositorySource.includes("HistoryCommitSkeleton")) {
    throw new Error("History needs a transcript-like loading skeleton");
  }
  if (!repositorySource.includes("fetchGitRepository(sessionId, undefined, after)")) {
    throw new Error("History pages must request the after cursor");
  }
});

Deno.test("a commit patch has no inner back chrome and lists files in the strip", () => {
  if (commitSource.includes("Back to commit files")) {
    throw new Error("Commit patch should not add a second back control");
  }
  if (!appSource.includes("commitFileTabs(commitPaths)")) {
    throw new Error("Commit view must put involved files in the tab strip");
  }
  if (!appSource.includes("allowPin={mode === \"files\"}")) {
    throw new Error("Commit file tabs must not offer close or pin");
  }
});

Deno.test("repository history opens commit content on the main review surface", () => {
  assertStringIncludes(repositorySource, "onOpenCommit(commit);");
  assertStringIncludes(repositorySource, "onClose();");
  assertStringIncludes(appSource, "<ReviewCommit");
  assertStringIncludes(appSource, 'mode === "git" && commitTarget');
});

Deno.test("repository tabs retain authoritative selected paint after an iOS touch", () => {
  const start = repositorySource.indexOf("data-mobile-repository-tabs");
  const end = repositorySource.indexOf("</Stack>", start);
  const tabs = repositorySource.slice(start, end);
  assertStringIncludes(tabs, "disableRipple");
  assertStringIncludes(tabs, 'event.currentTarget.dataset.touchActivated = "true"');
  assertStringIncludes(tabs, "event.currentTarget.blur()");
  assertStringIncludes(tabs, "&[aria-selected='true']");
  assertStringIncludes(
    tabs,
    "[data-touch-activated='true'][aria-selected='true']:hover",
  );
  assertStringIncludes(tabs, 'bgcolor: "action.selected"');
  assertEquals(tabs.includes('"@media (hover: none), (pointer: coarse)"'), false);
});

Deno.test("repository header uses a machine chip and stable project path", () => {
  assertStringIncludes(repositorySource, "label={machineLabel}");
  assertStringIncludes(repositorySource, "data-repository-project-path");
  assertStringIncludes(appSource, "{ projectPath: currentProjectPath }");
  assertStringIncludes(appSource, "currentRegisteredWorkspace?.canonical_path");
});

Deno.test("repository footer keeps Settings and close in one capsule", () => {
  assertStringIncludes(repositorySource, 'key: "settings"');
  assertStringIncludes(repositorySource, 'key: "close"');
  assertStringIncludes(repositorySource, 'justifyContent: "flex-start"');
  if (repositorySource.includes("MobileSheetDismiss")) {
    throw new Error("Repository close belongs in the Settings capsule");
  }
});

Deno.test("commit patches are not rendered inside the repository drawer", () => {
  assertStringIncludes(commitSource, "data-review-commit-patch");
  assertStringIncludes(commitSource, 'component="main"');
  if (repositorySource.includes("fetchGitCommitDiff")) {
    throw new Error("Repository drawer must not own commit patch rendering");
  }
});

Deno.test("review tabs retain independent fail-safe scroll surfaces", () => {
  assertStringIncludes(appSource, "tabScrollPositions");
  assertStringIncludes(appSource, "outerScrollKey");
  assertStringIncludes(appSource, "editorScrollKey");
  assertStringIncludes(codeViewerSource, "restoreReviewScrollTop");
  assertStringIncludes(codeViewerSource, "scrollRestoreKey");
  assertStringIncludes(commitSource, "overviewScrollKey");
  assertStringIncludes(commitSource, 'data-mobile-overflow-layer="true"');
});

Deno.test("tab close confirmation uses the medium Cowboy corner radius", () => {
  const content = appSource.indexOf("data-review-tab-close-confirm");
  const start = appSource.lastIndexOf("<Popover", content);
  const end = appSource.indexOf("</Popover>", content);
  const confirmation = appSource.slice(start, end);

  assertEquals(start >= 0 && content > start && end > content, true);
  assertStringIncludes(confirmation, 'borderRadius: "12px"');
  assertEquals(confirmation.includes("borderRadius: 2.5"), false);
});
