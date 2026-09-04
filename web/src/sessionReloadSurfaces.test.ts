import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const dialogSource = await Deno.readTextFile(
  new URL("./SessionReloadDialog.tsx", import.meta.url),
);

Deno.test("desktop session dialog exposes confirmed runtime reload", () => {
  assert(appSource.includes('data-session-shortcut="l"'));
  assert(appSource.includes("onRequestReload(menuAnchor.row)"));
  assert(appSource.includes("<SessionReloadDialog"));
});

Deno.test("mobile reload is a labeled session action behind the shared confirmation", () => {
  assert(
    composerSource.includes(
      'aria-label="reload session from session settings"',
    ),
  );
  assert(
    composerSource.includes("onReload={(): void => setReloadConfirm(true)}"),
  );
  assert(composerSource.includes("<SessionReloadDialog"));
  assert(
    composerSource.includes("session={open && reloadConfirm ? session : null}"),
  );
  assertEquals(composerSource.includes("data-session-provider-reload"), false);
  assertEquals(composerSource.includes("reloadSession(session.id)"), false);
});

Deno.test("desktop reload confirmation names every preserved session state", () => {
  for (const phrase of [
    "Conversation history",
    "session ID",
    "title",
    "working directory",
    "queue",
    "drafts",
    "saved agent configuration",
  ]) {
    assert(dialogSource.includes(phrase));
  }
  assert(dialogSource.includes("The current turn will stop"));
  assert(dialogSource.includes("confirmActiveTurn: activeTurn"));
  assert(dialogSource.includes('activeTurn ? "Stop & reload" : "Reload"'));
});
