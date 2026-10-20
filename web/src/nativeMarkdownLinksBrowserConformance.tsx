import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import MarkdownImpl from "./MarkdownImpl";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 300));
function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

export async function checkNativeMarkdownLinks(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const native = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: { invoke: (command: string, args: Record<string, unknown>) => Promise<void> };
  };
  const previous = native.__TAURI_INTERNALS__;
  const urls: unknown[] = [];
  native.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
    check(command === "plugin:opener|open_url", "Link uses the OS opener");
    urls.push(args.url);
  } };
  const url = "https://example.com/start/#/device";
  const render = (key: string): void => flushSync(() => root.render(
    <SurfaceContext.Provider value={{ kind: "desktop", input: "keyboard-pointer", finePointer: true, hover: true, touchCapable: false }}><BrowserProductTheme>
      <MarkdownImpl key={key} text={`[SSO sign-in](${url})`} />
    </BrowserProductTheme></SurfaceContext.Provider>,
  ));
  try {
    render("native");
    await tick();
    const link = container.querySelector("a")!;
    for (const metaKey of [false, true]) {
      const click = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, metaKey });
      link.dispatchEvent(click);
      check(click.defaultPrevented, "Native primary/Command click uses the browser handoff");
    }
    check(urls.length === 2 && urls.every((value) => value === url), "Each native click opens once with its full URL");
    const context = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 40, clientY: 40 });
    link.dispatchEvent(context);
    await tick();
    check(context.defaultPrevented, "Native Desktop uses the working link menu");
    const items = [...document.querySelectorAll<HTMLElement>("[role='menuitem']")];
    check(items.some((item) => item.textContent === "Copy link"), "Link menu retains Copy link");
    const open = items.find((item) => item.textContent === "Open in browser");
    check(open, "Link menu exposes the system-browser action");
    open.click();
    await tick();
    check(Number(urls.length) === 3 && urls[2] === url, "Context-menu Open launches the exact URL once");

    link.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
    await tick();
    document.querySelector("[role='menu']")?.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
    await tick();
    check(!document.querySelector("[role='menu']"), "Escape dismisses the link menu");
    delete native.__TAURI_INTERNALS__;
    render("browser");
    await tick();
    const ordinary = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 });
    container.querySelector("a")!.dispatchEvent(ordinary);
    check(!ordinary.defaultPrevented, "Browser/PWA retains its native context menu");
    return "Native Desktop links open once for click, Command-click and context menu; Copy link and Escape remain available; Browser/PWA keeps its system menu";
  } finally {
    root.unmount();
    container.remove();
    if (previous) native.__TAURI_INTERNALS__ = previous;
    else delete native.__TAURI_INTERNALS__;
  }
}
