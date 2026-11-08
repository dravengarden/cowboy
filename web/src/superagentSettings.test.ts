import { test } from "bun:test";
import { assertEquals, assertThrows } from "../../tools/test-assert.ts";
import { parsePatternBundle, sourceProfileId } from "./superagentPreferences";

test("pattern import preserves source content and rejects malformed or oversized bundles", () => {
  const source = {
    revision: "commit",
    profile: "PROFILE.md",
    files: {
      "PROFILE.md": '+++\nid = "global"\n+++\n# Scope\nid = "wrong-body-id"\n',
    },
  };
  assertEquals(sourceProfileId(source), "global");
  assertEquals(parsePatternBundle(JSON.stringify([source])), [source]);
  for (const value of [
    null,
    {},
    [null],
    [{ profile: "missing", files: {} }],
    [
      {
        ...source,
        files: { "PROFILE.md": 42 },
      },
    ],
  ]) {
    assertThrows(() => parsePatternBundle(JSON.stringify(value)));
  }
  assertThrows(() =>
    parsePatternBundle(JSON.stringify([{ ...source, revision: "x".repeat(800_000) }])),
  );
});
