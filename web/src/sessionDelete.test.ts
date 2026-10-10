import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const storeSource = await readFile(
  new URL("./store.ts", import.meta.url), "utf8",
);

test("session delete waits for the authoritative list and has a timeout", () => {
  assert(storeSource.includes("export function deleteSession("));
  assert(storeSource.includes('type: "delete_session"'));
  assert(storeSource.includes("deletingSessionIds"));
  assert(storeSource.includes('"Delete session"'));
  assert(storeSource.includes("NETWORK_ACTION_TIMEOUT_MS"));
  assert(storeSource.includes("Could not delete this session. Try again."));
  assertEquals(storeSource.includes("sendWithAck("), true);
});

test("a deleting session row is busy, disabled, and shows delayed progress", () => {
  assert(appSource.includes("void deleteSession(pendingDelete.id)"));
  assertEquals(appSource.includes('type: "delete_session"'), false);
  assert(appSource.includes("data-session-deleting"));
  assert(appSource.includes("deletingSessionIds.has(s.id)"));
  assert(appSource.includes("<DelayedNetworkProgress size={desktopSize(18)} />"));
  assert(appSource.includes('pointerEvents: "none"'));
  assert(appSource.includes("aria-busy={deleting || undefined}"));
});

test("a pending environment stop acknowledges deletion and keeps the row busy", () => {
  assert(storeSource.includes("session.id === sessionId && !session.closing"));
  assert(storeSource.includes("reportRetainedClosures(msg.sessions)"));
  assert(appSource.includes("deletingSessionIds.has(s.id) || s.closing === true"));
});
