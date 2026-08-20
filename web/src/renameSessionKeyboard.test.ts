import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("session rename focuses its real input inside the initiating tap", () => {
  const menuClose = appSource.indexOf(
    "flushSync(() => setMenuAnchor(null));",
  );
  const requestRename = appSource.indexOf(
    "onRequestRename(session);",
    menuClose,
  );
  assert(menuClose >= 0);
  assert(requestRename > menuClose);

  assert(
    appSource.includes("flushSync(() => setPendingRename(s));"),
  );
  assert(
    appSource.includes("{pendingRename && (\n                <RenameSessionShell"),
  );

  const renameShell = appSource.indexOf("function RenameSessionShell(");
  const layoutFocus = appSource.indexOf("useLayoutEffect(() => {", renameShell);
  const focus = appSource.indexOf("inputRef.current?.focus();", layoutFocus);
  const select = appSource.indexOf("inputRef.current?.select();", focus);
  assert(renameShell >= 0);
  assert(layoutFocus > renameShell);
  assert(focus > layoutFocus);
  assert(select > focus);

  const renameOpenerStart = appSource.indexOf(
    "onRequestRename={(s): void => {",
  );
  const renameOpener = appSource.slice(
    renameOpenerStart,
    appSource.indexOf("loaded={sessionsLoaded}", renameOpenerStart),
  );
  assertEquals(renameOpener.includes("claimKeyboard()"), false);
  assertEquals(renameOpener.includes("setTimeout"), false);
});
