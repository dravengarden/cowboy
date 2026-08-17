import { assertEquals } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);
const fullscreenSource = await Deno.readTextFile(
  new URL("../FullscreenComposer.tsx", import.meta.url),
);

Deno.test("Draft and Queue edits defer end-focus to the interactive Desktop editor", () => {
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
