/** Real-browser checks of the remote copy prompt and the Remote clipboard
 * sheet (`MachineBrowserClipboard.tsx`): a remote copy raises the prompt,
 * one tap copies it here, a refused write opens the clip selected for the
 * system's Copy, and history survives a reconnect. */
import { Box } from "@mui/material";
import { useReducer } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceContext } from "../surface/SurfaceProfile";
import { classifySurface } from "../surface/profile";
import {
  type BrowserEvent,
  type BrowserModel,
  INITIAL_BROWSER_MODEL,
  reduceBrowser,
} from "./browserModel";
import { useRemoteClipboard } from "./MachineBrowserClipboard";

let send: ((event: BrowserEvent) => void) | undefined;
let open: (() => void) | undefined;

function Harness(): React.JSX.Element {
  const [model, dispatch] = useReducer(
    (state: BrowserModel, event: BrowserEvent) => reduceBrowser(state, event),
    INITIAL_BROWSER_MODEL,
  );
  send = dispatch;
  const clipboard = useRemoteClipboard(model, dispatch);
  open = clipboard.openSheet;
  return (
    <Box sx={{ position: "relative", width: 390, height: 600 }}>
      {clipboard.prompt}
      {clipboard.dialog}
    </Box>
  );
}

export async function remoteClipboardBrowser(): Promise<string[]> {
  const checks: string[] = [];
  const check = (ok: boolean, name: string): void => {
    if (!ok) throw new Error(`remote clipboard: ${name}`);
    checks.push(`remote clipboard: ${name}`);
  };
  const tick = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 20));
  const until = async (predicate: () => boolean): Promise<void> => {
    for (let index = 0; index < 100 && !predicate(); index += 1) await tick();
  };
  const clipboard = globalThis.navigator.clipboard as Clipboard | undefined;
  const original = clipboard?.writeText;
  const execCommand = document.execCommand;
  const written: string[] = [];
  let refuse = false;
  const writeText = async (text: string): Promise<void> => {
    if (refuse) throw new DOMException("denied", "NotAllowedError");
    written.push(text);
  };
  if (clipboard)
    Object.defineProperty(clipboard, "writeText", { configurable: true, value: writeText });
  else
    Object.defineProperty(globalThis.navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
  document.execCommand = () => !refuse;
  // A phone: the sheet is a bottom sheet and taps are the only gesture.
  const surface = classifySurface({
    finePointer: false,
    hover: false,
    tabletWidth: false,
    touchCapable: true,
    desktopHost: false,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() =>
    root.render(
      <BrowserProductTheme>
        <SurfaceContext.Provider value={surface}>
          <Harness />
        </SurfaceContext.Provider>
      </BrowserProductTheme>,
    ),
  );
  const prompt = () => document.querySelector<HTMLElement>("[data-machine-browser-clip-prompt]");
  const button = (scope: ParentNode, name: string) =>
    [...scope.querySelectorAll<HTMLButtonElement>("button")].find(
      (node) => node.textContent === name,
    );
  try {
    flushSync(() => send?.({ type: "remote_clipboard", text: "验证码 4242 🎉", now: Date.now() }));
    await until(() => prompt() !== null);
    check(
      prompt()?.textContent?.includes("验证码 4242 🎉") === true,
      "a remote copy raises the prompt with its preview",
    );
    button(prompt()!, "Copy")?.click();
    await until(() => written.length === 1);
    check(written[0] === "验证码 4242 🎉", "one tap copies the exact text to this device");
    await until(() => prompt()?.textContent?.includes("Copied to this device") === true);
    check(button(prompt()!, "Copied")?.disabled === true, "the prompt confirms the copy");
    await until(() => prompt() === null);
    check(prompt() === null, "the prompt leaves after confirming");

    // The display server re-announcing the same clipboard after a reconnect
    // is not a new copy.
    flushSync(() => send?.({ type: "remote_clipboard", text: "验证码 4242 🎉", now: Date.now() }));
    await tick();
    check(prompt() === null, "a re-announced clipboard raises no prompt");

    refuse = true;
    flushSync(() =>
      send?.({ type: "remote_clipboard", text: "second line\nof text", now: Date.now() }),
    );
    await until(() => prompt() !== null);
    button(prompt()!, "Copy")?.click();
    const field = () =>
      document.querySelector<HTMLTextAreaElement>("[data-machine-browser-clip-text]");
    await until(() => field() !== null);
    const text = field();
    check(
      text?.readOnly === true && text.value === "second line\nof text",
      "a refused write opens the full text read-only",
    );
    const selected = (): boolean =>
      text !== null && text.selectionStart === 0 && text.selectionEnd === text.value.length;
    await until(selected);
    check(selected(), "the refused clip is selected for the system's Copy");
    check(
      document.querySelectorAll("[data-machine-browser-clip]").length === 2,
      "the sheet lists every held copy",
    );
    check(prompt() === null, "the prompt steps aside while the sheet is open");
    button(document, "Done")?.click();
    await until(() => field() === null);

    // A dropped viewer and reattach keep the history.
    flushSync(() =>
      send?.({ type: "viewer_ended", message: "dropped", resumable: true, now: Date.now() }),
    );
    flushSync(() => open?.());
    await until(() => document.querySelectorAll("[data-machine-browser-clip]").length === 2);
    check(
      document.querySelectorAll("[data-machine-browser-clip]").length === 2,
      "copies survive a dropped viewer",
    );
    button(document, "Clear all")?.click();
    await tick();
    flushSync(() => open?.());
    await until(() => document.body.textContent?.includes("Nothing copied yet") === true);
    check(
      document.body.textContent?.includes("Nothing copied yet") === true,
      "Clear all empties the history",
    );
    button(document, "Done")?.click();
    await tick();
  } finally {
    root.unmount();
    host.remove();
    document.execCommand = execCommand;
    if (clipboard && original)
      Object.defineProperty(clipboard, "writeText", { configurable: true, value: original });
  }
  return checks;
}
