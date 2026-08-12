import { assertEquals } from "jsr:@std/assert";
import {
  type ExploreSessionState,
  exploreStateAfterContextClear,
} from "./contextClear.ts";

Deno.test("context clear drops deleted page identities but preserves Page mode", () => {
  const previous: ExploreSessionState = {
    projection: "explore",
    pageId: "old-question",
    pageStartId: "old-question",
    pageLoadingId: "old-question",
    transitionAnchorKey: "old-answer",
    followTailRequested: true,
    pageParents: { "old-follow-up": "old-question" },
    pendingFollowUp: {
      targetPageId: "old-question",
      knownPageIds: ["old-question"],
    },
  };

  assertEquals(exploreStateAfterContextClear(previous), {
    projection: "explore",
    pageId: null,
    pageStartId: null,
    pageLoadingId: null,
    transitionAnchorKey: null,
    followTailRequested: false,
    pageParents: {},
    pendingFollowUp: null,
  });
});

Deno.test("context clear keeps History mode selected", () => {
  const previous: ExploreSessionState = {
    projection: "history",
    pageId: "old-question",
    pageStartId: null,
    pageLoadingId: null,
    transitionAnchorKey: null,
    followTailRequested: false,
    pageParents: {},
    pendingFollowUp: null,
  };

  assertEquals(
    exploreStateAfterContextClear(previous).projection,
    "history",
  );
});
