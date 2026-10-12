import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { permissionResponseNotice } from "./permissionResponse";

test("an unconfirmed permission answer always says what happens next", () => {
  assertEquals(permissionResponseNotice(null), null);
  // Sent and waiting is shown by the spinner, not by a message.
  assertEquals(permissionResponseNotice("sent"), null);
  assertEquals(
    permissionResponseNotice("unsent")?.startsWith("Not connected"),
    true,
  );
  assertEquals(
    permissionResponseNotice("stalled")?.includes("answer again"),
    true,
  );
});
