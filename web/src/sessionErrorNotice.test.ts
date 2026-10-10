import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { shouldShowSessionErrorSnackbar } from "./sessionErrorNotice.ts";

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

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);

test("App gates the composer snackbar on the focused session", () => {
  assertEquals(appSource.includes("shouldShowSessionErrorSnackbar("), true);
});
