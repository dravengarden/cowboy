import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  DESKTOP_APPROVAL_CHOICE_LIMIT,
  desktopApprovalLeaderKey,
} from "./DesktopApprovalCommands";
import { shortcutRegistrationConflict } from "./commands/shortcutRegistrationPolicy";
import {
  DESKTOP_WORKSPACE_COMMANDS,
  desktopLeaderSequence,
} from "./commands/workspaceShortcuts";

test("approval choices take the leader digits in order", () => {
  assertEquals(desktopApprovalLeaderKey(0), "1");
  assertEquals(desktopApprovalLeaderKey(8), "9");
  assertEquals(desktopApprovalLeaderKey(DESKTOP_APPROVAL_CHOICE_LIMIT), null);
  assertEquals(desktopApprovalLeaderKey(-1), null);
});

test("approval digits collide with no workspace leader key or each other", () => {
  const sequences = Array.from(
    { length: DESKTOP_APPROVAL_CHOICE_LIMIT },
    (_, index) => ({
      id: "approval.choice." + String(index + 1),
      sequence: desktopLeaderSequence(desktopApprovalLeaderKey(index)!),
    }),
  );
  for (const [index, command] of sequences.entries()) {
    assertEquals(DESKTOP_WORKSPACE_COMMANDS[command.sequence[1]!], undefined);
    assertEquals(
      shortcutRegistrationConflict(command, sequences.slice(0, index)),
      null,
    );
  }
});
