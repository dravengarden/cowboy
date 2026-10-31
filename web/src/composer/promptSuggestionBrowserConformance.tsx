/** The Desktop composer's predicted-prompt ghost on the real CodeMirror
 *  editor and product theme: shown only while empty, Tab fills without
 *  sending, typing or Tab dismisses it. */
import { createRef, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceContext, type SurfaceProfile } from "../surface/SurfaceProfile";
import type { ComposerEditorHandle } from "../ComposerEditor";
import { PlatformComposerEditor } from "./PlatformComposerEditor";

const DESKTOP: SurfaceProfile = {
  kind: "desktop",
  input: "keyboard-pointer",
  touchCapable: false,
  finePointer: true,
  hover: true,
};

const tick = (): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, 35));

export async function checkPromptSuggestion(): Promise<string> {
  const container = document.createElement("div");
  container.style.width = "640px";
  document.body.append(container);
  const root = createRoot(container);
  const editorRef = createRef<ComposerEditorHandle>();
  const sent: string[] = [];
  let dismissed = 0;
  // A getter: TypeScript would otherwise keep the narrowed value across awaits.
  const dismissals = (): number => dismissed;
  let offer!: (text: string | null) => void;
  function check(value: unknown, message: string): asserts value {
    if (!value) throw new Error(`prompt suggestion: ${message}`);
  }
  function Harness(): React.JSX.Element {
    const [suggestion, setSuggestion] = useState<string | null>(null);
    offer = setSuggestion;
    return (
      <PlatformComposerEditor
        ref={editorRef}
        value=""
        onChange={() => {}}
        onSubmit={() => {
          sent.push(editorRef.current?.getValue() ?? "");
          return true;
        }}
        sessionId="prompt-suggestion-fixture"
        commands={() => []}
        placeholder="Message the agent…"
        vim={false}
        borderless
        promptSuggestion={suggestion}
        onPromptSuggestionDismiss={() => {
          dismissed++;
          setSuggestion(null);
        }}
      />
    );
  }
  const tab = (): void => {
    const content = container.querySelector<HTMLElement>(".cm-content");
    check(content, "missing editor content");
    flushSync(() =>
      content.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Tab",
          code: "Tab",
          bubbles: true,
          cancelable: true,
        }),
      )
    );
  };
  const ghost = (): HTMLElement | null =>
    container.querySelector<HTMLElement>('[data-prompt-suggestion="ghost"]');
  try {
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={DESKTOP}>
          <BrowserProductTheme>
            <CssBaseline />
            <Harness />
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
    await tick();
    check(!ghost(), "no ghost without a suggestion");
    check(
      container.querySelector(".cm-placeholder"),
      "the placeholder shows while nothing is predicted",
    );

    flushSync(() => offer("Run the full test suite"));
    await tick();
    check(
      ghost()?.textContent?.startsWith("Run the full test suite"),
      "the ghost shows the suggestion",
    );
    check(
      ghost()?.getAttribute("aria-hidden") === "true",
      "the ghost is presentation only",
    );
    const placeholder = container.querySelector<HTMLElement>(".cm-placeholder");
    check(
      !placeholder || getComputedStyle(placeholder).display === "none",
      "the ghost replaces the placeholder",
    );
    check(
      editorRef.current?.getValue() === "",
      "showing a suggestion writes nothing",
    );

    editorRef.current?.focus();
    await tick();
    tab();
    await tick();
    check(
      editorRef.current?.getValue() === "Run the full test suite",
      "Tab fills the composer",
    );
    check(sent.length === 0, "Tab never sends");
    check(!ghost(), "an accepted suggestion leaves");
    check(dismissals() === 1, "accepting dismisses the held suggestion");
    tab();
    await tick();
    // Tab in a written prompt keeps its ordinary editing meaning.
    check(
      (editorRef.current?.getValue().split("Run the full test suite").length ??
        0) === 2,
      "Tab in a written prompt does not refill",
    );

    // Typing in an empty editor moves past a new suggestion.
    flushSync(() => root.render(null));
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={DESKTOP}>
          <BrowserProductTheme>
            <CssBaseline />
            <Harness />
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
    await tick();
    flushSync(() => offer("Commit the change"));
    await tick();
    check(ghost(), "a fresh editor shows a fresh suggestion");
    editorRef.current?.focus();
    editorRef.current?.insertText("x");
    await tick();
    check(
      !ghost() && editorRef.current?.getValue() === "x",
      "typing hides the ghost",
    );
    check(dismissals() === 2, "typing dismisses the held suggestion");
    check(sent.length === 0, "nothing was sent");
    return "Desktop predicted prompt is ghost text that Tab fills without sending";
  } finally {
    root.unmount();
    container.remove();
  }
}
