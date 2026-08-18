import {
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { ContentPaste, Tune } from "@mui/icons-material";
import type {
  ComposerEditorHandle,
  ComposerEditorSelection,
} from "./composer/PlatformComposerEditor";
import type { NativeClipboardImagePasteRequest } from "./composer/nativeClipboardImagePaste";
import {
  type ClipboardAvailability,
  createClipboardPort,
} from "./composer/clipboardPort";
import {
  COMPOSER_COMMANDS_BY_ID,
  type ComposerCommand,
} from "./composerCommands";
import { MobileComposerAccessoryButton } from "./MobileComposerAccessoryDock";
import { haptic } from "./haptic";
import { flushObservability, reportClientLog } from "./observability";
import { useReliableTouchTap } from "./useReliableTouchTap";

const clipboardPort = createClipboardPort();

interface MobileComposerFormatActionsProps {
  commandIds: readonly string[];
  editorRef: RefObject<ComposerEditorHandle | null>;
  onAttach: () => void;
  onPasteImages: (
    request: NativeClipboardImagePasteRequest,
  ) => Promise<void> | void;
  onCustomize?: () => void;
}

function MobileClipboardPasteButton({
  editorRef,
  onPasteImages,
}: Pick<
  MobileComposerFormatActionsProps,
  "editorRef" | "onPasteImages"
>): React.JSX.Element {
  const [availability, setAvailability] = useState<ClipboardAvailability>({
    surface: clipboardPort.surface,
    pasteAvailable: clipboardPort.surface === "web",
    stageImagesFirst: false,
    imageCount: 0,
  });
  const [reading, setReading] = useState(false);
  const refreshGeneration = useRef(0);
  const readingRef = useRef(false);
  const capturedSelectionRef = useRef<ComposerEditorSelection | null>(null);

  const refresh = useCallback((): void => {
    const generation = ++refreshGeneration.current;
    void clipboardPort.status().then((status) => {
      if (generation !== refreshGeneration.current) return;
      setAvailability(status);
    });
  }, []);

  useEffect(() => {
    refresh();
    const refreshVisible = (): void => {
      if (typeof document === "undefined" || !document.hidden) refresh();
    };
    globalThis.addEventListener("focus", refresh);
    globalThis.addEventListener("pageshow", refresh);
    globalThis.addEventListener("cowboy:native-resume", refresh);
    globalThis.addEventListener("cowboy:clipboard-change", refresh);
    globalThis.document?.addEventListener("visibilitychange", refreshVisible);
    // Native can see pasteboard metadata without reading payloads. Web
    // cannot; polling would only burn the user-gesture budget. Payloads
    // stay gated behind the explicit tap on both surfaces.
    const poll = clipboardPort.surface === "native"
      ? globalThis.setInterval(refreshVisible, 1000)
      : 0;
    return (): void => {
      refreshGeneration.current += 1;
      if (poll !== 0) globalThis.clearInterval(poll);
      globalThis.removeEventListener("focus", refresh);
      globalThis.removeEventListener("pageshow", refresh);
      globalThis.removeEventListener("cowboy:native-resume", refresh);
      globalThis.removeEventListener("cowboy:clipboard-change", refresh);
      globalThis.document?.removeEventListener(
        "visibilitychange",
        refreshVisible,
      );
    };
  }, [refresh]);

  const paste = async (): Promise<void> => {
    if (readingRef.current) return;
    const selection = capturedSelectionRef.current ??
      editorRef.current?.getSelection();
    capturedSelectionRef.current = null;
    if (!selection) {
      reportClientLog(
        "warn",
        "mobile_paste_blocked",
        "Mobile Paste had no editor selection",
        { reason: "no_selection" },
      );
      void flushObservability();
      return;
    }
    if (!availability.pasteAvailable) {
      reportClientLog(
        "warn",
        "mobile_paste_blocked",
        "Mobile Paste was unavailable at activation",
        { reason: "unavailable", surface: availability.surface },
      );
      void flushObservability();
      return;
    }
    reportClientLog(
      "info",
      "mobile_paste_started",
      "Mobile Paste activation started",
      {
        path: availability.stageImagesFirst ? "image" : "auto",
        surface: availability.surface,
        advertised_images: availability.imageCount,
        selection_span: Math.abs(selection.anchor - selection.head),
      },
    );
    void flushObservability();
    haptic();
    readingRef.current = true;
    try {
      if (availability.stageImagesFirst) {
        let fileCount = 0;
        // Native image paste must stage placeholders before the privacy-gated
        // read so textarea -> CM6 stays inside the originating UIKit tap.
        const completion = onPasteImages({
          expectedCount: Math.max(1, availability.imageCount),
          selection,
          read: async (): Promise<File[]> => {
            const contents = await clipboardPort.read();
            fileCount = contents.files.length;
            return contents.files;
          },
        });
        // Staging must happen before this state update can disable a button that
        // WebKit briefly made the accessibility focus owner during the tap.
        setReading(true);
        await completion;
        reportClientLog(
          fileCount > 0 ? "info" : "warn",
          "mobile_paste_finished",
          fileCount > 0
            ? "Mobile image Paste completed"
            : "Mobile image Paste returned no usable image",
          {
            path: "image",
            surface: availability.surface,
            advertised_images: availability.imageCount,
            file_count: fileCount,
          },
        );
      } else {
        // Web, and native text-only: read first, then insert. Images discovered
        // on the tap still go through the shared staging host.
        const editor = editorRef.current;
        if (!editor) return;
        editor.focusSelection(selection);
        setReading(true);
        const contents = await clipboardPort.read();
        if (contents.files.length > 0) {
          await onPasteImages({
            expectedCount: contents.files.length,
            selection,
            read: (): Promise<File[]> => Promise.resolve(contents.files),
          });
          reportClientLog(
            "info",
            "mobile_paste_finished",
            "Mobile image Paste completed",
            {
              path: "image",
              surface: availability.surface,
              file_count: contents.files.length,
            },
          );
        } else {
          if (contents.text.length > 0) {
            editorRef.current?.insertText(contents.text, selection);
          }
          reportClientLog(
            contents.text.length > 0 ? "info" : "warn",
            "mobile_paste_finished",
            contents.text.length > 0
              ? "Mobile text Paste completed"
              : "Mobile text Paste returned no text",
            {
              path: "text",
              surface: availability.surface,
              result: contents.text.length > 0 ? "ok" : "empty",
              text_length: contents.text.length,
            },
          );
        }
      }
    } finally {
      readingRef.current = false;
      setReading(false);
      refresh();
      void flushObservability();
    }
  };

  const pasteTap = useReliableTouchTap<HTMLButtonElement>(() => {
    void paste();
  });
  const pasteAvailable = availability.pasteAvailable;

  return (
    <MobileComposerAccessoryButton
      title="Paste"
      disabled={
        !pasteAvailable ||
        (reading && availability.stageImagesFirst)
      }
      color={pasteAvailable ? "primary" : "default"}
      onPointerDown={(event): void => {
        // Accessibility activation in iOS WebKit can focus the button even
        // though its pointer default is prevented. Preserve the textarea range
        // before that transfer; the reliable pointerup path restores/promotes
        // the editor before WebKit's mis-targeted compatibility click.
        capturedSelectionRef.current = editorRef.current?.getSelection() ??
          null;
        pasteTap.onPointerDown(event);
      }}
      onPointerMove={pasteTap.onPointerMove}
      onPointerUp={pasteTap.onPointerUp}
      onPointerCancel={(event): void => {
        capturedSelectionRef.current = null;
        pasteTap.onPointerCancel(event);
      }}
      onClick={(event): void => {
        // VoiceOver/XCUITest may issue a trusted click without Pointer Events.
        // Capture the editor's remembered range and cancel the button's default
        // focus action before it can end the native first-responder session.
        capturedSelectionRef.current ??=
          editorRef.current?.getSelection() ?? null;
        event.preventDefault();
        pasteTap.onClick(event);
      }}
    >
      <ContentPaste />
    </MobileComposerAccessoryButton>
  );
}

/** Shared ordering for compact, fullscreen, Queue, and Draft touch editors. */
export function MobileComposerFormatActions({
  commandIds,
  editorRef,
  onAttach,
  onPasteImages,
  onCustomize,
}: MobileComposerFormatActionsProps): React.JSX.Element {
  const commands = commandIds
    .map((id) => COMPOSER_COMMANDS_BY_ID[id])
    .filter((command): command is ComposerCommand => command !== undefined);
  const redoIndex = commands.findIndex((command) => command.id === "redo");
  const pasteIndex = redoIndex >= 0
    ? redoIndex + 1
    : Math.min(2, commands.length);

  const renderCommand = (command: ComposerCommand): React.JSX.Element => (
    <MobileComposerAccessoryButton
      key={command.id}
      title={command.label}
      onClick={(): void => {
        const editor = editorRef.current;
        if (editor === null) return;
        haptic();
        command.run({ editor, attach: onAttach });
      }}
    >
      {command.icon}
    </MobileComposerAccessoryButton>
  );

  return (
    <>
      {commands.slice(0, pasteIndex).map(renderCommand)}
      <MobileClipboardPasteButton
        editorRef={editorRef}
        onPasteImages={onPasteImages}
      />
      {commands.slice(pasteIndex).map(renderCommand)}
      {onCustomize && (
        <MobileComposerAccessoryButton
          title="Customize toolbar"
          onClick={onCustomize}
        >
          <Tune />
        </MobileComposerAccessoryButton>
      )}
    </>
  );
}
