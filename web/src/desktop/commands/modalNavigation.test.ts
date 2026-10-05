import { assertEquals } from "jsr:@std/assert";
import { modalRows } from "./modalNavigation.ts";

const box = (top: number, height: number, left: number) => ({
  top,
  bottom: top + height,
  left,
  right: left + 40,
});

Deno.test("modal rows group controls that share a vertical band", () => {
  const stops = {
    session: box(0, 40, 0),
    draft: box(0, 40, 100),
    folder: box(2, 36, 200),
    title: box(60, 56, 0),
    clear: box(70, 36, 300),
    project: box(140, 56, 0),
    cancel: box(220, 36, 200),
    create: box(218, 40, 300),
  };
  const rows = modalRows(
    Object.keys(stops) as (keyof typeof stops)[],
    (name) => stops[name],
  );
  assertEquals(rows, [
    ["session", "draft", "folder"],
    ["title", "clear"],
    ["project"],
    ["cancel", "create"],
  ]);
});
