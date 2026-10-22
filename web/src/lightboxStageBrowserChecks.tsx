/** Real ImageLightbox: a figure fits and pans only inside the unobstructed
 *  stage, so its top never rests under the status bar / Dynamic Island and its
 *  bottom never rests under the control dock. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { ImageLightbox } from "@cowboy/app-shell";
import { BrowserProductTheme } from "./browserProductTheme";

const TALL_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 2000" width="200" height="2000">' +
  '<rect width="200" height="2000" fill="#ddd"/></svg>';

export async function runLightboxStageBrowserChecks(): Promise<void> {
  const check = (ok: unknown, message: string): void => {
    if (!ok) throw new Error(`lightbox stage: ${message}`);
  };
  const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  const key = (value: string, times = 1) => {
    for (let i = 0; i < times; i++) {
      globalThis.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true }));
    }
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <ImageLightbox
            images={[{ kind: "inline-svg", markup: TALL_SVG, alt: "Mermaid diagram", themed: true }]}
            index={0}
            onIndex={() => {}}
            onClose={() => {}}
          />
        </BrowserProductTheme>,
      )
    );
    await wait(300);
    const overlay = document.querySelector<HTMLElement>('[role="dialog"][aria-label="Image preview"]');
    const dock = document.querySelector<HTMLElement>("[data-lightbox-dock]");
    const svg = overlay?.querySelector<SVGSVGElement>("svg[aria-label='Mermaid diagram']");
    check(overlay && dock && svg, "renders the stage, dock and figure");
    const stageTop = Number.parseFloat(getComputedStyle(overlay!).paddingTop);
    const dockTop = () => dock!.getBoundingClientRect().top;
    const figure = () => svg!.getBoundingClientRect();
    check(stageTop >= 12, "the stage keeps a top inset");
    check(
      figure().top >= stageTop - 1 && figure().bottom <= dockTop() + 1,
      "a fitted tall figure sits between the top inset and the dock",
    );
    key("+", 3);
    await wait(400);
    check(figure().height > overlay!.clientHeight, "the figure zooms past the viewport");
    key("k", 60);
    await wait(400);
    check(
      Math.abs(figure().top - stageTop) <= 1,
      `panning to the top stops at the inset (top ${figure().top}, inset ${stageTop})`,
    );
    key("j", 120);
    await wait(400);
    check(
      figure().bottom <= dockTop() - 4 && figure().bottom >= dockTop() - 12,
      `panning to the bottom stops above the dock (bottom ${figure().bottom}, dock ${dockTop()})`,
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
}
