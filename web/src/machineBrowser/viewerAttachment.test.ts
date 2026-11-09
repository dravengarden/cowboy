import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { LANDSCAPE_MODE } from "./api";
import { attachViewer } from "./viewerAttachment";

function attached(touch: boolean, streamOpen = true) {
  let open = streamOpen;
  const opened: { width: number; height: number }[] = [];
  const resizes: [number, number][] = [];
  let closes = 0;
  let changed: (() => void) | undefined;
  let watching = false;
  const attachment = attachViewer({
    // An iPad in landscape: wide enough for the largest desktop mode.
    frame: () => ({ width: 2000, height: 1200 }),
    portrait: () => false,
    touch,
    debounceMs: 1,
    open: (viewport) => {
      opened.push({ width: viewport.width, height: viewport.height });
      return {
        resize: (width: number, height: number) => {
          if (!open) return false;
          resizes.push([width, height]);
          return true;
        },
        close: () => {
          closes += 1;
        },
      };
    },
    watch: (listener) => {
      changed = listener;
      watching = true;
      return () => {
        watching = false;
      };
    },
  });
  return {
    attachment,
    opened,
    resizes,
    closes: () => closes,
    changed: () => changed?.(),
    watching: () => watching,
    openStream: () => {
      open = true;
      attachment.streamOpened();
    },
  };
}

test("a touch-mode flip resizes the same session once and never reattaches", () => {
  const view = attached(true);
  const session = view.attachment.session;
  assertEquals(view.opened, [LANDSCAPE_MODE]);
  // An iPad trackpad connects: pointer input, the largest desktop mode.
  view.attachment.setTouch(false);
  assertEquals(view.resizes, [[1920, 1080]]);
  assertEquals(view.attachment.session, session);
  assertEquals(view.opened.length, 1);
  assertEquals(view.closes(), 0);
  // Repeating the same mode sends nothing.
  view.attachment.setTouch(false);
  assertEquals(view.resizes.length, 1);
  // The trackpad goes away again.
  view.attachment.setTouch(true);
  assertEquals(view.resizes, [
    [1920, 1080],
    [LANDSCAPE_MODE.width, LANDSCAPE_MODE.height],
  ]);
  assertEquals(view.closes(), 0);
});

test("a frame change in the same mode sends no resize; close ends the session and the watch", async () => {
  const view = attached(true);
  view.changed();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(view.resizes, []);
  view.attachment.close();
  assertEquals(view.closes(), 1);
  assertEquals(view.watching(), false);
});

test("a mode change before the stream opens is sent once it opens", async () => {
  const view = attached(true, false);
  // The trackpad connects while the device proof or socket is pending.
  view.attachment.setTouch(false);
  assertEquals(view.resizes, []);
  // A later frame observation still retries: nothing counted as sent.
  view.changed();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(view.resizes, []);
  view.openStream();
  assertEquals(view.resizes, [[1920, 1080]]);
  // Only once.
  view.attachment.streamOpened();
  view.changed();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(view.resizes, [[1920, 1080]]);
  assertEquals(view.opened, [LANDSCAPE_MODE]);
  assertEquals(view.closes(), 0);
});

test("a stream opening in the mode it was requested with sends no resize", () => {
  const view = attached(true, false);
  view.openStream();
  assertEquals(view.resizes, []);
});
