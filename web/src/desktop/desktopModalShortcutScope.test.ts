import { assert } from "jsr:@std/assert";

const modalSource = await Deno.readTextFile(
  new URL("./DesktopModal.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
);
const mainSource = await Deno.readTextFile(
  new URL("../main.tsx", import.meta.url),
);

Deno.test("DesktopModal owns shortcuts across its complete dialog root", () => {
  assert(
    modalSource.includes(
      "onShortcutKeyDown?: KeyboardEventHandler<HTMLDivElement>",
    ),
  );
  assert(modalSource.includes("onKeyDown={onShortcutKeyDown}"));
});

Deno.test("desktop Session actions use the modal-wide shortcut scope", () => {
  const start = appSource.indexOf('title="Session"');
  const end = appSource.indexOf("</DesktopModalShell>", start);
  const sessionModal = appSource.slice(start, end);

  assert(start >= 0);
  assert(end > start);
  assert(sessionModal.includes("onShortcutKeyDown={(event): void =>"));
  assert(
    sessionModal.includes(
      "event.currentTarget.querySelector<HTMLButtonElement>",
    ),
  );
});

Deno.test("modal Escape reaches MUI before the native fullscreen guard", () => {
  assert(!mainSource.includes("claimModalEscape"));
  assert(!mainSource.includes('addEventListener("keydown", claimModalEscape, true)'));
  const bubbleGuard = mainSource.indexOf(
    'globalThis.addEventListener("keydown", (e: KeyboardEvent): void =>',
  );
  assert(bubbleGuard >= 0);
  assert(
    mainSource.slice(bubbleGuard).includes(
      'if (e.key === "Escape" && !isImeKeyEvent(e)) e.preventDefault();',
    ),
  );
});
