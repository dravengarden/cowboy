import { assertEquals } from "jsr:@std/assert";
import {
  desktopComposerOwnsWorkspace,
  desktopDefaultRegionForPane,
  desktopShouldBlockStaleVimSink,
  desktopVimSinkShouldHandleKeys,
} from "./desktopComposerOwnership.ts";

Deno.test("the Vim sink owns keys only while Prompt is the focused region", () => {
  assertEquals(desktopComposerOwnsWorkspace("prompt.composer"), true);
  assertEquals(desktopComposerOwnsWorkspace("conversation.transcript"), false);
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      focusedRegion: "conversation.transcript",
      targetIsVimSink: true,
    }),
    false,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      focusedRegion: "prompt.composer",
      targetIsVimSink: true,
    }),
    true,
  );
  assertEquals(
    desktopVimSinkShouldHandleKeys({
      focusedRegion: "prompt.composer",
      targetIsVimSink: false,
    }),
    false,
  );
  assertEquals(
    desktopShouldBlockStaleVimSink({
      focusedRegion: "conversation.transcript",
      targetIsVimSink: true,
    }),
    true,
  );
  assertEquals(
    desktopShouldBlockStaleVimSink({
      focusedRegion: "conversation.transcript",
      targetIsVimSink: false,
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
  assertEquals(vim.includes('dataset.desktopFocused !== "true"'), true);
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
