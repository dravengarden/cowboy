import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { ScrollEdges } from "@cowboy/app-shell";

export async function checkScrollEdges(): Promise<void> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const wait = () => new Promise(resolve => setTimeout(resolve, 60));
  const check = (value: boolean, message: string) => { if (!value) throw new Error(message); };
  try {
    flushSync(() => root.render(<div style={{ height: 120, overflowY: "auto" }}>
      <div style={{ height: 400 }}><input aria-label="Edge focus check" /></div><ScrollEdges />
    </div>));
    const owner = host.firstElementChild as HTMLElement;
    const content = owner.firstElementChild as HTMLElement;
    await wait();
    check(owner.dataset.scrollEdges === "bottom", "At the top, only undisplayed trailing content fades");
    const height = owner.scrollHeight;
    owner.scrollTop = 80;
    await wait();
    check(owner.dataset.scrollEdges === "top bottom", "Middle scroll fades both edges");
    check(owner.scrollHeight === height, "Scroll edges must not change content geometry");
    owner.scrollTop = owner.scrollHeight;
    await wait();
    check(owner.dataset.scrollEdges === "top", "Last item is fully visible at the bottom");
    owner.querySelector("input")!.focus({ preventScroll: true });
    await wait();
    check(owner.style.maskImage === "none", "Editing never fades a focused input");
    owner.querySelector("input")!.blur();
    content.style.height = "40px";
    await wait();
    check(owner.style.maskImage === "none", "Short content has no fade after resizing");
    content.style.height = "500px";
    await wait();
    check(owner.dataset.scrollEdges === "bottom", "Growing content updates overflow without a scroll event");
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    content.append(dialog);
    await wait();
    check(owner.style.maskImage === "none", "Inline dialogs escape the reader mask");
    dialog.remove();
    await wait();
    check(owner.dataset.scrollEdges === "bottom", "Closing a nested dialog restores the scroll hint");
  } finally { flushSync(() => root.unmount()); host.remove(); }
}
