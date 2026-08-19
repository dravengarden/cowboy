import { assertEquals } from "jsr:@std/assert";
import { pendingRowMatchesArrival } from "./pendingPanelState.ts";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const storeSource = await Deno.readTextFile(
  new URL("./store.ts", import.meta.url),
);

Deno.test("a just-staged optimistic row still matches after the server id arrives", () => {
  assertEquals(
    pendingRowMatchesArrival({ id: "opt-abc", cmid: "abc" }, {
      kind: "draft",
      id: "opt-abc",
      cmid: "abc",
    }),
    true,
  );
  assertEquals(
    pendingRowMatchesArrival({ id: "draft-1", cmid: "abc" }, {
      kind: "draft",
      id: "opt-abc",
      cmid: "abc",
    }),
    true,
  );
  assertEquals(
    pendingRowMatchesArrival({ id: "other", cmid: "zzz" }, {
      kind: "queued",
      id: "opt-abc",
      cmid: "abc",
    }),
    false,
  );
});

Deno.test("staging a draft or queue item expands that panel and flashes the row", () => {
  assertEquals(storeSource.includes("revealPendingArrival({"), true);
  assertEquals(composerSource.includes("subscribePendingArrival"), true);
  assertEquals(composerSource.includes("data-pending-row-flash"), true);
  assertEquals(composerSource.includes("scrollPendingRowIntoView"), true);
});

Deno.test("queue and draft bulk actions live in the header kebab", () => {
  assertEquals(composerSource.includes('aria-label={kind === "draft" ? "Draft actions" : "Queue actions"}'), true);
  assertEquals(composerSource.includes("<MoreVert fontSize=\"small\" />"), true);
  assertEquals(composerSource.includes("setBulkConfirm(\"send-all\")"), true);
  assertEquals(composerSource.includes("setBulkConfirm(\"clear-all\")"), true);
  assertEquals(composerSource.includes('label="Clear All"'), false);
  assertEquals(composerSource.includes('label="Send all"'), false);
});
