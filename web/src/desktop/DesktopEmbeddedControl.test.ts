import { assertEquals } from "jsr:@std/assert";
import { desktopEmbeddedControlIconSx } from "./DesktopEmbeddedIcon.ts";

const conversationControlsSource = await Deno.readTextFile(
  new URL("./DesktopConversationControls.tsx", import.meta.url),
);
const readingModeSource = await Deno.readTextFile(
  new URL("./DesktopReadingModeControl.tsx", import.meta.url),
);
const projectionToggleSource = await Deno.readTextFile(
  new URL("../explore/ProjectionToggle.tsx", import.meta.url),
);
const embeddedControlSource = await Deno.readTextFile(
  new URL("./DesktopEmbeddedControl.ts", import.meta.url),
);
const shortcutKeycapSource = await Deno.readTextFile(
  new URL("../ShortcutKeycap.tsx", import.meta.url),
);

Deno.test("desktop embedded control icons follow the global root font size", () => {
  assertEquals(desktopEmbeddedControlIconSx(), {
    fontSize: "calc(20px * var(--cowboy-font-scale, 1))",
    width: "calc(20px * var(--cowboy-font-scale, 1))",
    height: "calc(20px * var(--cowboy-font-scale, 1))",
    flexShrink: 0,
  });
});

Deno.test("desktop Follow delegates its glyph to the global font-scale primitive", () => {
  assertEquals(
    conversationControlsSource.includes(
      "startIcon={<South sx={desktopEmbeddedControlIconSx()} />}",
    ),
    true,
  );
  assertEquals(
    conversationControlsSource.includes('fontSize: "1.25rem"'),
    false,
  );
});

Deno.test("Conversation top-bar selection is not derived from shared shortcut availability", () => {
  for (const source of [
    projectionToggleSource,
    readingModeSource,
    conversationControlsSource,
  ]) {
    assertEquals(
      source.includes("desktopEmbeddedControlSx({ active: shortcutActive })"),
      false,
    );
  }
  assertEquals(projectionToggleSource.includes("<ToggleButtonGroup\n        exclusive"), true);
  assertEquals(conversationControlsSource.includes("aria-pressed={followingCurrentView}"), true);
  assertEquals(conversationControlsSource.includes('color="inherit"'), true);
  assertEquals(
    conversationControlsSource.includes('color={followingCurrentView ? "primary" : "inherit"}'),
    false,
  );
  assertEquals(conversationControlsSource.includes('bgcolor: "action.selected"'), false);
  assertEquals(conversationControlsSource.includes("accent={false}"), true);
});

Deno.test("idle Desktop controls use a neutral boundary", () => {
  assertEquals(embeddedControlSource.includes(": theme.palette.divider"), true);
  assertEquals(
    embeddedControlSource.includes("open ? 0.68 : active ? 0.5 : 0.3"),
    false,
  );
});

Deno.test("desktop shortcut keycap geometry follows root font size", () => {
  assertEquals(
    shortcutKeycapSource.includes(
      '? (rendered.length > 2 ? "1.5rem" : "1.125rem")',
    ),
    true,
  );
  assertEquals(
    shortcutKeycapSource.includes('height: compact ? "1.125rem"'),
    true,
  );
  assertEquals(shortcutKeycapSource.includes('px: compact ? "0.175rem" : "0.25rem"'), true);
});
