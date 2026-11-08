import { expect, test } from "bun:test";
import { type MobileReviewState, mobileReviewMutators, revealedFile } from "./mobileReviewState";

const empty: MobileReviewState = { mode: "git", tabs: [], progress: {}, positions: {} };

test("a reveal records its path and line at the revision it sets", () => {
  const revealed = mobileReviewMutators.open(empty, { path: "src/lib.rs", reveal: true, line: 42 });
  expect(revealed.reveal).toEqual({ path: "src/lib.rs", line: 42, revision: 1 });
  expect(revealedFile(revealed)).toEqual({ path: "src/lib.rs", line: 42 });
  // Browsing moves `active` but not what the reveal named.
  const browsed = mobileReviewMutators.open(revealed, { path: "README.md" });
  expect(revealedFile(browsed)).toEqual({ path: "src/lib.rs", line: 42 });
});

test("a line needs a reveal, and an older writer falls back to active", () => {
  expect(() => mobileReviewMutators.open(empty, { path: "a.rs", line: 3 })).toThrow();
  expect(() => mobileReviewMutators.open(empty, { path: "a.rs", reveal: true, line: 0 })).toThrow();
  expect(revealedFile({ ...empty, active: "a.rs", reveal_revision: 2 })).toEqual({ path: "a.rs" });
  expect(revealedFile({ ...empty, active: "a.rs", reveal_revision: 2, reveal: { path: "b.rs", revision: 1 } }))
    .toEqual({ path: "a.rs" });
});
