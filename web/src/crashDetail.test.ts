import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { RenderItem } from "./derive.ts";
import {
  crashDetailsMatch,
  hideLiveCrashDuplicate,
  prettifyCrashDetail,
} from "./crashDetail.ts";

const jsonDump =
  'Internal error: { "message": "You\'ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 11:45 AM.", "codexErrorInfo": "usageLimitExceeded" }';

test("usage-limit JSON dumps keep the human sentence", () => {
  assertEquals(
    prettifyCrashDetail(jsonDump),
    "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 11:45 AM.",
  );
  assertEquals(
    crashDetailsMatch(jsonDump, prettifyCrashDetail(jsonDump)),
    true,
  );
});

test("plain crash details stay as written", () => {
  assertEquals(
    prettifyCrashDetail("runtime: broken pipe"),
    "runtime: broken pipe",
  );
});

test("ACP restore timeouts become a reopen/handoff sentence", () => {
  assertEquals(
    prettifyCrashDetail(
      "agent did not complete ACP session/resume within 240s",
    ),
    "Reopening this conversation timed out. Reload to retry, or continue in a new session.",
  );
  assertEquals(
    prettifyCrashDetail(
      "worker sess-1 exited before readiness: agent did not complete ACP session/load within 60s",
    ),
    "Reopening this conversation timed out. Reload to retry, or continue in a new session.",
  );
  assertEquals(
    crashDetailsMatch(
      "agent did not complete ACP session/resume within 240s",
      "worker sess-1 exited before readiness: agent did not complete ACP session/resume within 240s",
    ),
    true,
  );
});

test("a live crash bar hides the matching trailing lifecycle row", () => {
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

const overlaySource = await readFile(
  new URL("./TurnStatusOverlay.tsx", import.meta.url), "utf8",
);
const transcriptSource = await readFile(
  new URL("./Transcript.tsx", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const storeSource = await readFile(
  new URL("./store.ts", import.meta.url), "utf8",
);
const protocolSource = await readFile(
  new URL("./protocol.ts", import.meta.url), "utf8",
);

test("composer overlay does not retry a crashed turn", () => {
  assertEquals(overlaySource.includes("retryTurn"), false);
  assertEquals(overlaySource.includes("Agent error"), false);
  assertEquals(transcriptSource.includes("hideLiveCrashDuplicate"), true);
  assertEquals(transcriptSource.includes("prettifyCrashDetail"), true);
});

test("interrupted turns expose status without synthetic resume controls", () => {
  assertEquals(overlaySource.includes("resumeTurn"), false);
  assertEquals(overlaySource.includes('label: "Resume"'), true);
  assertEquals(storeSource.includes('type: "resume_turn"'), false);
  assertEquals(protocolSource.includes('type: "resume_turn"'), true);
  assertEquals(
    protocolSource.includes("Deprecated rollout tombstones"),
    true,
  );
  assertEquals(appSource.includes("Auto-resume"), false);
  assertEquals(appSource.includes("AutoResumeSettings"), false);
});
