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
const desktopAppSource = await Deno.readTextFile(
  new URL("./DesktopApp.tsx", import.meta.url),
);

Deno.test("DesktopModal owns shortcuts across its complete dialog root", () => {
  assert(
    modalSource.includes(
      "onShortcutKeyDown?: KeyboardEventHandler<HTMLDivElement>",
    ),
  );
  assert(!modalSource.includes("onKeyDown={onShortcutKeyDown}"));
  assert(modalSource.includes("root: { onKeyDown: onShortcutKeyDown }"));
});

Deno.test("desktop Session actions use the modal-wide shortcut scope", () => {
  const start = appSource.indexOf('title="Session"');
  const end = appSource.indexOf("</DesktopModalShell>", start);
  const sessionModal = appSource.slice(start, end);

  assert(start >= 0);
  assert(end > start);
  assert(sessionModal.includes("onShortcutKeyDown={(event): void =>"));
  assert(
    /event\.currentTarget\.querySelector<\s*HTMLButtonElement\s*>/u.test(
      sessionModal,
    ),
  );
});

Deno.test("Desktop owns the native Escape guard instead of the shared app root", () => {
  assert(!mainSource.includes("claimModalEscape"));
  assert(!mainSource.includes('addEventListener("keydown"'));
  assert(desktopAppSource.includes("installDesktopNativeEscapeGuard"));
});
