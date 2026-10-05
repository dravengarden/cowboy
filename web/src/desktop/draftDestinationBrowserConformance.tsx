import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { DraftDestinationModal } from "../DraftDestinationPicker";
import { SurfaceContext } from "../surface/SurfaceProfile";
import type { SessionMeta } from "../protocol";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 70));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
export async function checkDraftDestinationDialog(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const picks: string[] = [];
  let cancels = 0;
  const sessions: SessionMeta[] = ["current", "bound", "nested", "global"].map((
    id,
  ) => ({
    id,
    title: id === "bound" || id === "nested" ? "Same title" : id,
    cwd: `/tmp/cowboy-machine/worktrees/${id}`,
    provider: "codex",
    status: "running",
    workspace_name: "cowboy",
  }));
  const folders = {
    folders: [
      {
        id: "work",
        name: "Work",
        parent: null,
        position: 0,
        project: "cowboy",
      },
      {
        id: "child",
        name: "Nested",
        parent: "work",
        position: 0,
        project: null,
      },
    ],
    placement: { nested: "child", global: "" },
  };
  const row = (selector: string) =>
    document.querySelector<HTMLElement>(selector);
  const session = (id: string) =>
    row(`[data-session-destination-session='${id}']`);
  const folder = (id: string) =>
    row(`[data-session-destination-folder='${id}']`);
  const input = () =>
    document.querySelector<HTMLInputElement>(
      "input[aria-label='Search destination Sessions']",
    )!;
  const search = (value: string) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
      .call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  };
  const press = (
    target: Element,
    key: string,
    init: KeyboardEventInit = {},
  ) =>
    target.dispatchEvent(
      new KeyboardEvent("keydown", {
        key,
        code: /^[a-z]$/i.test(key)
          ? `Key${key.toUpperCase()}`
          : key === "/"
          ? "Slash"
          : key,
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  try {
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider
          value={{
            kind: "desktop",
            input: "pointer",
            touchCapable: false,
            finePointer: true,
            hover: true,
          } as never}
        >
          <BrowserProductTheme>
            <CssBaseline />
            <DraftDestinationModal
              title="Move draft to…"
              sessions={sessions}
              folders={folders}
              order={[]}
              sourceId="current"
              onPick={(s) => picks.push(s.id)}
              onClose={() => cancels++}
            />
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
    await tick();
    await tick();
    check(
      document.activeElement === input(),
      "Desktop opens in search (Insert)",
    );
    check(
      session("current")?.getAttribute("aria-disabled") === "true" &&
        session("current")?.getAttribute("aria-current") === "true",
      "The source Session stays visible as the current, unselectable row",
    );
    check(
      folder("work")?.getAttribute("aria-expanded") === "true" &&
        folder("child")?.getAttribute("aria-expanded") === "false",
      "The current Session's folder path opens; other folders start folded",
    );
    check(
      session("bound") && session("global"),
      "Project bindings and explicit Global placements agree with Sessions",
    );
    check(
      session("bound")?.textContent?.includes("cowboy") &&
        !document.querySelector("[role='tree']")?.textContent?.includes(
          "cowboy-machine/worktrees",
        ),
      "Rows show the stable project, never generated execution paths",
    );
    const guide = getComputedStyle(folder("child")!, "::before");
    check(
      guide.backgroundImage.includes("gradient"),
      "Nested rows draw the sidebar's ancestor guides",
    );
    check(
      document.querySelector("[data-desktop-shortcut-bar]")?.textContent
        ?.includes("Fold"),
      "The modal shortcut bar is a live legend for tree motions",
    );
    press(input(), "ArrowDown", { isComposing: true });
    check(document.activeElement === input(), "IME owns candidate arrows");
    press(input(), "Escape", { isComposing: true });
    check(cancels === 0, "IME Escape does not dismiss the picker");
    press(input(), "Escape");
    await tick();
    check(
      cancels === 0 &&
        document.activeElement?.hasAttribute("data-session-destination-row"),
      "Esc leaves search for the tree instead of closing",
    );
    press(document.activeElement!, "G", { shiftKey: true });
    check(
      document.activeElement === session("global"),
      "G jumps to the last destination",
    );
    press(document.activeElement!, "g");
    await tick();
    press(document.activeElement!, "g");
    await tick();
    check(
      document.activeElement === folder("work"),
      "gg jumps to the first destination",
    );
    press(document.activeElement!, "j");
    check(
      document.activeElement === folder("child"),
      "j skips the current Session",
    );
    press(document.activeElement!, "l");
    await tick();
    check(
      folder("child")?.getAttribute("aria-expanded") === "true" &&
        session("nested"),
      "l expands a folded folder",
    );
    folder("child")!.focus();
    press(folder("child")!, "l");
    check(
      document.activeElement === session("nested"),
      "l on an open folder enters it",
    );
    press(session("nested")!, "h");
    check(
      document.activeElement === folder("child"),
      "h moves from a Session to its folder",
    );
    press(folder("child")!, "h");
    await tick();
    check(!session("nested"), "h collapses an open folder");
    press(folder("child")!, "/");
    check(document.activeElement === input(), "/ returns to search");
    search("Nested");
    await tick();
    check(
      session("nested") && !session("bound"),
      "Folder search reveals ancestors and filters destinations",
    );
    press(input(), "Enter");
    check(
      document.activeElement === session("nested"),
      "Enter in search reaches the first matching Session",
    );
    press(session("nested")!, "l");
    check(
      picks.join() === "nested",
      "l chooses; duplicate titles choose by immutable session id",
    );
    search("current");
    await tick();
    session("current")?.click();
    check(picks.length === 1, "The current Session is never a destination");
    search("no such destination");
    await tick();
    check(
      document.querySelector("[role='status']")?.textContent?.includes(
        "No matching",
      ),
      "Empty search has explicit feedback",
    );
    const paper = document.querySelector<HTMLElement>(".MuiDialog-paper")!;
    const originalFont = document.documentElement.style.fontSize;
    try {
      for (const font of [16, 24]) {
        document.documentElement.style.fontSize = `${font}px`;
        for (const width of [320, 640]) {
          paper.style.width = `${width}px`;
          search("");
          await tick();
          const tree = document.querySelector<HTMLElement>("[role='tree']")!;
          check(
            tree.scrollWidth <= tree.clientWidth + 1,
            "Destination tree fits narrow dialogs and enlarged fonts",
          );
        }
      }
    } finally {
      document.documentElement.style.fontSize = originalFont;
    }
    folder("work")!.focus();
    press(folder("work")!, "Escape");
    await tick();
    check(
      Number(cancels) === 1 && picks.length === 1,
      "Esc from the tree closes",
    );
    return "Desktop draft destinations are the Sessions tree: current Session in context, sidebar guides, stable project context, search Insert/Esc layering, J/K/H/L/gg/G and / motions, IME guards and id-based selection";
  } finally {
    root.unmount();
    container.remove();
  }
}
