import { assertEquals } from "jsr:@std/assert";
import type { RenderItem } from "./derive.ts";
import {
  crashDetailsMatch,
  hideLiveCrashDuplicate,
  prettifyCrashDetail,
} from "./crashDetail.ts";

const jsonDump =
  'Internal error: { "message": "You\'ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 11:45 AM.", "codexErrorInfo": "usageLimitExceeded" }';

Deno.test("usage-limit JSON dumps keep the human sentence", () => {
  assertEquals(
    prettifyCrashDetail(jsonDump),
    "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 11:45 AM.",
  );
  assertEquals(
    crashDetailsMatch(jsonDump, prettifyCrashDetail(jsonDump)),
    true,
  );
});

Deno.test("plain crash details stay as written", () => {
  assertEquals(prettifyCrashDetail("runtime: broken pipe"), "runtime: broken pipe");
});

Deno.test("a live crash bar hides the matching trailing lifecycle row", () => {
  const items: RenderItem[] = [
    {
      kind: "message",
      role: "assistant",
      chunks: [{ type: "text", text: "You've hit your usage limit." }],
      key: "1",
    },
    {
      kind: "lifecycle",
      status: "crashed",
      detail: jsonDump,
      key: "2",
    },
  ];
  const hidden = hideLiveCrashDuplicate(items, "crashed", jsonDump);
  assertEquals(hidden.length, 1);
  assertEquals(hidden[0]?.kind, "message");
  assertEquals(hideLiveCrashDuplicate(items, "running", jsonDump), items);
});

const overlaySource = await Deno.readTextFile(
  new URL("./TurnStatusOverlay.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("composer overlay does not retry a crashed turn", () => {
  assertEquals(overlaySource.includes("retryTurn"), false);
  assertEquals(overlaySource.includes("Agent error"), false);
  assertEquals(transcriptSource.includes("hideLiveCrashDuplicate"), true);
  assertEquals(transcriptSource.includes("prettifyCrashDetail"), true);
});
