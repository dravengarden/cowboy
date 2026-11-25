import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import type { QueuedMessage } from "../store.ts";
import {
  buildToolbarPresentation,
  type NewPromptCallbacks,
} from "./useDesktopBufferActions.ts";

const NEW_PROMPT: NewPromptCallbacks = {
  sendable: true,
  canInsert: true,
  canJumpFront: false,
  canForce: false,
  canCurrentTurn: false,
  pending: false,
  progress: false,
  sendLabel: "Send",
  sendDescription: "Send prompt",
  unavailableReason: "",
  onAttach: () => {},
  onSaveDraft: () => {},
  onSchedule: () => {},
  onJumpFront: () => {},
  onForce: () => {},
  onCurrentTurn: () => {},
  onSubmit: () => {},
  getText: () => "",
};

const ROW: QueuedMessage = { id: "row-1", text: "hello", attachments: [] };

test("Queue edit uses Mod+S Save as primary action and keeps Save icon for drafts", () => {
  const queued = buildToolbarPresentation(
    {
      activeBuffer: { kind: "queued", id: "row-1", origin: "list", entryMode: "insert" },
      resolvedRow: ROW,
      bufferMeta: { sendable: true, dirty: true, saving: false, scheduled: false },
      queue: [ROW],
      newPrompt: NEW_PROMPT,
    },
    true,
  );
  assertEquals(queued.primaryAction, "save");
  assertEquals(queued.sendLabel, "Save");
  assertEquals(queued.sendIconKind, "check");
  assertEquals(queued.saveIconKind, "save");

  const draftIdle = buildToolbarPresentation(
    {
      activeBuffer: { kind: "draft", id: "row-1", origin: "list", entryMode: "insert" },
      resolvedRow: ROW,
      bufferMeta: { sendable: true, dirty: false, saving: false, scheduled: false },
      queue: [],
      newPrompt: NEW_PROMPT,
    },
    false,
  );
  assertEquals(draftIdle.primaryAction, "submit");
  assertEquals(draftIdle.sendLabel, "Send");
  assertEquals(draftIdle.saveIconKind, "save");

  const draftBusy = buildToolbarPresentation(
    {
      activeBuffer: { kind: "draft", id: "row-1", origin: "list", entryMode: "insert" },
      resolvedRow: ROW,
      bufferMeta: { sendable: true, dirty: false, saving: false, scheduled: false },
      queue: [],
      newPrompt: NEW_PROMPT,
    },
    true,
  );
  assertEquals(draftBusy.primaryAction, "submit");
  assertEquals(draftBusy.sendLabel, "Send…");
  assertEquals(draftBusy.saveIconKind, "save");

  const scheduled = buildToolbarPresentation(
    {
      activeBuffer: { kind: "draft", id: "row-1", origin: "list", entryMode: "insert" },
      resolvedRow: ROW,
      bufferMeta: { sendable: true, dirty: true, saving: false, scheduled: true },
      queue: [],
      newPrompt: NEW_PROMPT,
    },
    false,
  );
  assertEquals(scheduled.primaryAction, "submit");
  assertEquals(scheduled.sendLabel, "Deliver…");
  assertEquals(scheduled.saveIconKind, "save");

  const fresh = buildToolbarPresentation(
    {
      activeBuffer: { kind: "new" },
      resolvedRow: null,
      bufferMeta: { sendable: false, dirty: false, saving: false, scheduled: false },
      queue: [],
      newPrompt: NEW_PROMPT,
    },
    false,
  );
  assertEquals(fresh.primaryAction, "submit");
  assertEquals(fresh.saveLabel, "Draft");
  assertEquals(fresh.saveIconKind, "draft");
});
