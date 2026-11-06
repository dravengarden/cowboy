import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  nextRunConfigChoiceIndex,
  runConfigKeyAction,
  runConfigMotionAction,
} from "./runConfigKeyboard.ts";

const controlsSource = await readFile(
  new URL("./RunConfigurationDialog.tsx", import.meta.url), "utf8",
);
const navigationSource = await readFile(
  new URL("./runConfigNavigation.ts", import.meta.url), "utf8",
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
  assertEquals(runConfigKeyAction("p"), { type: "direct", shortcut: "p" });
  assertEquals(runConfigKeyAction("x"), null);
});

test("run configuration number keys select every visible preset", () => {
  assertEquals(runConfigKeyAction("1"), { type: "preset", index: 0 });
  assertEquals(runConfigKeyAction("3"), { type: "preset", index: 2 });
  // The bar advertises 1–6 for six presets; 4–6 used to do nothing.
  assertEquals(runConfigKeyAction("6"), { type: "preset", index: 5 });
  assertEquals(runConfigKeyAction("9"), { type: "preset", index: 8 });
  assertEquals(runConfigKeyAction("0"), null);
});

test("run configuration panes and ends follow Vim", () => {
  assertEquals(runConfigKeyAction("]"), { type: "pane", delta: 1, wrap: true });
  assertEquals(runConfigKeyAction("["), {
    type: "pane",
    delta: -1,
    wrap: true,
  });
  assertEquals(runConfigMotionAction("l"), {
    type: "pane",
    delta: 1,
    wrap: false,
  });
  assertEquals(runConfigMotionAction("j"), { type: "field", delta: 1 });
  assertEquals(runConfigKeyAction("g"), { type: "go" });
  assertEquals(runConfigKeyAction("G"), { type: "edge", edge: "last" });
});

test("choice navigation clamps while direct field actions wrap", () => {
  assertEquals(nextRunConfigChoiceIndex(3, 0, -1, false), 0);
  assertEquals(nextRunConfigChoiceIndex(3, 2, 1, false), 2);
  assertEquals(nextRunConfigChoiceIndex(3, 2, 1, true), 0);
  assertEquals(nextRunConfigChoiceIndex(3, 0, -1, true), 2);
  assertEquals(nextRunConfigChoiceIndex(0, 0, 1, true), -1);
});

test("run configuration shortcuts apply choices instead of only moving focus", () => {
  assertEquals(navigationSource.includes("choice.click();"), true);
  assertEquals(
    navigationSource.includes('choice.hasAttribute("data-config-select")'),
    true,
  );
  assertEquals(navigationSource.includes('new MouseEvent("mousedown"'), true);
  assertEquals(navigationSource.includes('action.type === "direct"'), true);
  assertEquals(
    controlsSource.includes('{ shortcut: "H/L", label: "Change" }'),
    true,
  );
  assertEquals(
    controlsSource.includes('direct.join("/")'),
    true,
  );
  assertEquals(navigationSource.includes("focusConfigRow(firstRow)"), true);
});

test("run configuration fills the window as side-by-side panes", () => {
  assertEquals(controlsSource.includes('title="Run configuration"'), true);
  assertEquals(/title="Run configuration"[\s\S]{0,400}\n\s+fill\n/.test(controlsSource), true);
  for (const pane of ["agent", "context", "tools"]) {
    assertEquals(controlsSource.includes(`pane="${pane}"`), true);
  }
  assertEquals(
    controlsSource.includes(`gridTemplateAreas: '"agent context" "tools context"'`),
    true,
  );
});
