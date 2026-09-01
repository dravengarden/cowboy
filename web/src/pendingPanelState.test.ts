import { assertEquals } from "jsr:@std/assert";
import {
  pendingRowMatchesArrival,
  pendingRowRevealDelta,
} from "./pendingPanelState.ts";

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

Deno.test("an arrived row is anchored instead of accepting partial visibility", () => {
  assertEquals(pendingRowRevealDelta(520, 200), 312);
  assertEquals(pendingRowRevealDelta(208, 200), 0);
  assertEquals(composerSource.includes("onEntered={revealArrivalRow}"), true);
  assertEquals(composerSource.includes('addEventListener("load", onImageLoad, true)'), true);
});

Deno.test("the first pending row reserves room for its arrival focus ring", () => {
  const scrollportStart = composerSource.indexOf(
    "data-mobile-pending-scrollport=",
  );
  const rowsStart = composerSource.indexOf(
    "{sortable.order.map((id, index) => {",
    scrollportStart,
  );
  const scrollport = composerSource.slice(scrollportStart, rowsStart);

  assertEquals(scrollportStart >= 0 && rowsStart > scrollportStart, true);
  assertEquals(scrollport.includes("px: mobileFloatingEdit ? 0 : 0.5"), true);
  assertEquals(scrollport.includes("pt: mobileFloatingEdit ? 0 : 0.5"), true);
  assertEquals(scrollport.includes("pb: mobileFloatingEdit ? 0 : 0.5"), true);
});

Deno.test("queue and draft bulk actions live in the header kebab", () => {
  assertEquals(composerSource.includes('aria-label={kind === "draft" ? "Draft actions" : "Queue actions"}'), true);
  assertEquals(composerSource.includes("<MoreVert fontSize=\"small\" />"), true);
  assertEquals(composerSource.includes("setBulkConfirm(\"send-all\")"), true);
  assertEquals(composerSource.includes("setBulkConfirm(\"clear-all\")"), true);
  assertEquals(composerSource.includes('label="Clear All"'), false);
  assertEquals(composerSource.includes('label="Send all"'), false);
});
