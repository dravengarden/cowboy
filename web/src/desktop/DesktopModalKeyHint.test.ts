import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { desktopModalShortcutGroups } from "./DesktopModalKeyHint.tsx";

test("desktopModalShortcutGroups omits I Edit when modal has no text field and includes leader when requested", () => {
  const withoutField = desktopModalShortcutGroups("normal", {
    hasTextField: false,
    includeLeader: true,
  });
  const slots = withoutField.flatMap((group) => group.slots);
  assertEquals(slots.some((slot) => slot.shortcut === "I"), false);
  assertEquals(slots.some((slot) => slot.shortcut === "␣" && slot.label === "Labels"), true);
});

test("desktopModalShortcutGroups advertises [/] for tab stepping when Vim is enabled", () => {
  const vimOn = desktopModalShortcutGroups("normal", { tabs: 3, vim: true });
  const vimOff = desktopModalShortcutGroups("normal", { tabs: 3, vim: false });
  assertEquals(vimOn[0]?.slots.map((slot) => slot.shortcut), ["1…3", "[/]"]);
  assertEquals(vimOff[0]?.slots.map((slot) => slot.shortcut), ["1…3", "H/L"]);
});

test("desktopModalShortcutGroups insert mode only advertises Esc Normal", () => {
  assertEquals(desktopModalShortcutGroups("insert", { tabs: 3, hasTextField: true, vim: true }), [
    { slots: [{ shortcut: "Esc", label: "Normal" }] },
  ]);
});
