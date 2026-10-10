import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { dataTransferCarriesFiles } from "./composerFileDrop.ts";

const composer = await readFile(
  new URL("../Composer.tsx", import.meta.url), "utf8",
);
const editor = await readFile(
  new URL("../ComposerEditor.tsx", import.meta.url), "utf8",
);

test("only an OS file drag is a composer attachment drop", () => {
  assertEquals(dataTransferCarriesFiles({ types: ["Files"] }), true);
  assertEquals(
    dataTransferCarriesFiles({ types: ["text/uri-list", "Files"] }),
    true,
  );
  // Dragging selected text or a link keeps CodeMirror's native text drop.
  assertEquals(dataTransferCarriesFiles({ types: ["text/plain"] }), false);
  assertEquals(dataTransferCarriesFiles(null), false);
});

test("desktop composer card and editor both attach dropped files", () => {
  assert(composer.includes("useComposerFileDrop(\n    !touchInput,"));
  assert(composer.includes("addFiles(files, { preserveFocus: true })"));
  assert(composer.includes("{...fileDrop.handlers}"));
  assert(composer.includes('data-composer-file-drop="true"'));

  const drop = editor.slice(editor.indexOf("drop: (event, view): boolean => {"));
  const body = drop.slice(0, drop.indexOf("\n        },"));
  // The editor claims drops over its text at the drop point and prevents the
  // default, so the card target does not attach the same files twice.
  assert(body.includes("dataTransferCarriesFiles(event.dataTransfer)"));
  assert(body.includes("touchInput ||"));
  assert(body.includes("event.preventDefault()"));
  assert(body.includes("view.posAtCoords({ x: event.clientX, y: event.clientY })"));
  assert(body.includes("onPasteFilesRef.current(files)"));
});
