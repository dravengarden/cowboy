import type { CodeChange, CodeDiffScope } from "./codeApi.ts";

export type GitReviewSectionKind = "conflicts" | "unstaged" | "staged";

export interface GitReviewEntry {
  change: CodeChange;
  scope: CodeDiffScope;
}

export interface GitReviewSection {
  kind: GitReviewSectionKind;
  label: string;
  entries: GitReviewEntry[];
}

export function groupGitChanges(changes: CodeChange[]): GitReviewSection[] {
  const conflicts: GitReviewEntry[] = [];
  const unstaged: GitReviewEntry[] = [];
  const staged: GitReviewEntry[] = [];

  for (const change of changes) {
    if (change.status === "conflicted") {
      conflicts.push({ change, scope: "combined" });
      continue;
    }
    if (change.unstaged) {
      unstaged.push({ change, scope: "unstaged" });
    }
    if (change.staged) {
      staged.push({ change, scope: "staged" });
    }
  }

  const sections: GitReviewSection[] = [
    { kind: "conflicts", label: "Conflicts", entries: conflicts },
    { kind: "unstaged", label: "Unstaged", entries: unstaged },
    { kind: "staged", label: "Staged", entries: staged },
  ];
  return sections.filter((section) => section.entries.length > 0);
}

export function reviewQueue(
  sections: GitReviewSection[],
): GitReviewEntry[] {
  return sections.flatMap((section) => section.entries);
}
