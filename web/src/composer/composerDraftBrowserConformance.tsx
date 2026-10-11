import { Suspense, use, useRef } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { imageTokensInText } from "../attachments";
import { BrowserProductTheme } from "../browserProductTheme";
import { getDraft, setDraft } from "../draftStore";
import { SurfaceContext } from "../surface/SurfaceProfile";
import {
  type ComposerEditorHandle,
  PlatformComposerEditor,
} from "./PlatformComposerEditor";
import {
  type ComposerDraftController,
  useComposerDraftController,
} from "./useComposerDraftController";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

async function until(done: () => boolean, message: string): Promise<void> {
  const deadline = Date.now() + 8_000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error(message);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function screenshot(): Promise<File> {
  const canvas = document.createElement("canvas");
  canvas.width = 640;
  canvas.height = 400;
  const context = canvas.getContext("2d");
  check(context, "No 2D canvas context");
  context.fillStyle = "#3566d6";
  context.fillRect(0, 0, 640, 400);
  const blob = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, "image/png")
  );
  check(blob, "The fixture image did not encode");
  return new File([blob], "shot.png", { type: "image/png" });
}

interface Live {
  editor: ComposerEditorHandle | null;
  controller: ComposerDraftController | null;
}

/** Suspends its boundary while a promise is set, as a lazy sibling does the
 * first time it renders. React hides the boundary's content, including the
 * prompt, and disconnects its layout effects until the promise settles. */
interface Gate {
  pending: Promise<void> | null;
}
function Sibling({ gate }: { gate: Gate }): null {
  if (gate.pending) use(gate.pending);
  return null;
}

/** The product wiring of the Desktop prompt: the real draft controller, the
 * real editor gateway and the real draft store. One instance per session, as
 * App mounts it (`key={session.id}`). */
function Prompt(
  { sessionId, vim, live }: { sessionId: string; vim: boolean; live: Live },
): React.JSX.Element {
  const editorRef = useRef<ComposerEditorHandle | null>(null);
  const controller = useComposerDraftController(sessionId, editorRef, {
    mirrorTextInReact: false,
  });
  live.controller = controller;
  return (
    <PlatformComposerEditor
      ref={(handle): void => {
        editorRef.current = handle;
        live.editor = handle;
      }}
      value={controller.initialText.current}
      onChange={controller.setText}
      onSubmit={() => false}
      sessionId={sessionId}
      commands={() => []}
      vim={vim}
      fill
      borderless
      endInset={0}
    />
  );
}

/** Editing in the Desktop prompt must survive what replaces the composer: a
 * session switch at any moment, including while a pasted image is still
 * encoding, and the editor swap that turning Vim on performs. */
export async function checkComposerDraftPersistence(): Promise<string> {
  const first = "conformance-draft-a";
  const second = "conformance-draft-b";
  for (const id of [first, second]) setDraft(id, { text: "", attachments: [] });
  const container = document.createElement("div");
  container.style.cssText = "width:720px;height:320px;display:flex;flex-direction:column";
  document.body.append(container);
  const root = createRoot(container);
  const live: Live = { editor: null, controller: null };
  const gate: Gate = { pending: null };
  const show = (sessionId: string, vim = false): void =>
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider
          value={{
            kind: "desktop",
            input: "keyboard-pointer",
            touchCapable: false,
            finePointer: true,
            hover: true,
          }}
        >
          <BrowserProductTheme>
            <CssBaseline />
            <Suspense fallback={null}>
              <Prompt key={sessionId} sessionId={sessionId} vim={vim} live={live} />
              <Sibling gate={gate} />
            </Suspense>
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
  const editor = (): ComposerEditorHandle => {
    check(live.editor, "The prompt editor did not mount");
    return live.editor;
  };
  try {
    show(first);
    await until(() => live.editor !== null, "The prompt editor did not mount");

    // Typing, then a switch in the same task: nothing has rendered or idled.
    editor().insertText("typed then switched");
    show(second);
    check(
      getDraft(first).text === "typed then switched",
      "Typing was not stored before the composer was replaced",
    );
    check(editor().getValue() === "", "The other session shows a foreign draft");
    show(first);
    check(
      editor().getValue() === "typed then switched",
      "The draft did not come back after switching sessions",
    );

    // Paste an image, keep typing while it encodes, switch before it settles.
    check(live.controller, "The draft controller did not mount");
    live.controller.addFiles([await screenshot()], { preserveFocus: true });
    const [token] = imageTokensInText(editor().getValue());
    check(token, "The pasted image has no inline token");
    editor().insertText(" typed while encoding");
    show(second);
    const stored = getDraft(first).text;
    check(
      stored.includes("typed while encoding") && stored.includes(token.id),
      "Typing during image encoding was not stored",
    );
    await until(
      () => getDraft(first).attachments.some((attachment) => attachment.id === token.id),
      "The image that finished encoding after the switch never reached the draft",
    );
    show(first);
    check(live.controller, "The draft controller did not remount");
    check(
      editor().getValue().includes("typed while encoding") &&
        editor().getValue().includes(token.id),
      "The remounted prompt lost text or the image token",
    );
    check(
      live.controller.attachments.some((attachment) =>
        attachment.id === token.id && attachment.pending !== true
      ) && live.controller.sendable,
      "The remounted prompt cannot send the pasted image",
    );

    // A sibling suspends the boundary around the prompt and resumes: React
    // hides the prompt and the editor library rebuilds its view on return.
    editor().insertText(" before suspending");
    const beforeSuspending = editor().getValue();
    let resume!: () => void;
    gate.pending = new Promise<void>((resolve) => {
      resume = resolve;
    });
    show(first);
    await new Promise((resolve) => setTimeout(resolve, 60));
    gate.pending = null;
    resume();
    await until(
      () =>
        (container.querySelector<HTMLElement>(".cm-editor")?.offsetParent ?? null) !== null,
      "The prompt did not come back after its boundary resumed",
    );
    await new Promise((resolve) => setTimeout(resolve, 300));
    check(
      editor().getValue() === beforeSuspending,
      "A suspended boundary reverted the prompt: " + JSON.stringify(editor().getValue()),
    );
    editor().insertText("!");
    // The rebuilt view starts with its caret at the document start.
    check(
      getDraft(first).text === editor().getValue() &&
        getDraft(first).text.length === beforeSuspending.length + 1 &&
        getDraft(first).text.includes("before suspending"),
      "Typing after the boundary resumed overwrote the stored draft",
    );

    // Turning Vim on swaps the editor (placeholder, then the real one).
    editor().insertText(" before vim");
    const typed = editor().getValue();
    live.editor = null;
    show(first, true);
    await until(
      () => live.editor !== null && container.querySelector(".cm-content[contenteditable='true']") !== null,
      "The Vim editor did not become interactive",
    );
    check(
      editor().getValue() === typed,
      "Turning Vim on reverted the prompt to the text it opened with",
    );
    check(getDraft(first).text === typed, "Turning Vim on changed the stored draft");
    return "Desktop prompt drafts survive a session switch, a slow image, a suspended boundary and the Vim editor swap";
  } finally {
    root.unmount();
    container.remove();
    for (const id of [first, second]) setDraft(id, { text: "", attachments: [] });
  }
}
