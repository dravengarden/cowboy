import { checkInputVim } from "./desktop/inputVimBrowserConformance";
import { checkDraftKeyboard } from "./desktop/draftKeyboardBrowserConformance";
import { checkDraftDestinationDialog } from "./desktop/draftDestinationBrowserConformance";
import { checkPendingPanelLayout } from "./pendingPanelBrowserConformance";
import { createRef, StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import type { ComposerEditorHandle } from "./ComposerEditor";
import { PlatformComposerEditor } from "./composer/PlatformComposerEditor";
import {
  DesktopComposerToolbar,
  type DesktopComposerToolbarProps,
} from "./desktop/DesktopComposerToolbar";
import {
  DesktopWorkspaceProvider,
  useDesktopWorkspace,
} from "./desktop/DesktopWorkspaceController";
import {
  DesktopCommandProvider,
  useDesktopCommands,
} from "./desktop/commands/DesktopCommandProvider";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_WORKSPACE_KEYS,
} from "./desktop/commands/workspaceShortcuts";
import { clearImeStatus, setImeComposing } from "./desktop/vim/imeStatusStore";
import { isMac } from "./platform";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 35));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

/** Real toolbar, CodeMirror, command dispatcher and product theme; no account. */
export async function runDesktopComposerBrowserConformance(): Promise<
  string[]
