import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { topBarRegionHints } from "./topBarRegionHints";

const topBar = (id: string, shortcut: string, when?: () => boolean) => ({
  id,
  title: id,
  shortcut,
  regions: ["topbar.controls"],
  ...(when ? { when } : {}),
});

test("the Top bar legend lists bare keys of commands that can run now", () => {
  const hints = topBarRegionHints([
    topBar("topbar.runConfiguration", "R"),
    topBar("topbar.usage", "U"),
    // Offered only while a verification is requested.
    topBar("topbar.reauthenticate", "A", () => false),
    topBar("topbar.providerUpdate", "N", () => true),
    topBar("topbar.reload", "L"),
    topBar("topbar.clear", "X", () => false),
    // Stop is a global chord, not a bare Top bar key.
    { id: "topbar.stop", title: "Stop", shortcut: "Mod+." },
    // Another region's bare key never appears here.
    { id: "conversation.toggleFollow", title: "Follow", shortcut: "F", regions: ["conversation.transcript"] },
  ]);
  assertEquals(hints, [
    { keys: "R", label: "Config" },
    { keys: "U", label: "Usage" },
    { keys: "N", label: "Update" },
    { keys: "L", label: "Reload" },
  ]);
});

test("an unknown Top bar command falls back to its title", () => {
  assertEquals(
    topBarRegionHints([topBar("topbar.future", "Z")]),
    [{ keys: "Z", label: "topbar.future" }],
  );
});
