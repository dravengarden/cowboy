import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./SessionFolderUi.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("folder sheets mount beside the session shells, not inline and not on body", () => {
  // SessionList renders inside the drawer: a transformed, overflow-hidden
  // layer stacked under the page peek. An inline fixed sheet is laid out
  // against that layer, clipped to the drawer width and covered by the page
  // (physical iPhone, 2026-09-17). A <body> portal escapes the
  // keyboard-resized app box instead: the prompt sat behind the iPad keyboard
  // and left the page double-lifted (physical iPad, same day). The only
  // placement proven on devices is the Rename shell's, so every sheet of this
  // module portals into the host App renders next to it.
  const sheets = [...source.matchAll(/<Sheet\b([\s\S]*?)>/g)];
  assert(sheets.length >= 4);
  for (const [, props] of sheets) {
    assertEquals(/^\s*portal\b/m.test(props ?? ""), false);
  }
  assertEquals(
    (source.match(/<InAppRoot>/g) ?? []).length,
    sheets.length,
  );
  assert(source.includes("createPortal(children, host)"));
  assertEquals(source.includes("document.body"), false);

  const host = appSource.indexOf(
    '<div {...{ [SESSION_FOLDER_SHEET_HOST]: "" }} />',
  );
  const rename = appSource.indexOf("{pendingRename && (", host);
  assert(host >= 0);
  assert(rename > host && rename - host < 200);

  // The shells are mounted by SessionList itself, which is why they cannot
  // rely on App's root-level placement like the session Rename/Delete shells.
  const list = appSource.indexOf("function SessionList(");
  const listEnd = appSource.indexOf("type WorkspaceWorkItem", list);
  const body = appSource.slice(list, listEnd);
  for (
    const shell of [
      "<FolderNameShell",
      "<FolderPickerShell",
      "<ProjectPickerShell",
      "<DeleteFolderShell",
    ]
  ) assert(body.includes(shell));
});

Deno.test("the folder name prompt keeps a two-button action row", () => {
  // A third action squeezed Create off a phone-width sheet; the organizer
  // lives under the field instead.
  const shell = source.slice(
    source.indexOf("export function FolderNameShell("),
    source.indexOf("export function FolderPickerShell("),
  );
  const actions = shell.slice(
    shell.indexOf("actions={"),
    shell.indexOf("<TextField"),
  );
  assertEquals(actions.includes("{extra}"), false);
  assertEquals((actions.match(/<Button\b/g) ?? []).length, 2);
  assert(shell.indexOf("{extra}") > shell.indexOf("<TextField"));
});