> {
  const container = document.createElement("div");
  container.style.width = "960px";
  document.body.append(container);
  const root = createRoot(container);
  const editorRef = createRef<ComposerEditorHandle>();
  const sendButtonRef = createRef<HTMLButtonElement>();
  const calls: string[] = [];
  let setOptions!: (next: Partial<DesktopComposerToolbarProps>) => void;
  let workspace!: ReturnType<typeof useDesktopWorkspace>;
  let commands!: ReturnType<typeof useDesktopCommands>;
  function Harness() {
    workspace = useDesktopWorkspace();
    commands = useDesktopCommands();
    const [options, update] = useState<Partial<DesktopComposerToolbarProps>>(
      {},
    );
    setOptions = update;
    return (
      <Box
        data-desktop-pane="prompt"
        sx={{
          display: "flex",
          flexDirection: "column",
          height: 650,
          minHeight: 0,
        }}
      >
        <Box
          sx={{
            px: 2,
            py: 1,
            borderBottom: 1,
            borderColor: "divider",
            fontSize: "0.7rem",
            letterSpacing: ".15em",
            color: "text.secondary",
          }}
        >
          PROMPT
        </Box>
        <Box
          data-desktop-region="prompt.composer"
          tabIndex={-1}
          sx={{
            display: "flex",
            flexDirection: "column",
            flex: 1,
            minHeight: 0,
          }}
        >
          <PlatformComposerEditor
            ref={editorRef}
            value="A prompt worth writing"
            onChange={() => {}}
            onSubmit={() => {
              calls.push("send");
              return true;
            }}
            onSaveDraft={() => {
              calls.push("draft");
            }}
            sessionId="composer-fixture"
            commands={() => []}
            vim={false}
            fill
            borderless
            endInset={0}
          />
          <DesktopComposerToolbar
            editorRef={editorRef}
            sendButtonRef={sendButtonRef}
            canInsert
            sendable
            canJumpFront
            canForce
            pending={false}
            progress={false}
            sendLabel="Send"
            sendDescription="Send message"
            unavailableReason="Write a message first"
            overlayOpen={false}
            onAttach={() => {
              calls.push("attach");
            }}
            onSaveDraft={() => {
              calls.push("draft");
            }}
            onSchedule={() => {
              calls.push("schedule");
            }}
            onJumpFront={() => {
              calls.push("next");
            }}
            onForce={() => {
              calls.push("force");
            }}
            onSubmit={() => {
              calls.push("send");
            }}
            {...options}
          />
        </Box>
        <button
          type="button"
          data-desktop-pane="conversation"
          data-desktop-region="conversation.transcript"
        >
          Conversation
        </button>
      </Box>
    );
  }
  const button = (id: string): HTMLButtonElement => {
    const element = container.querySelector<HTMLButtonElement>(
      `[data-composer-action="${id}"]`,
    );
    check(element, `Missing ${id}`);
    return element;
  };
  const key = (
    key: string,
    code: string,
    modifiers: Partial<KeyboardEventInit> = {},
  ): void => {
    flushSync(() =>
      (document.activeElement ?? globalThis).dispatchEvent(
        new KeyboardEvent("keydown", {
          key,
          code,
          bubbles: true,
          cancelable: true,
          ...modifiers,
        }),
      )
    );
  };
  const prefix = (): void =>
    key("k", "KeyK", isMac ? { metaKey: true } : { altKey: true });
  // A leader path: one key at the root (`A`) or a group and its key (`MB`).
  const sequence = (path: string): void => {
    prefix();
    for (const letter of path) {
      key(
        letter.toLowerCase(),
        letter === "/" ? "Slash" : `Key${letter.toUpperCase()}`,
      );
    }
  };
  const click = (id: string): void => flushSync(() => button(id).click());
  const results: string[] = [];
  const originalFont = document.documentElement.style.fontSize;
  try {
    flushSync(() =>
      root.render(
        <StrictMode>
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <DesktopWorkspaceProvider>
                <DesktopCommandProvider>
                  <Harness />
                </DesktopCommandProvider>
              </DesktopWorkspaceProvider>
            </BrowserProductTheme>
          </SurfaceProvider>
        </StrictMode>,
      )
    );
    await tick();
    check(editorRef.current, "Real editor mounted");
    flushSync(() => workspace.focusRegion("prompt.composer"));
    editorRef.current.focusSelection({ anchor: 2, head: 8 });
    const editorNode = container.querySelector(".cm-content");
    const selection = editorRef.current.getSelection();
    for (const font of [16, 22]) {
      document.documentElement.style.fontSize = `${font}px`;
      for (const width of [320, 420, 540, 600, 960, 1560]) {
        container.style.width = `${width}px`;
        await tick();
        const toolbar = container.querySelector<HTMLElement>(
          "[data-desktop-composer-toolbar]",
        )!;
        const bounds = toolbar.getBoundingClientRect();
        check(
          toolbar.scrollWidth <= toolbar.clientWidth + 1,
          `No toolbar overflow at ${width}/${font}`,
        );
        const visible = [
          ...toolbar.querySelectorAll<HTMLButtonElement>("button"),
        ].filter((item) => item.getClientRects().length > 0);
        for (const item of visible) {
          const rect = item.getBoundingClientRect();
          check(
            rect.left >= bounds.left && rect.right <= bounds.right + 1,
            `${item.ariaLabel} inside ${width}/${font}`,
          );
        }
        for (
          const id of [
            "slash",
            "reference",
            "attach",
            "draft",
            "schedule",
            "next",
            "force",
            "send",
            "source",
            "more",
          ]
        ) {
          check(
            button(id).getClientRects().length > 0,
            `${id} remains visible at ${width}/${font}`,
          );
        }
        check(
          button("send").textContent?.includes("Send"),
          "Send always retains a label",
        );
        check(
          button("send").getBoundingClientRect().right >= bounds.right - 10,
          "Send remains at right edge",
        );
        check(
          container.querySelector(".cm-content") === editorNode,
          "Resize retains editor DOM",
        );
        check(
          JSON.stringify(editorRef.current.getSelection()) ===
            JSON.stringify(selection),
          "Resize retains selection",
        );
      }
    }
    document.documentElement.style.fontSize = originalFont;
    container.style.width = "960px";
    await tick();
    results.push(
      "320–1560px panes and enlarged text retain visible delivery controls, right-edge Send, editor and selection",
    );

    for (
      const [letter, call] of [["A", "attach"], ["H", "schedule"], [
        "J",
        "next",
      ]]
    ) {
      sequence(letter!);
      check(calls.at(-1) === call, `${letter} dispatches ${call}`);
    }
    key("s", "KeyS", isMac ? { metaKey: true } : { ctrlKey: true });
    check(calls.at(-1) === "draft", "Native save chord saves draft");
    key("Enter", "Enter", { altKey: true });
    check(
      calls.at(-1) === "force",
      "Force shortcut opens confirmation callback",
    );
    // Space leader (FOCUS.md "Leader"): armed only where Cowboy owns the
    // key, lights every live slot, and leaves typed spaces alone.
    {
      const region = container.querySelector<HTMLElement>(
        "[data-desktop-region='prompt.composer']",
      )!;
      const space = (target: Element, init: KeyboardEventInit = {}) => {
        const event = new KeyboardEvent("keydown", {
          key: " ",
          code: "Space",
          bubbles: true,
          cancelable: true,
          ...init,
        });
        flushSync(() => target.dispatchEvent(event));
        return event;
      };
      const typed = space(
        container.querySelector("[contenteditable=true], textarea")!,
      );
      check(
        !typed.defaultPrevented && workspace.mode === "normal",
        "Space in a text editor stays text",
      );
      const composing = space(region, { isComposing: true });
      check(
        !composing.defaultPrevented && workspace.mode === "normal",
        "Space confirming an IME candidate stays with the IME",
      );
      const held = space(region, { repeat: true });
      check(!held.defaultPrevented, "Auto-repeated Space never arms");
      const armed = space(region);
      check(
        armed.defaultPrevented && (workspace.mode as string) === "command",
        "Space on Cowboy-owned focus arms the leader",
      );
      const up = new KeyboardEvent("keyup", {
        key: " ",
        code: "Space",
        bubbles: true,
        cancelable: true,
      });
      region.dispatchEvent(up);
      check(up.defaultPrevented, "The leader Space never activates a button on keyup");
      await tick();
      check(
        button("attach").querySelector("[data-shortcut-state='active']") &&
          button("attach").textContent?.includes("␣A"),
        "Armed leader lights the Attach slot as one ␣A keycap",
      );
      flushSync(() =>
        region.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "a",
            code: "KeyA",
            bubbles: true,
            cancelable: true,
          }),
        )
      );
      check(
        calls.at(-1) === "attach" && workspace.mode === "normal",
        "␣A runs Attach and closes the leader",
      );
      space(region);
      flushSync(() =>
        region.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "Escape",
            code: "Escape",
            bubbles: true,
            cancelable: true,
          }),
        )
      );
      check(workspace.mode === "normal", "Esc closes the leader");
      space(region);
      flushSync(() =>
        document.body.dispatchEvent(
          new PointerEvent("pointerdown", { bubbles: true }),
        )
      );
      check(workspace.mode === "normal", "A pointer press closes the leader");
      results.push(
        "Space leader arms only on Cowboy-owned focus (not text, IME or repeat), swallows its keyup, lights ␣ slots, runs ␣A and closes on Esc/pointer",
      );
    }
    for (const id of ["attach", "draft", "schedule", "next", "force", "send"]) {
      click(id);
      check(calls.at(-1) === id, `${id} click shares action`);
    }
    // Rich text is direct chords, never the leader.
    for (const [id, chord] of Object.entries(DESKTOP_COMPOSER_FORMAT_CHORDS)) {
      check(
        commands.list().some((command) =>
          command.id === `composer.format.${id}` &&
          command.shortcut === chord && !command.sequence
        ),
        `${id} is searchable with its chord`,
      );
    }
    {
      // Obsidian's direct chords run the same format commands as ␣B / ␣I.
      const editor = editorRef.current!;
      if (editor.getValue().trim() === "") editor.insertText("word");
      const mod = isMac ? { metaKey: true } : { ctrlKey: true };
      const original = editor.getValue();
      const end = original.search(/\s|$/);
      editor.focusSelection({ anchor: 0, head: end });
      key("b", "KeyB", mod);
      await tick();
      check(
        editor.getValue() === `**${original.slice(0, end)}**${original.slice(end)}`,
        `Mod+B toggles bold: ${JSON.stringify(editor.getValue())}`,
      );
      const bold = editor.getValue();
      editor.focusSelection(editor.getSelection());
      key("i", "KeyI", mod);
      await tick();
      check(editor.getValue() !== bold && editor.getValue().includes("*"), "Mod+I toggles italic");
      check(
        commands.list().some((command) =>
          command.id === "composer.format.bold" && command.shortcut === "Mod+B"
        ),
        "Bold advertises Mod+B in the palette",
      );
      editor.insertText(original, { anchor: 0, head: editor.getValue().length });
      await tick();
    }
    results.push(
      "Buttons, native save/force chords, prefix actions and command palette share callbacks; Mod+B/Mod+I format in the editor",
    );

    flushSync(() =>
      setOptions({
        sendable: false,
        canInsert: false,
        canForce: false,
        canJumpFront: false,
      })
    );
    const before = calls.length;
    for (const letter of ["A", "H", "J"]) sequence(letter);
    key("s", "KeyS", isMac ? { metaKey: true } : { ctrlKey: true });
    key("Enter", "Enter", { altKey: true });
    check(
      calls.length === before,
      "Unavailable actions never execute from keys",
    );
    for (
      const id of [
        "slash",
        "reference",
        "attach",
        "draft",
        "schedule",
        "next",
        "force",
        "send",
      ]
    ) check(button(id).disabled, `${id} disabled truthfully`);
    for (
      const options of [
        { pending: true, progress: true },
        { sendLabel: "Queue" as const, canForce: false },
        { sendLabel: "Queue" as const, canForce: true },
        { sendLabel: "Send" as const },
      ]
    ) {
      flushSync(() => setOptions(options));
      check(
        button("send").disabled === Boolean(options.pending),
        "Pending submit blocks repeat click",
      );
      check(
        button("send").textContent?.includes(options.sendLabel ?? "Send"),
        "State has truthful Send/Queue label",
      );
    }
    results.push(
      "Empty, unavailable, preparing/queued, paused and pending states keep stable controls and disabled bindings",
    );

    flushSync(() => setOptions({}));
    editorRef.current.focusSelection({ anchor: 2, head: 8 });
    key("b", "KeyB", isMac ? { metaKey: true } : { ctrlKey: true });
    check(
      editorRef.current.getValue() === "A **prompt** worth writing",
      "Mod+B bolds the current selection",
    );
    flushSync(() => {
      commands.list().find((command) => command.id === "composer.format.undo")
        ?.run();
    });
    editorRef.current.focusSelection({ anchor: 2, head: 8 });
    key("x", "KeyX", { ...(isMac ? { metaKey: true } : { ctrlKey: true }), shiftKey: true });
    check(
      editorRef.current.getValue() === "A ~~prompt~~ worth writing",
      `Mod+Shift+X strikes through (got ${editorRef.current.getValue()})`,
    );
    flushSync(() => {
      commands.list().find((command) => command.id === "composer.format.undo")
        ?.run();
    });
    editorRef.current.focusSelection({ anchor: 2, head: 8 });
    key("b", "KeyB", isMac ? { metaKey: true } : { ctrlKey: true });
    // Undo stays with the editor (Mod+Z / Vim u) and the toolbar command; the
    // leader keeps Z for zoom.
    flushSync(() => {
      commands.list().find((command) => command.id === "composer.format.undo")
        ?.run();
    });
    check(
      editorRef.current.getValue() === "A prompt worth writing",
      "Toolbar formatting is undoable",
    );
    const originalText = editorRef.current.getValue();
    const originalSelection = editorRef.current.getSelection();
    sequence(DESKTOP_WORKSPACE_KEYS.toggleSourceMode);
    await tick();
    check(
      editorRef.current.getValue() === originalText,
      "Source mode preserves document",
    );
    check(
      JSON.stringify(editorRef.current.getSelection()) ===
        JSON.stringify(originalSelection),
      "Source mode preserves selection",
    );
    check(
      button("source").getAttribute("aria-pressed") === "true",
      "Source mode has toggle semantics",
    );
    sequence(DESKTOP_WORKSPACE_KEYS.toggleSourceMode);
    results.push(
      "Formatting and Undo edit the real selection; Source mode preserves text, caret and editor",
    );

    prefix();
    check(
      button("attach").querySelector("[data-shortcut-state='active']"),
      "Armed leader lights the continuation slot",
    );
    key("Escape", "Escape");
    check(
      button("attach").querySelector("[data-shortcut-state='available']"),
      "At rest the in-scope ␣A slot is available, not lit",
    );
    flushSync(() => workspace.focusRegion("conversation.transcript"));
    const outside = calls.length;
    sequence("A");
    check(
      calls.length === outside,
      "Composer action cannot run in Conversation",
    );
    check(
      button("draft").querySelector("[data-shortcut-state='inactive']"),
      "Draft hint follows focus owner",
    );
    flushSync(() => workspace.focusRegion("prompt.composer"));
    flushSync(() => setImeComposing(false));
    prefix();
    key("a", "KeyA", { isComposing: true });
    check(calls.length === outside, "Composition owns keys");
    check(
      button("source").disabled && button("bold").disabled,
      "No source/formatting reconfigure during IME",
    );
    flushSync(() => clearImeStatus());
    results.push(
      "Shortcut slots track prefix, focus and IME ownership without consuming bare editor letters",
    );

    sequence(DESKTOP_WORKSPACE_KEYS.composerMore);
    await tick();
    check(
      document.querySelector("[role='menu']"),
      "More formatting opens by shortcut",
    );
    check(
      button("draft").querySelector("[data-shortcut-state='inactive']"),
      "Menu owns shortcuts",
    );
    const menuCalls = calls.length;
    sequence("A");
    check(
      calls.length === menuCalls,
      "Exclusive menu blocks underlying actions",
    );
    key("Escape", "Escape");
    for (
      let attempt = 0;
      attempt < 60 && document.querySelector("[role='menu']");
      attempt++
    ) {
      await tick();
    }
    check(
      !document.querySelector("[role='menu']"),
      "Escape closes only the menu",
    );
    check(
      container.querySelector(".cm-content") === editorNode,
      "Menu retained editor instance",
    );
    results.push(
      "Formatting menu and Escape respect exclusive shortcut scope and preserve the editor",
    );
    results.push(await checkPendingPanelLayout());
    results.push(await checkDraftDestinationDialog());
    flushSync(() => root.render(null));
    results.push(await checkDraftKeyboard());
    results.push(await checkInputVim());
    return results;
  } finally {
    clearImeStatus();
    document.documentElement.style.fontSize = originalFont;
    if (new URL(location.href).searchParams.has("preview")) {
      container.style.width =
        new URL(location.href).searchParams.get("width") ?? "600px";
    } else {
      root.unmount();
      container.remove();
    }
  }
}
