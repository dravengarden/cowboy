import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import type { DesktopCommand } from "./DesktopCommandProvider.tsx";
import {
  filterPaletteCommands,
  firstSelectablePaletteIndex,
  stepPaletteSelection,
} from "./commandPaletteModel.ts";

function cmd(
  id: string,
  title: string,
  opts: Partial<DesktopCommand> = {},
): DesktopCommand {
  return {
    id,
    title,
    group: opts.group ?? "Workspace",
    run: () => {},
    ...opts,
  };
}

test("filterPaletteCommands excludes palette opener and prioritizes available commands", () => {
  const commands = [
    cmd("commandPalette.open", "Command Palette"),
    cmd("review.diffLayout", "Review: Unified or split diff", {
      group: "Review",
      when: () => false,
    }),
    cmd("workspace.togglePrompt", "Collapse Prompt", {
      group: "Workspace",
      when: () => true,
    }),
    cmd("review.open", "Open Review", {
      group: "Review",
      when: () => true,
    }),
  ];

  const all = filterPaletteCommands(commands, "");
  assertEquals(
    all.map((entry) => `${entry.command.id}:${entry.available ? "on" : "off"}`),
    [
      "workspace.togglePrompt:on",
      "review.open:on",
      "review.diffLayout:off",
    ],
  );
});

test("filterPaletteCommands matches multi-token queries and title subsequences", () => {
  const commands = [
    cmd("review.diffLayout", "Review: Unified or split diff", {
      group: "Review",
      description: "Toggle split diff layout",
    }),
    cmd("workspace.togglePrompt", "Collapse Prompt", {
      group: "Workspace",
      description: "Show or hide the Prompt",
    }),
    cmd("session.rename", "Rename Session", {
      group: "Sessions",
    }),
  ];

  const multi = filterPaletteCommands(commands, "review diff");
  assertEquals(multi.map((entry) => entry.command.id), ["review.diffLayout"]);

  const fuzzy = filterPaletteCommands(commands, "clsp");
  assertEquals(fuzzy.map((entry) => entry.command.id), ["workspace.togglePrompt"]);

  assertEquals(filterPaletteCommands(commands, "nonexistent xyz"), []);
});

test("stepPaletteSelection skips unavailable commands and clamps cleanly", () => {
  const entries = [
    { command: cmd("a", "A"), available: true },
    { command: cmd("b", "B"), available: false },
    { command: cmd("c", "C"), available: true },
    { command: cmd("d", "D"), available: false },
  ];

  assertEquals(firstSelectablePaletteIndex(entries), 0);
  assertEquals(stepPaletteSelection(entries, 0, 1), 2);
  assertEquals(stepPaletteSelection(entries, 2, 1), 2);
  assertEquals(stepPaletteSelection(entries, 2, -1), 0);
  assertEquals(stepPaletteSelection(entries, 0, -1), 0);

  const allDisabled = [
    { command: cmd("a", "A"), available: false },
    { command: cmd("b", "B"), available: false },
  ];
  assertEquals(firstSelectablePaletteIndex(allDisabled), 0);
  assertEquals(stepPaletteSelection(allDisabled, 0, 1), 1);
  assertEquals(stepPaletteSelection(allDisabled, 1, 1), 1);
  assertEquals(stepPaletteSelection([], 0, 1), 0);
});
