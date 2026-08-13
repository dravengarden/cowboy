import { assert } from "jsr:@std/assert";

const modalSource = await Deno.readTextFile(
  new URL("./DesktopModal.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
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
