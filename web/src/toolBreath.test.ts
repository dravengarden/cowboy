import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { createTheme } from "@mui/material/styles";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import { toolBreathSx } from "./toolBreath.ts";

const source = await readFile(new URL("./toolBreath.ts", import.meta.url), "utf8");
const items = await readFile(new URL("./TranscriptItems.tsx", import.meta.url), "utf8");

test("a running tool card breathes by fill colour, never by a compositor property", () => {
  const frames = source.match(/keyframes`([\s\S]*?)`/)?.[1] ?? "";
  assert(frames.includes("background-color"));
  for (const property of ["opacity", "transform", "filter", "will-change"]) {
    assertEquals(frames.includes(property), false);
  }
  // The card must not be dimmed or lifted into a layer in any other way.
  const card = items.slice(items.indexOf("...toolBreathSx(running, theme)"));
  assertEquals(items.includes("${pulse}"), false);
  assert(card.length > 0);
});

test("only a running card animates, and it keeps its own paper fill", () => {
  const theme = createTheme();
  const idle = toolBreathSx(false, theme);
  assertEquals(idle.bgcolor, theme.palette.background.paper);
  assertEquals("animation" in idle, false);
  const running = toolBreathSx(true, theme) as Record<string, string>;
  assertEquals(running["bgcolor"], theme.palette.background.paper);
  assertEquals(running["--tool-fill"], theme.palette.background.paper);
  assert(running["--tool-fill-dim"] !== running["--tool-fill"]);
  assert(running["animation"]?.includes("1.6s"));
});
