import { assertEquals } from "jsr:@std/assert";
import type { CodeChange } from "./codeApi.ts";
import {
  groupGitChanges,
  limitGitSections,
  reviewQueue,
} from "./gitReviewModel.ts";

function change(
  path: string,
  values: Partial<CodeChange> = {},
): CodeChange {
  return {
    path,
    status: "modified",
    staged: false,
    unstaged: true,
    ...values,
  };
}

Deno.test("git review groups conflicts and duplicates partial changes by intent", () => {
  const sections = groupGitChanges([
    change("conflict.ts", {
      status: "conflicted",
      staged: true,
      unstaged: true,
    }),
    change("partial.ts", { staged: true, unstaged: true }),
    change("staged.ts", { staged: true, unstaged: false }),
    change("new.ts", { status: "untracked" }),
  ]);

  assertEquals(sections.map((section) => section.kind), [
    "conflicts",
    "unstaged",
    "staged",
  ]);
  assertEquals(
    reviewQueue(sections).map(({ change, scope }) => `${scope}:${change.path}`),
    [
      "combined:conflict.ts",
      "unstaged:partial.ts",
      "unstaged:new.ts",
      "staged:partial.ts",
      "staged:staged.ts",
    ],
  );
});

Deno.test("git review window preserves section order and intent", () => {
  const sections = groupGitChanges([
    change("conflict.ts", { status: "conflicted" }),
    change("first.ts"),
    change("partial.ts", { staged: true, unstaged: true }),
    change("staged.ts", { staged: true, unstaged: false }),
  ]);

  const visible = limitGitSections(sections, 3);
  assertEquals(visible.map((section) => section.kind), [
    "conflicts",
    "unstaged",
  ]);
  assertEquals(
    reviewQueue(visible).map(({ change, scope }) => `${scope}:${change.path}`),
    [
      "combined:conflict.ts",
      "unstaged:first.ts",
      "unstaged:partial.ts",
    ],
  );
  assertEquals(reviewQueue(sections).length, 5);
});
