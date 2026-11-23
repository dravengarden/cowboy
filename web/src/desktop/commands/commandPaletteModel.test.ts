import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import type { DesktopCommand } from "./DesktopCommandProvider.tsx";
import {
  filterPaletteCommands,
  firstSelectablePaletteIndex,
  loadRecentPaletteCommands,
  PALETTE_RECENT_LIMIT,
  recordRecentPaletteCommand,
  saveRecentPaletteCommands,
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
    { command: cmd("a", "A"), available: true, recent: false },
    { command: cmd("b", "B"), available: false, recent: false },
    { command: cmd("c", "C"), available: true, recent: false },
    { command: cmd("d", "D"), available: false, recent: false },
  ];

  assertEquals(firstSelectablePaletteIndex(entries), 0);
  assertEquals(stepPaletteSelection(entries, 0, 1), 2);
  assertEquals(stepPaletteSelection(entries, 2, 1), 2);
  assertEquals(stepPaletteSelection(entries, 2, -1), 0);
  assertEquals(stepPaletteSelection(entries, 0, -1), 0);

  const allDisabled = [
    { command: cmd("a", "A"), available: false, recent: false },
    { command: cmd("b", "B"), available: false, recent: false },
  ];
  assertEquals(firstSelectablePaletteIndex(allDisabled), 0);
  assertEquals(stepPaletteSelection(allDisabled, 0, 1), 1);
  assertEquals(stepPaletteSelection(allDisabled, 1, 1), 1);
  assertEquals(stepPaletteSelection([], 0, 1), 0);
});

test("filterPaletteCommands floats MRU recent commands to the top and persists them safely", () => {
  const commands = [
    cmd("a.first", "Alpha Action"),
    cmd("b.second", "Beta Action"),
    cmd("c.third", "Gamma Action"),
    cmd("d.disabled", "Delta Action", { when: () => false }),
  ];
  const recent = ["c.third", "d.disabled", "b.second"];
  const browsing = filterPaletteCommands(commands, "", recent);
  assertEquals(
    browsing.map((entry) => `${entry.command.id}:${entry.recent ? "recent" : "plain"}`),
    [
      "c.third:recent",
      "b.second:recent",
      "a.first:plain",
      "d.disabled:recent",
    ],
  );
  // Tie-break equal-score search results by recency.
  const search = filterPaletteCommands(commands, "action", ["b.second"]);
  assertEquals(search[0]?.command.id, "b.second");

  let mru: string[] = [];
  for (const id of ["c1", "c2", "c3", "c4", "c5", "c6", "c2", "commandPalette.open"]) {
    mru = recordRecentPaletteCommand(mru, id);
  }
  assertEquals(mru.length, PALETTE_RECENT_LIMIT);
  assertEquals(mru, ["c2", "c6", "c5", "c4", "c3"]);

  const store = new Map<string, string>();
  const mockStorage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => {
      store.set(key, value);
    },
  };
  saveRecentPaletteCommands(mru, mockStorage);
  assertEquals(loadRecentPaletteCommands(mockStorage), mru);
  assertEquals(loadRecentPaletteCommands({ getItem: () => "{broken" }), []);
});
