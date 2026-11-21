import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "../../tools/test-assert.ts";

const source = await readFile(new URL("./SessionList.tsx", import.meta.url), "utf8");

test("New session here closes the folder menu before the opener claims the keyboard", () => {
  const item = source.indexOf('<ListItemText primary="New session here" />');
  assert(item >= 0);
  const handler = source.slice(source.lastIndexOf("<MenuItem", item), item);
  const close = handler.indexOf("flushSync(() => setFolderMenu(null));");
  const open = handler.indexOf("onNewInFolder(folderMenuFolder.id);");
  // MUI's Menu restores focus to its kebab on close; that blur must happen
  // inside the tap but before the opener's in-gesture keyboard claim.
  assert(close >= 0 && open > close);
});
