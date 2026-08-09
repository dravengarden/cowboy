import { assert, assertEquals } from "jsr:@std/assert";

const topBarSource = await Deno.readTextFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);

Deno.test("desktop Clear follows Compact with a matching labelled control", () => {
  const compact = topBarSource.indexOf('data-desktop-item="topbar-compact"');
  const clear = topBarSource.indexOf('data-desktop-item="topbar-clear"');
  const stop = topBarSource.indexOf("<AutoScrollAndStop");

  assert(compact >= 0);
  assert(clear > compact);
  assert(stop > clear);
  assert(topBarSource.includes("data-desktop-clear"));
  assert(topBarSource.includes("<CleaningServices"));
  assert(topBarSource.includes('keyLabel="X"'));
  assert(topBarSource.includes('shortcut: "X"'));
  assert(topBarSource.includes("await resetSession(sessionId)"));
});

Deno.test("desktop composer no longer owns the icon-only Clear action", () => {
  assertEquals(
    composerSource.includes(
      'size="small"\n                    aria-label="clear conversation"',
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      'aria-label="clear conversation from session settings"',
    ),
    true,
  );
});
