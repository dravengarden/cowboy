import { assertEquals } from "jsr:@std/assert";
import {
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
  shouldRestoreDesktopComposerFocus,
} from "./desktopVimMountPolicy";

Deno.test("only a pending Desktop Vim runtime starts the preload promise", () => {
  assertEquals(shouldPreloadDesktopVim("desktop", true, "pending"), true);
  assertEquals(shouldPreloadDesktopVim("desktop", true, "ready"), false);
  assertEquals(shouldPreloadDesktopVim("desktop", true, "failed"), false);
  assertEquals(shouldPreloadDesktopVim("mobile", true, "pending"), false);
  assertEquals(shouldPreloadDesktopVim("desktop", false, "pending"), false);
});

Deno.test("ready Desktop Vim restores only an otherwise unowned focused composer", () => {
  assertEquals(
    shouldRestoreDesktopComposerFocus("desktop", true, "ready", true, true),
    true,
  );
  assertEquals(
    shouldRestoreDesktopComposerFocus("desktop", true, "pending", true, true),
    false,
  );
  assertEquals(
    shouldRestoreDesktopComposerFocus("desktop", true, "ready", false, true),
    false,
  );
  assertEquals(
    shouldRestoreDesktopComposerFocus("desktop", true, "ready", true, false),
    false,
  );
  assertEquals(
    shouldRestoreDesktopComposerFocus("mobile", true, "ready", true, true),
    false,
  );
});

Deno.test("Desktop Vim waits for its runtime before the interactive mount", () => {
  assertEquals(desktopVimMountPolicy("desktop", true, false), {
    awaitingRuntime: true,
    enableVim: false,
  });
  assertEquals(desktopVimMountPolicy("desktop", true, true), {
    awaitingRuntime: false,
    enableVim: true,
  });
});

Deno.test("touch surfaces never wait for or enable Desktop Vim", () => {
  for (const kind of ["mobile", "tablet"] as const) {
    assertEquals(desktopVimMountPolicy(kind, true, false), {
      awaitingRuntime: false,
      enableVim: false,
    });
  }
});

Deno.test("a failed Vim chunk leaves the Desktop composer usable", () => {
  assertEquals(desktopVimMountPolicy("desktop", true, false, true), {
    awaitingRuntime: false,
    enableVim: false,
  });
});
