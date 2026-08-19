import { assertEquals } from "jsr:@std/assert";
import {
  desktopComposerOwnsWorkspace,
  desktopDefaultRegionForPane,
  desktopShouldBlockStaleVimSink,
  desktopVimSinkShouldHandleKeys,
} from "./desktopComposerOwnership.ts";

Deno.test("a Vim sink owns keys only while its containing region is focused", () => {
  assertEquals(desktopComposerOwnsWorkspace("prompt.composer"), true);
  assertEquals(desktopComposerOwnsWorkspace("conversation.transcript"), false);
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      targetIsVimSink: true,
      targetRegionFocused: false,
    }),
    false,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      targetIsVimSink: true,
      targetRegionFocused: true,
    }),
    true,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      targetIsVimSink: false,
      targetRegionFocused: true,
    }),
    false,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      targetIsVimSink: true,
      targetRegionFocused: true,
    }),
    true,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      targetIsVimSink: true,
      targetRegionFocused: true,
    }),
    true,
  );
  assertEquals(
    desktopShouldBlockStaleVimSink({
      targetIsVimSink: true,
      targetRegionFocused: false,
    }),
    true,
  );
  assertEquals(
    desktopShouldBlockStaleVimSink({
      targetIsVimSink: false,
      targetRegionFocused: false,
    }),
    false,
  );
});

Deno.test("workspace capture blocks a stale Prompt sink after Conversation is highlighted", async () => {
  const provider = await Deno.readTextFile(
    new URL("./commands/DesktopCommandProvider.tsx", import.meta.url),
  );
  const controller = await Deno.readTextFile(
    new URL("./DesktopWorkspaceController.tsx", import.meta.url),
  );
  const vim = await Deno.readTextFile(
    new URL("./vim/imeAutoInsertVim.ts", import.meta.url),
  );
  assertEquals(provider.includes("desktopShouldBlockStaleVimSink("), true);
  assertEquals(provider.includes("!textEditorOwnsKey && !event.metaKey && !event.altKey"), true);
  assertEquals(controller.includes("desktopRegionFromPointerTarget("), true);
  assertEquals(controller.includes("desktopPointerLeftComposer("), true);
  assertEquals(controller.includes("desktopPointerLeftRegion("), true);
  assertEquals(vim.includes('dataset.desktopFocused !== "true"'), true);
  assertEquals(vim.includes("if (!this.cm) this.connect();"), true);
  assertEquals(vim.includes("ownsFocus && this.cm &&"), true);
});

Deno.test("pointerdown on a Conversation tab leaves the Sessions region", async () => {
  const ownership = await Deno.readTextFile(
    new URL("./desktopComposerOwnership.ts", import.meta.url),
  );
  assertEquals(ownership.includes("export function desktopPointerLeftRegion("), true);
  assertEquals(ownership.includes('active.closest("[data-desktop-region]")'), true);
  assertEquals(ownership.includes("return !owned.contains(target);"), true);
});

Deno.test("Conversation chrome without a nested region still owns the transcript", () => {
  assertEquals(
    desktopDefaultRegionForPane("conversation"),
    "conversation.transcript",
  );
  assertEquals(desktopDefaultRegionForPane("sessions"), "sessions.list");
  assertEquals(desktopDefaultRegionForPane("prompt"), "prompt.composer");
  assertEquals(desktopDefaultRegionForPane("unknown"), null);
});
