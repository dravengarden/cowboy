import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "../browserProductTheme";
import { shortcutKey } from "./commands/imeShortcut";
import { DesktopModal } from "./DesktopModal";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

/** A Desktop dialog's letter actions under an idle macOS Pinyin source, which
 *  labels every keydown on dialog buttons `Process`/229 (the Session dialog's
 *  `F` Replace or fork). */
export async function checkImeShortcutChrome(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const ran: string[] = [];
  try {
    flushSync(() => root.render(
      <BrowserProductTheme>
        <DesktopModal
          open
          onClose={() => ran.push("close")}
          title="Session"
          onShortcutKeyDown={(event): void => {
            const key = shortcutKey(event.nativeEvent);
            if (key === null || event.metaKey || event.ctrlKey || event.altKey) return;
            const action = event.currentTarget.querySelector<HTMLButtonElement>(
              `[data-ime-fixture-shortcut="${key.toLowerCase()}"]`,
            );
            if (!action) return;
            event.preventDefault();
            action.click();
          }}
        >
          <button type="button" autoFocus data-ime-fixture-shortcut="f" onClick={() => ran.push("fork")}>
            Replace or fork
          </button>
          <button type="button" data-ime-fixture-shortcut="," onClick={() => ran.push("settings")}>
            Settings
          </button>
          <input aria-label="Fixture field" />
        </DesktopModal>
      </BrowserProductTheme>,
    ));
    await new Promise((resolve) => setTimeout(resolve, 300));
    const button = document.querySelector<HTMLButtonElement>("[data-ime-fixture-shortcut='f']");
    check(button, "The dialog renders its actions");
    button.focus();
    const press = (target: Element, init: KeyboardEventInit): KeyboardEvent => {
      const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
      target.dispatchEvent(event);
      return event;
    };
    const marked = { key: "Process", keyCode: 229 } as KeyboardEventInit;
    check(press(button, { ...marked, code: "KeyF" }).defaultPrevented, "Pinyin F on a dialog button runs its action");
    check(press(button, { ...marked, code: "Comma" }).defaultPrevented, "Pinyin comma runs the Settings action");
    check(ran.join() === "fork,settings", `Each IME-labelled letter runs once (${ran.join()})`);
    const field = document.querySelector<HTMLInputElement>("[aria-label='Fixture field']")!;
    field.focus();
    check(!press(field, { ...marked, code: "KeyF" }).defaultPrevented, "A field keeps its IME composition");
    check(ran.join() === "fork,settings", "Typing Pinyin in a field runs no action");
    return "Desktop dialog letters run by physical key under an idle CJK source; text fields keep the IME";
  } finally {
    root.unmount();
    container.remove();
  }
}
