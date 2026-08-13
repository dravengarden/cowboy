import { assert, assertEquals } from "jsr:@std/assert";
import { desktopImeKeyIsReserved } from "./imeShortcutPolicy.ts";

const appSource = await Deno.readTextFile(
  new URL("../../App.tsx", import.meta.url),
);
const hostSource = await Deno.readTextFile(
  new URL("./DesktopCommandHost.tsx", import.meta.url),
);
const providerSource = await Deno.readTextFile(
  new URL("./DesktopCommandProvider.tsx", import.meta.url),
);
const topBarSource = await Deno.readTextFile(
  new URL("../DesktopTopBarControls.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("../../Transcript.tsx", import.meta.url),
);
const exploreSource = await Deno.readTextFile(
  new URL("../../explore/ExploreSurface.tsx", import.meta.url),
);

function key(
  overrides: Partial<KeyboardEvent> = {},
): Pick<KeyboardEvent, "isComposing" | "key" | "keyCode"> {
  return {
    isComposing: false,
    key: "j",
    keyCode: 74,
    ...overrides,
  };
}

Deno.test("Desktop reserves every native IME key outside the Vim Normal sink", () => {
  assertEquals(
    desktopImeKeyIsReserved(key({ isComposing: true }), false, false),
    true,
  );
  assertEquals(
    desktopImeKeyIsReserved(key({ key: "Process" }), false, false),
    true,
  );
  assertEquals(
    desktopImeKeyIsReserved(key({ keyCode: 229 }), false, false),
    true,
  );
  assertEquals(
    desktopImeKeyIsReserved(key({ key: "Dead" }), false, false),
    true,
  );
  assertEquals(desktopImeKeyIsReserved(key(), false, false), false);
});

Deno.test("a real composition suspends even the Vim Normal command sink", () => {
  assertEquals(
    desktopImeKeyIsReserved(key({ key: "Process" }), true, false),
    false,
  );
  assertEquals(desktopImeKeyIsReserved(key(), true, true), true);
});

Deno.test("every Desktop shortcut owner checks the shared IME boundary", () => {
  for (
    const [name, source, minimum] of [
      ["session surfaces", appSource, 3],
      ["command palette", hostSource, 1],
      ["workspace provider", providerSource, 1],
      ["top bar overlays", topBarSource, 2],
      ["transcript inspector", transcriptSource, 1],
      ["question page navigation", exploreSource, 2],
    ] as const
  ) {
    const checks = source.match(/desktopImeOwnsKey\(/gu)?.length ?? 0;
    assert(
      checks >= minimum,
      name + " has " + String(checks) + " IME checks; expected " +
        String(minimum),
    );
  }
});
