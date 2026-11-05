import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { errorNoticeFrom, shouldShowSessionErrorSnackbar } from "./sessionErrorNotice.ts";
import { readAppSource } from "./appSource.test.ts";

const notice = {
  seq: 4,
  sessionId: "sess-other",
  message:
    "worker sess-other exited before readiness: agent did not complete ACP session/resume within 240s",
};

test("focused session still sees its own daemon error", () => {
  assertEquals(
    shouldShowSessionErrorSnackbar(notice, "sess-other", 0),
    true,
  );
});

test("another session's crash does not cover the focused composer", () => {
  assertEquals(
    shouldShowSessionErrorSnackbar(notice, "sess-focused", 0),
    false,
  );
});

test("global notices without a session id still show", () => {
  assertEquals(
    shouldShowSessionErrorSnackbar(
      { seq: 2, message: "bad inbound command" },
      "sess-focused",
      0,
    ),
    true,
  );
});

test("already-dismissed seqs stay closed", () => {
  assertEquals(shouldShowSessionErrorSnackbar(notice, "sess-other", 4), false);
  assertEquals(
    shouldShowSessionErrorSnackbar(undefined, "sess-other", 0),
    false,
  );
});

const appSource = await readAppSource();

test("App gates the composer snackbar on the focused session", () => {
  assertEquals(appSource.includes("shouldShowSessionErrorSnackbar("), true);
});

test("a global error frame shows whether session_id is absent or null", () => {
  for (const frame of [
    { message: "bad inbound command" },
    { session_id: null, message: "bad inbound command" },
  ]) {
    const global = errorNoticeFrom(3, frame);
    assertEquals(global, { seq: 3, message: "bad inbound command" });
    assertEquals(shouldShowSessionErrorSnackbar(global, "sess-focused", 0), true);
  }
  assertEquals(
    errorNoticeFrom(4, { session_id: "sess-1", message: "command failed" }),
    { seq: 4, sessionId: "sess-1", message: "command failed" },
  );
});
