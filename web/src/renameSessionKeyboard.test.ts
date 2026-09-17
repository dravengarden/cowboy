import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const focusHookSource = await Deno.readTextFile(
  new URL("./useDialogInputFocus.ts", import.meta.url),
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
  assert(/\{pendingRename && \(\s*<RenameSessionShell/u.test(appSource));

  // The shell delegates to the shared dialog-focus hook, whose first claim is
  // synchronous inside a layout effect (the in-tap iOS contract); only the
  // Desktop retry is deferred.
  const renameShell = appSource.indexOf("function RenameSessionShell(");
  const hookUse = appSource.indexOf(
    "useDialogInputFocus(inputRef, desktop);",
    renameShell,
  );
  assert(renameShell >= 0);
  assert(hookUse > renameShell);
  const layoutFocus = focusHookSource.indexOf("useLayoutEffect(() => {");
  const claim = focusHookSource.indexOf("claim();", layoutFocus);
  const retryGate = focusHookSource.indexOf(
    "if (!retry) return undefined;",
    claim,
  );
  const deferred = focusHookSource.indexOf("requestAnimationFrame(", retryGate);
  assert(layoutFocus >= 0);
  assert(claim > layoutFocus);
  assert(retryGate > claim);
  assert(deferred > retryGate);
  assert(focusHookSource.includes("target.focus({ preventScroll: true });"));
  assert(focusHookSource.includes("target.select();"));

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
