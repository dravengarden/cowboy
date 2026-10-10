import { test } from "bun:test";
import { assertEquals, assertThrows } from "@std/assert";
import {
  decodeRemoteReview,
  pullNumber,
  sameReview,
} from "./remoteReviewModel.ts";

const remote = { host: "github.com", owner: "owner", repository: "repo" };
const fixture = () => ({
  repositoryId: "123",
  number: "12",
  title: "Change",
  url: "https://github.com/owner/repo/pull/12",
  state: "open",
  head: "a".repeat(40),
  base: "b".repeat(40),
  revision: "c".repeat(64),
  totalFiles: 21,
  nextPage: 2,
  limited: false,
  files: [{
    path: "file[1].rs",
    oldPath: null,
    status: "modified",
    additions: 1,
    deletions: 1,
    patch: "@@ -1 +1 @@\n-old\n+new",
    limited: false,
  }],
});

test("PR input binds a number to the selected repository without accepting arbitrary URLs", () => {
  assertEquals(pullNumber(" 12 ", remote), "12");
  assertEquals(
    pullNumber("https://github.com/owner/repo/pull/12#discussion", remote),
    "12",
  );
  for (
    const input of [
      "0",
      "-1",
      "12/files",
      "https://evil.test/owner/repo/pull/12",
      "https://github.com/other/repo/pull/12",
      "https://token@github.com/owner/repo/pull/12",
      "http://github.com/owner/repo/pull/12",
      "https://github.com:444/owner/repo/pull/12",
    ]
  ) {
    assertThrows(() => pullNumber(input, remote));
  }
});

test("remote response refuses executable links, unbounded patches and invalid revisions", () => {
  assertEquals(decodeRemoteReview(fixture()), fixture());
  for (
    const change of [
      { url: "javascript:alert(1)" },
      { head: "main" },
      { nextPage: 151 },
      { totalFiles: -1 },
      { limited: "false" },
      { files: Array(21).fill(fixture().files[0]) },
      { files: [{ ...fixture().files[0], patch: "x".repeat(256 * 1024 + 1) }] },
    ]
  ) {
    assertThrows(() => decodeRemoteReview({ ...fixture(), ...change }));
  }
});

test("pages cannot cross PR, repository, base/head revision or file-count boundaries", () => {
  const first = decodeRemoteReview(fixture());
  for (
    const change of [{ repositoryId: "124" }, { number: "13" }, {
      revision: "d".repeat(64),
    }, { totalFiles: 22 }]
  ) {
    assertEquals(sameReview(first, { ...first, ...change }), false);
  }
  assertEquals(sameReview(first, { ...first, files: [] }), true);
});
