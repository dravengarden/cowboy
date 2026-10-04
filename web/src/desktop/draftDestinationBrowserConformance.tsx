import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { DraftDestinationDialog } from "./DesktopDraftDestinationPicker";
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
  const button = (selector: string) =>
    document.querySelector<HTMLElement>(selector);
  const search = (value: string) => {
    const input = document.querySelector<HTMLInputElement>(
      "input[aria-label='Search draft destinations']",
    )!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
      .call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <DraftDestinationDialog
            sessions={sessions}
            folders={folders}
            sourceId="current"
            onPick={(s) => picks.push(s.id)}
            onClose={() => cancels++}
          />
        </BrowserProductTheme>,
      )
    );
    await tick();
    check(
      !button("[data-draft-session='current']"),
      "Current session is excluded",
    );
    check(
      button("[data-draft-session='bound']") &&
        button("[data-draft-session='global']"),
      "Project bindings and explicit Global placements agree with Sessions",
    );
    check(
      button("[data-draft-session='nested']")?.textContent?.includes("cowboy"),
      "Stable project context replaces generated cwd",
    );
    button("[data-draft-folder='work']")!.click();
    await tick();
    check(
      !button("[data-draft-session='nested']") &&
        button("[data-draft-session='global']"),
      "Folder collapse does not conceal Global sessions",
    );
    search("Nested");
    await tick();
    check(
      button("[data-draft-session='nested']") &&
        !button("[data-draft-session='bound']"),
      "Folder search reveals ancestors and filters destinations",
    );
    const input = document.querySelector<HTMLInputElement>(
      "input[aria-label='Search draft destinations']",
    )!;
    input.focus();
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowDown",
        bubbles: true,
        isComposing: true,
      }),
    );
    check(document.activeElement === input, "IME owns candidate arrows");
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        isComposing: true,
      }),
    );
    check(Number(cancels) === 0, "IME Escape does not dismiss the picker");
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
    );
    check(
      document.activeElement === button("[data-draft-session='nested']"),
      "ArrowDown skips search-only folder headings",
    );
    button("[data-draft-session='nested']")!.click();
    check(
      picks.join() === "nested",
      "Duplicate titles choose by immutable session id",
    );
    search("no such destination");
    await tick();
    check(
      document.querySelector("[role='status']")?.textContent?.includes(
        "No matching",
      ),
      "Empty search has explicit feedback",
    );
    check(picks.length === 1, "Search never delivers a draft");
    const cancel = Array.from(
      document.querySelectorAll<HTMLElement>("[role='dialog'] button"),
    ).find((b) => b.textContent === "Cancel")!;
    cancel.click();
    check(
      cancels === 1 && picks.length === 1,
      "Cancel does not select or move",
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
          const list = document.querySelector<HTMLElement>(
            "[aria-label='Draft destination sessions']",
          )!;
          check(
            list.scrollWidth <= list.clientWidth + 1,
            "Destination list fits narrow dialogs and enlarged fonts",
          );
          check(
            !list.textContent?.includes("cowboy-machine/worktrees"),
            "Generated execution paths do not crowd destination rows",
          );
        }
      }
    } finally {
      document.documentElement.style.fontSize = originalFont;
    }
    return "Desktop draft destinations share Sessions folders/order/Global placement, stable project context, search, IME-safe keyboard focus and id-based selection";
  } finally {
    root.unmount();
    container.remove();
  }
}
