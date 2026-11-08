import { test } from "bun:test";
import { assertEquals, assertThrows } from "../../../../tools/test-assert.ts";
import type { Choice } from "../../mobile/review/remoteReviewSources.ts";
import { choiceForInput } from "./ReviewPullPicker.tsx";

const choice = (discovery: boolean): Choice => ({
  extension: {
    identity: { pluginId: "github", pluginVersion: "0.2.0", generationDigest: "sha256:x" },
    label: "GitHub", description: "", available: true,
    views: [{ id: "pulls", label: "Pulls", filters: [], review: "pull_request", discovery }],
  },
  view: "pulls",
  remote: { name: "origin", host: "github.com", owner: "dravengarden", repository: "cowboy" },
  key: "k",
});

test("a number opens in the selected repository", () => {
  const opened = choiceForInput(choice(true), " 42 ");
  assertEquals(opened.number, "42");
  assertEquals(opened.choice.remote.repository, "cowboy");
});

test("a URL names its own repository when the view discovers across repositories", () => {
  const opened = choiceForInput(choice(true), "https://github.com/other/tool/pull/7");
  assertEquals([opened.choice.remote.owner, opened.choice.remote.repository, opened.number], ["other", "tool", "7"]);
});

test("without discovery a URL must name the selected repository", () => {
  assertThrows(() => choiceForInput(choice(false), "https://github.com/other/tool/pull/7"));
  assertEquals(choiceForInput(choice(false), "https://github.com/dravengarden/cowboy/pull/9").number, "9");
});
