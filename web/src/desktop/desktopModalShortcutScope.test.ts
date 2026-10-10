import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

const modalSource = await readFile(
  new URL("./DesktopModal.tsx", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("../App.tsx", import.meta.url), "utf8",
);
const mainSource = await readFile(
  new URL("../main.tsx", import.meta.url), "utf8",
);
const desktopAppSource = await readFile(
  new URL("./DesktopApp.tsx", import.meta.url), "utf8",
);

test("DesktopModal owns shortcuts across its complete dialog root", () => {
  assert(
    modalSource.includes(
      "onShortcutKeyDown?: KeyboardEventHandler<HTMLDivElement>",
    ),
  );
  assert(!modalSource.includes("onKeyDown={onShortcutKeyDown}"));
  assert(modalSource.includes("root: { onKeyDown: onShortcutKeyDown }"));
});

test("desktop Session actions use the modal-wide shortcut scope", () => {
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

test("Desktop owns the native Escape guard instead of the shared app root", () => {
  assert(!mainSource.includes("claimModalEscape"));
  assert(!mainSource.includes('addEventListener("keydown"'));
  assert(desktopAppSource.includes("installDesktopNativeEscapeGuard"));
});
