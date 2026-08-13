import { assert } from "jsr:@std/assert";

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

Deno.test("mobile session settings sheet exposes the same reload dialog", () => {
  assert(composerSource.includes('aria-label="reload session from session settings"'));
  assert(composerSource.includes("onReload={(): void => setReloadConfirm(true)}"));
  assert(composerSource.includes("session={reloadConfirm ? session : null}"));
});

Deno.test("reload confirmation names every preserved session state", () => {
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
