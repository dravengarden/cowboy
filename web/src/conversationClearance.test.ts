import { assertEquals } from "jsr:@std/assert";
import {
  beginConversationClear,
  conversationClearEnterMs,
  conversationClearExitMs,
  subscribeConversationClear,
} from "./conversationClearance.ts";

const storeSource = await Deno.readTextFile(
  new URL("./store.ts", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Clear starts a page transition before the transcript is replaced", () => {
  const seen: string[] = [];
  const stop = subscribeConversationClear((sessionId) => {
    seen.push(sessionId);
  });
  beginConversationClear("sess-1");
  stop();
  beginConversationClear("sess-2");
  assertEquals(seen, ["sess-1"]);
  assertEquals(conversationClearExitMs, 320);
  assertEquals(conversationClearEnterMs, 280);
  assertEquals(storeSource.includes("beginConversationClear(sessionId);"), true);
  assertEquals(
    transcriptSource.includes("data-conversation-clear-phase={clearPhase === \"idle\""),
    true,
  );
  assertEquals(transcriptSource.includes("const liveItems = useMemo("), true);
  assertEquals(transcriptSource.includes("const items = exitItems ?? liveItems"), true);
});
