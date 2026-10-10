import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const dialogSource = await readFile(
  new URL("./SessionReloadDialog.tsx", import.meta.url), "utf8",
);

test("desktop session dialog exposes confirmed runtime reload", () => {
  assert(appSource.includes('data-session-shortcut="l"'));
  assert(appSource.includes("onRequestReload(menuAnchor.row)"));
  assert(appSource.includes("<SessionReloadDialog"));
});

test("mobile reload is a labeled session action behind the shared confirmation", () => {
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

test("desktop reload confirmation names every preserved session state", () => {
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
  assertEquals(
    dialogSource.match(/sx=\{\{ minHeight: 44 \}\}/g)?.length,
    2,
  );
});

test("mobile Provider update is badged on Options and offered in the session sheet", () => {
  assert(composerSource.includes("data-provider-update-badge"));
  assert(
    composerSource.includes(
      "<ProviderUpdateCard session={session} onUpdate={onProviderUpdate} />",
    ),
  );
  assert(
    composerSource.includes("session={open && updateConfirm ? session : null}"),
  );
});

test("session rows show a passive Provider update badge beside placement", () => {
  assert(appSource.includes("<SessionUpdateBadge session={s} />"));
});
