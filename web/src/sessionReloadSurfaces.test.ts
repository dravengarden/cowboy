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

Deno.test("mobile Provider reload uses the existing card language and acts directly", () => {
  assert(
    composerSource.includes(
      'aria-label="reload session runtime from provider"',
    ),
  );
  assert(composerSource.includes("data-session-provider-reload"));
  assert(
    composerSource.includes(
      "void action.run(() => reloadSession(sessionId));",
    ),
  );
  assert(composerSource.includes("minHeight: 58"));
  assertEquals(composerSource.includes("<SessionReloadDialog"), false);
  assertEquals(composerSource.includes("reloadConfirm"), false);
  assertEquals(
    composerSource.includes(
      'aria-label="reload session from session settings"',
    ),
    false,
  );
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
});
