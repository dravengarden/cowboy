import { expect, test } from "bun:test";
import { sessionLinks } from "./sessionLinks";

const live = [{ id: "a", title: "Alpha" }];
const trash = [{ id: "b", title: "Beta" }];

test("a continuation leads and every link keeps its point", () => {
  expect(sessionLinks(
    { continued_from: "a" },
    [
      { id: "b", available: true, title: "Beta", before: 4 },
      { id: "a", available: true, title: "Alpha", before: 9 },
    ],
    live,
    trash,
  )).toEqual([
    { id: "a", kind: "continued", state: "open", title: "Alpha", before: 9 },
    { id: "b", kind: "linked", state: "trash", title: "Beta", before: 4 },
  ]);
});

test("a deleted source stays named and is never lost", () => {
  expect(sessionLinks(
    { continued_from: "gone" },
    [{ id: "gone", available: false, title: "Old work" }, { id: "x", available: false, title: "" }],
    live,
    trash,
  )).toEqual([
    { id: "gone", kind: "continued", state: "deleted", title: "Old work" },
    { id: "x", kind: "linked", state: "deleted", title: "Deleted session" },
  ]);
});

test("a mark without a stored link and a fork still show", () => {
  expect(sessionLinks({ forked_from: "a", continued_from: "c" }, undefined, live, [])).toEqual([
    { id: "a", kind: "forked", state: "open", title: "Alpha" },
    { id: "c", kind: "continued", state: "deleted", title: "Deleted session" },
  ]);
});
