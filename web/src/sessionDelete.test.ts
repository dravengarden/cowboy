import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const storeSource = await Deno.readTextFile(
  new URL("./store.ts", import.meta.url),
);

Deno.test("session delete waits for the authoritative list and has a timeout", () => {
  assert(storeSource.includes("export function deleteSession("));
  assert(storeSource.includes('type: "delete_session"'));
  assert(storeSource.includes("deletingSessionIds"));
  assert(storeSource.includes('"Delete session"'));
  assert(storeSource.includes("NETWORK_ACTION_TIMEOUT_MS"));
  assert(storeSource.includes("Could not delete this session. Try again."));
  assertEquals(storeSource.includes("sendWithAck("), true);
});

Deno.test("a deleting session row is busy, disabled, and shows delayed progress", () => {
  assert(appSource.includes("void deleteSession(pendingDelete.id)"));
  assertEquals(appSource.includes('type: "delete_session"'), false);
  assert(appSource.includes("data-session-deleting"));
  assert(appSource.includes("deletingSessionIds.has(s.id)"));
  assert(appSource.includes("<DelayedNetworkProgress size={18} />"));
  assert(appSource.includes('pointerEvents: "none"'));
  assert(appSource.includes("aria-busy={deleting || undefined}"));
});
