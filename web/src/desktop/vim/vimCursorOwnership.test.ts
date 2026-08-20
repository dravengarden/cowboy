import { assertEquals } from "jsr:@std/assert";

import {
  syncVimCursorOwnership,
  VIM_NATIVE_CARET_CLASS,
} from "./vimCursorOwnership.ts";

class FakeClassList {
  readonly values = new Set<string>();

  toggle(token: string, force?: boolean): boolean {
    const enabled = force ?? !this.values.has(token);
    if (enabled) this.values.add(token);
    else this.values.delete(token);
    return enabled;
  }
}

Deno.test("Insert owns the native caret independently of browser focus", () => {
  const classList = new FakeClassList();
  const root = { classList };

  syncVimCursorOwnership(root, true);
  assertEquals(classList.values.has(VIM_NATIVE_CARET_CLASS), true);

  syncVimCursorOwnership(root, false);
  assertEquals(classList.values.has(VIM_NATIVE_CARET_CLASS), false);
});

Deno.test("the Vim runtime and theme consume the explicit cursor owner", async () => {
  const runtimeSource = await Deno.readTextFile(
    new URL("./imeAutoInsertVim.ts", import.meta.url),
  );
  const themeSource = await Deno.readTextFile(
    new URL("../../cmTheme.ts", import.meta.url),
  );

  assertEquals(runtimeSource.includes("syncVimCursorOwnership("), true);
  assertEquals(themeSource.includes(`&.${VIM_NATIVE_CARET_CLASS}`), true);
  assertEquals(
    themeSource.includes(".cm-cursorLayer.cm-vimCursorLayer"),
    true,
  );
  assertEquals(
    themeSource.includes(
      "&.cm-focused .cm-cursorLayer.cm-vimCursorLayer",
    ),
    false,
    "browser focus must not hide Normal mode's Vim cursor",
  );
});
