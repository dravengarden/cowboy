import { createRef, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceProvider } from "../surface/SurfaceProfile";
import { desktopSize } from "../surface/desktopSize";
import {
  type ComposerEditorHandle,
  PlatformComposerEditor,
} from "../composer/PlatformComposerEditor";
import { COMPOSER_COMMANDS_BY_ID } from "../composerCommands";
import { FullscreenComposer } from "../FullscreenComposer";
import { isMac } from "../platform";
import DesktopDraftToolbar from "./DesktopDraftToolbar";
import {
  DesktopWorkspaceProvider,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import {
  DesktopCommandProvider,
  useDesktopCommands,
} from "./commands/DesktopCommandProvider";
import { DesktopPendingEditCommandBindings } from "./commands/DesktopPendingEditShortcuts";
import { clearImeStatus, setImeComposing } from "./vim/imeStatusStore";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 40));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

export async function checkDraftKeyboard(): Promise<string> {
  const originalMatchMedia = globalThis.matchMedia;
  globalThis.matchMedia = (query) => {
    if (
      query.includes("(pointer: fine)") || query.includes("(hover: hover)") ||
      query.includes("(any-pointer: coarse)")
    ) {
      return {
        media: query,
        matches: !query.includes("coarse"),
        onchange: null,
        addListener() {},
        removeListener() {},
        addEventListener() {},
        removeEventListener() {},
        dispatchEvent: () => true,
      };
    }
    return originalMatchMedia.call(globalThis, query);
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const editorRef = createRef<ComposerEditorHandle>();
  let workspace!: ReturnType<typeof useDesktopWorkspace>;
  let registry!: ReturnType<typeof useDesktopCommands>;
  const calls: string[] = [];
  const originalFont = document.documentElement.style.fontSize;
  function Harness(
    { kind }: { kind: "queued" | "draft" | "document" | "fullscreen" },
  ) {
    workspace = useDesktopWorkspace();
    registry = useDesktopCommands();
    const save = () => {
      calls.push(kind);
    };
    let content: ReactNode;
    if (kind === "fullscreen") {
      content = (
        <FullscreenComposer
          value="Fullscreen draft"
          onChange={() => {}}
          onSubmit={save}
          onSaveDraft={save}
          onCollapse={() => {}}
          onAttach={() => {}}
          onPasteFiles={() => {}}
          onPasteClipboardImages={() => {}}
          sessionId="save-fixture"
          commands={() => []}
          placeholder="Edit"
          sendable
          editorRef={editorRef}
          saveOnly
          showCollapse={false}
          submitLabel="Save changes"
        />
      );
    } else {
      content = (
        <>
          <PlatformComposerEditor
            ref={editorRef}
            value="Editable draft"
            onChange={() => {}}
            borderless
            vim={false}
          />
          {kind === "document"
            ? (
              <DesktopDraftToolbar
                fallback={null}
                toolbar={[
                  "undo",
                  "bold",
                  "italic",
                  "code",
                  "link",
                  "bulletList",
                  "sourceMode",
                ]}
                status="Saved on this device"
                readableWidth={false}
                writable
                historyLoading={false}
                onFormat={(id) =>
                  COMPOSER_COMMANDS_BY_ID[id]?.run({
                    editor: editorRef.current!,
                    attach: () => {},
                  })}
                onSave={save}
                onAttach={() => calls.push("attach")}
                onCopy={() => calls.push("copy")}
                onHistory={() => calls.push("history")}
                onRename={() => calls.push("rename")}
                onExport={() => calls.push("export")}
                onReadableWidth={() => calls.push("width")}
              />
            )
            : (
              <DesktopPendingEditCommandBindings
                kind={kind}
                sendable
                onSlash={() => calls.push("slash")}
                onReference={() => calls.push("reference")}
                onAttach={() => calls.push("pending-attach")}
                onDone={save}
                onExpand={() => calls.push("expand")}
              />
            )}
        </>
      );
    }
    return (
      <Box
        data-desktop-pane="prompt"
        data-desktop-region={kind === "document"
          ? "prompt.composer"
          : `prompt.${kind}`}
        sx={{ width: "100%" }}
      >
        <Box
          data-size-fixture
          sx={{
            fontSize: desktopSize(16),
            width: desktopSize(16),
            height: desktopSize(16),
          }}
        >
          •
        </Box>
        {content}
      </Box>
    );
  }
  const key = (
    key: string,
    code: string,
    options: Partial<KeyboardEventInit> = {},
  ) => {
    const event = new KeyboardEvent("keydown", {
      key,
      code,
      bubbles: true,
      cancelable: true,
      ...options,
    });
    flushSync(() =>
      (document.activeElement ?? document.body).dispatchEvent(event)
    );
    return event;
  };
  const mod = isMac ? { metaKey: true } : { ctrlKey: true };
  const prefix = () =>
    key("k", "KeyK", isMac ? { metaKey: true } : { altKey: true });
  try {
    for (const kind of ["queued", "draft", "document", "fullscreen"] as const) {
      flushSync(() =>
        root.render(
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <DesktopWorkspaceProvider>
                <DesktopCommandProvider>
                  <Harness key={kind} kind={kind} />
                </DesktopCommandProvider>
              </DesktopWorkspaceProvider>
            </BrowserProductTheme>
          </SurfaceProvider>,
        )
      );
      await tick();
      check(
        (kind === "fullscreen" ? document : container).querySelector(
          ".cm-editor",
        ),
        "Keyboard acceptance uses the actual Desktop CodeMirror branch",
      );
      editorRef.current!.focus();
      flushSync(() =>
        workspace.focusRegion(
          kind === "document" ? "prompt.composer" : `prompt.${kind}`,
        )
      );
      const before = calls.length;
      key("Enter", "Enter", mod);
      await tick();
      check(
        calls.length === before,
        `${kind}: Mod+Enter must neither save nor send`,
      );
      key("s", "KeyS", { ...mod, isComposing: true });
      check(calls.length === before, `${kind}: IME Save is fenced`);
      setImeComposing(false);
      key("s", "KeyS", mod);
      check(
        calls.length === before,
        `${kind}: shared composition prevents Save even without a keydown marker`,
      );
      clearImeStatus();
      key("s", "KeyS", mod);
      check(
        calls.at(-1) === kind && calls.length === before + 1,
        `${kind}: Mod+S saves exactly once`,
      );
      key("s", "KeyS", { ...mod, repeat: true });
      check(calls.length === before + 1, `${kind}: held Save does not repeat`);
      if (kind === "queued" || kind === "draft") {
        // The row editor toolbar: every button has a leader key in its scope.
        for (
          const [letter, code, call] of [
            ["/", "Slash", "slash"],
            ["f", "KeyF", "reference"],
            ["a", "KeyA", "pending-attach"],
            ["z", "KeyZ", "expand"],
          ] as const
        ) {
          prefix();
          key(letter, code);
          check(calls.at(-1) === call, `${kind}: ␣${letter} runs ${call}`);
        }
      }
      if (kind === "document") {
        check(
          registry.list().some((c) => c.id === "document.export") &&
            registry.list().some((c) => c.id === "composer.format.bold"),
          "Document and formatting actions are searchable commands",
        );
        setImeComposing(false);
        prefix();
        key("a", "KeyA");
        check(calls.at(-1) === kind, "Composition prevents prefix actions");
        clearImeStatus();
        prefix();
        key("a", "KeyA");
        check(
          calls.at(-1) === "attach",
          "Draft uses the existing attachment prefix",
        );
        // The open document's own `␣D` group: rename, copy, history,
        // export, readable width.
        for (
          const [letter, code, call] of [
            ["r", "KeyR", "rename"],
            ["v", "KeyV", "copy"],
            ["h", "KeyH", "history"],
            ["e", "KeyE", "export"],
          ] as const
        ) {
          prefix();
          key("d", "KeyD");
          key(letter, code);
          check(calls.at(-1) === call, `␣D${letter.toUpperCase()} runs ${call}`);
        }
        prefix();
        key("m", "KeyM");
        key("m", "KeyM");
        await tick();
        check(
          document.querySelector('[role="menu"]'),
          "More prefix opens the complete formatting menu",
        );
        check(
          document.querySelector('[role="menu"]')!.textContent?.includes(
            "Toggle heading",
          ),
          "Narrow toolbars retain less frequent formatting in More",
        );
        key("Escape", "Escape");
        for (
          let attempt = 0;
          attempt < 10 && document.querySelector('[role="menu"]');
          attempt++
        ) await tick();
        check(
          !document.querySelector('[role="menu"]'),
          "Closing More releases its exclusive keyboard scope",
        );
        editorRef.current!.focus();
        prefix();
        key("m", "KeyM");
        key("b", "KeyB");
        check(
          editorRef.current!.getValue().includes("**"),
          "Formatting prefix reaches the actual editor",
        );
        for (const font of [8, 16, 24, 32]) {
          document.documentElement.style.fontSize = `${font}px`;
          for (const width of [320, 600, 960]) {
            container.style.width = `${width}px`;
            await tick();
            const toolbar = container.querySelector<HTMLElement>(
              "[data-desktop-draft-toolbar]",
            )!;
            check(
              toolbar.scrollWidth <= toolbar.clientWidth + 1,
              `Draft toolbar fits ${width}px at ${font}px`,
            );
            const icon = [...toolbar.querySelectorAll<SVGElement>("svg")].find(
              (svg) => svg.getClientRects().length > 0,
            )!;
            check(
              Math.abs(icon.getBoundingClientRect().width - 1.25 * font) < 1,
              "Draft icon tracks global root font",
            );
            const sample = container.querySelector<HTMLElement>(
              "[data-size-fixture]",
            )!;
            check(
              Math.abs(sample.getBoundingClientRect().width - font) < 1,
              "Shared fixed-size glyph tracks Desktop font",
            );
          }
        }
      }
      flushSync(() => root.render(null));
    }
    globalThis.matchMedia = (query) => {
      if (
        query.includes("(pointer: fine)") || query.includes("(hover: hover)") ||
        query.includes("(any-pointer: coarse)")
      ) {
        return {
          media: query,
          matches: query.includes("coarse"),
          onchange: null,
          addListener() {},
          removeListener() {},
          addEventListener() {},
          removeEventListener() {},
          dispatchEvent: () => true,
        };
      }
      return originalMatchMedia.call(globalThis, query);
    };
    flushSync(() =>
      root.render(
        <SurfaceProvider>
          <Box
            data-touch-size
            sx={{
              width: desktopSize(18),
              height: desktopSize(18),
              fontSize: desktopSize(18),
            }}
          >
            •
          </Box>
        </SurfaceProvider>,
      )
    );
    document.documentElement.style.fontSize = "24px";
    check(
      container.querySelector<HTMLElement>("[data-touch-size]")!
        .getBoundingClientRect().width === 18,
      "Touch keeps original fixed-size glyph geometry at enlarged root fonts",
    );
    return "Desktop Queue/Draft inline and expanded edits save only with Mod+S; repeats/IME fenced; independent Draft uses scoped formatting/attachment/copy/history commands and wraps at 320–960px across 8–32px fonts";
  } finally {
    clearImeStatus();
    document.documentElement.style.fontSize = originalFont;
    root.unmount();
    container.remove();
    globalThis.matchMedia = originalMatchMedia;
  }
}
