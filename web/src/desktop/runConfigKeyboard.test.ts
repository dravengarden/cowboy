import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  nextRunConfigChoiceIndex,
  runConfigKeyAction,
} from "./runConfigKeyboard.ts";

const controlsSource = await readFile(
  new URL("./DesktopTopBarControls.tsx", import.meta.url), "utf8",
);

test("run configuration accepts standard arrows and Vim motions", () => {
  assertEquals(runConfigKeyAction("ArrowUp"), { type: "field", delta: -1 });
  assertEquals(runConfigKeyAction("j"), { type: "field", delta: 1 });
  assertEquals(runConfigKeyAction("ArrowLeft"), { type: "choice", delta: -1 });
  assertEquals(runConfigKeyAction("L"), { type: "choice", delta: 1 });
});

test("run configuration mnemonic keys are direct field actions", () => {
  assertEquals(runConfigKeyAction("A"), { type: "direct", shortcut: "a" });
  assertEquals(runConfigKeyAction("m"), { type: "direct", shortcut: "m" });
  assertEquals(runConfigKeyAction("x"), null);
});

test("run configuration number keys select recommended presets", () => {
  assertEquals(runConfigKeyAction("1"), { type: "preset", index: 0 });
  assertEquals(runConfigKeyAction("2"), { type: "preset", index: 1 });
  assertEquals(runConfigKeyAction("3"), { type: "preset", index: 2 });
});

test("choice navigation clamps while direct field actions wrap", () => {
  assertEquals(nextRunConfigChoiceIndex(3, 0, -1, false), 0);
  assertEquals(nextRunConfigChoiceIndex(3, 2, 1, false), 2);
  assertEquals(nextRunConfigChoiceIndex(3, 2, 1, true), 0);
  assertEquals(nextRunConfigChoiceIndex(3, 0, -1, true), 2);
  assertEquals(nextRunConfigChoiceIndex(0, 0, 1, true), -1);
});

test("run configuration shortcuts apply choices instead of only moving focus", () => {
  assertEquals(controlsSource.includes("choice.click();"), true);
  assertEquals(
    controlsSource.includes('choice.hasAttribute("data-config-select")'),
    true,
  );
  assertEquals(
    controlsSource.includes('new MouseEvent("mousedown"'),
    true,
  );
  assertEquals(controlsSource.includes('action.type === "direct"'), true);
  assertEquals(
    controlsSource.includes('{ shortcut: "←/→", label: "Change" }'),
    true,
  );
  assertEquals(
    controlsSource.includes('directConfigShortcuts.join("/")'),
    true,
  );
  assertEquals(
    controlsSource.includes("(selected ?? choices[0])?.focus();"),
    true,
  );
});
