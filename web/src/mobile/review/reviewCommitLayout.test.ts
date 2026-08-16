import { assertStringIncludes } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./ReviewApp.tsx", import.meta.url),
);
const repositorySource = await Deno.readTextFile(
  new URL("./ReviewRepository.tsx", import.meta.url),
);
const commitSource = await Deno.readTextFile(
  new URL("./ReviewCommit.tsx", import.meta.url),
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

Deno.test("repository history opens commit content on the main review surface", () => {
  assertStringIncludes(repositorySource, "onOpenCommit(commit);");
  assertStringIncludes(repositorySource, "onClose();");
  assertStringIncludes(appSource, "<ReviewCommit");
  assertStringIncludes(appSource, 'mode === "git" && commitTarget');
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
