/** Isolated native-input acceptance surface. Used only by the conformance bundle. */
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Box, Button, CssBaseline, Stack } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { PlatformComposerEditor } from "./composer/PlatformComposerEditor";
import type { ComposerEditorHandle } from "./ComposerEditor";
import { EditorExtensionsDialog } from "./editorExtensions/EditorExtensionsDialog";
import { openEditorExtensions } from "./editorExtensions/host";
import { fileToAttachment } from "./attachments";
import { seedInlineAttachments } from "./inlineImages";
import { toggleComposerSourceMode } from "./composerSourceMode";

export function mountDraftNativeInputFixture(): void {
  const events: { type: string; data: string | null }[] = [];
  for (
    const type of [
      "compositionstart",
      "compositionupdate",
      "compositionend",
      "beforeinput",
    ]
  ) {
    document.addEventListener(type, (event) => {
      events.push({ type, data: (event as InputEvent).data ?? null });
      if (events.length > 120) events.shift();
    });
  }
  function InputFixture() {
    const editor = useRef<ComposerEditorHandle | null>(null);
    const [epoch, setEpoch] = useState(0);
    const [seed, setSeed] = useState("");
    const [value, setValue] = useState("");
    const [expanded, setExpanded] = useState(true);
    const [context, setContext] = useState<"document" | "session">("document");
    const current = useRef(value);
    current.current = value;
    useEffect(() => {
      const timer = setInterval(() => {
        const payload = {
          value: current.current,
          context,
          expanded,
          selection: editor.current?.getSelection(),
          focus: editor.current?.hasFocus(),
          native: !!document.querySelector(
            "textarea[data-mobile-native-textarea]",
          ),
          images: document.querySelectorAll(".cm-content img").length,
          events,
          userAgent: navigator.userAgent,
          viewport: {
            width: innerWidth,
            height: innerHeight,
            visualHeight: visualViewport?.height,
          },
        };
        const bridge = (globalThis as typeof globalThis & {
          webkit?: {
            messageHandlers: {
              report: { postMessage: (value: unknown) => void };
            };
          };
        }).webkit;
        bridge?.messageHandlers.report.postMessage(payload);
      }, 200);
      return () => clearInterval(timer);
    }, [context, expanded]);
    const clipboard = (kind: string): void => {
      (globalThis as typeof globalThis & {
        webkit?: {
          messageHandlers: {
            clipboard: { postMessage: (value: string) => void };
          };
        };
      }).webkit?.messageHandlers.clipboard.postMessage(kind);
    };
    const reset = (next: string): void => {
      setSeed(next);
      setValue(next);
      setEpoch((n) => n + 1);
    };
    const pasteFiles = (files: File[]): void => {
      void Promise.all(files.map((file) => fileToAttachment(file))).then(
        (attachments) => {
          seedInlineAttachments(attachments);
          editor.current?.insertImages(attachments);
        },
      );
    };
    return (
      <Stack
        sx={{
          height: "100dvh",
          pt: "env(safe-area-inset-top)",
          pb: "env(safe-area-inset-bottom)",
        }}
      >
        <Stack direction="row" sx={{ flexWrap: "wrap" }}>
          <Button onClick={() => reset("")}>Plain</Button>
          <Button onClick={() => reset("# Heading\n\n**bold** body\n")}>
            Markdown
          </Button>
          <Button
            onClick={() =>
              setContext((c) => c === "document" ? "session" : "document")}
          >
            Context
          </Button>
          <Button
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => setExpanded((v) => !v)}
          >
            Expand
          </Button>
          <Button
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => editor.current?.focus()}
          >
            Focus
          </Button>
          <Button
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => void toggleComposerSourceMode()}
          >
            Source
          </Button>
          <Button
            onPointerDown={(e) => e.preventDefault()}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => editor.current?.undo()}
          >
            Undo
          </Button>
          <Button onClick={() => clipboard("text")}>Copy text</Button>
          <Button onClick={() => clipboard("image")}>Copy image</Button>
          <Button
            onClick={() => {
              if (editor.current) openEditorExtensions(editor.current);
            }}
          >
            Extensions
          </Button>
        </Stack>
        <Box
          sx={{
            display: "flex",
            flex: expanded ? 1 : "0 0 180px",
            minHeight: 0,
            px: 1,
          }}
        >
          <PlatformComposerEditor
            key={epoch}
            ref={editor}
            value={seed}
            nativeValue={value}
            onChange={setValue}
            {...(context === "session"
              ? { sessionId: "native-fixture-session" }
              : { documentId: "native-fixture-document" })}
            fill
            borderless
            placeholder="Write here"
            onPasteFiles={pasteFiles}
          />
        </Box>
        <EditorExtensionsDialog />
      </Stack>
    );
  }
  document.body.style.margin = "0";
  const container = document.createElement("div");
  document.body.append(container);
  createRoot(container).render(
    <SurfaceProvider>
      <BrowserProductTheme>
        <CssBaseline />
        <InputFixture />
      </BrowserProductTheme>
    </SurfaceProvider>,
  );
}
