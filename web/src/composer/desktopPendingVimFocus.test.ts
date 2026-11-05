import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { readComposerSource } from "../appSource.test.ts";

const composerSource = await readComposerSource();
const fullscreenSource = await readFile(
  new URL("../FullscreenComposer.tsx", import.meta.url), "utf8",
);

test("Draft and Queue edits defer end-focus to the interactive Desktop editor", () => {
  const pendingRow = composerSource.slice(
    composerSource.indexOf("function PendingRow("),
    composerSource.indexOf("function PendingRowPeek("),
  );

  assertEquals(pendingRow.includes('kind: "queued" | "draft"'), true);
  assertEquals(
    pendingRow.match(/focusEndOnMount=\{desktop\}/g)?.length,
    2,
  );
  assertEquals(
    fullscreenSource.includes("focusEndOnMount={focusEndOnMount}"),
    true,
  );
});
