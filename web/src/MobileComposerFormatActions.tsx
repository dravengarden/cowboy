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
  COMPOSER_COMMANDS_BY_ID,
  type ComposerCommand,
} from "./composerCommands";
import { MobileComposerAccessoryButton } from "./MobileComposerAccessoryDock";
import {
  nativeClipboardImageStatus,
  readNativeClipboardImages,
} from "./nativeShell";
import { haptic } from "./haptic";
import { useReliableTouchTap } from "./useReliableTouchTap";

interface MobileComposerFormatActionsProps {
  commandIds: readonly string[];
  editorRef: RefObject<ComposerEditorHandle | null>;
  onAttach: () => void;
  onPasteImages: (
    request: NativeClipboardImagePasteRequest,
  ) => Promise<void> | void;
  onCustomize?: () => void;
}

function MobileClipboardImagePasteButton({
  editorRef,
  onPasteImages,
}: Pick<
  MobileComposerFormatActionsProps,
  "editorRef" | "onPasteImages"
>): React.JSX.Element {
  const [availability, setAvailability] = useState({
    hasImages: false,
    imageCount: 0,
  });
  const [reading, setReading] = useState(false);
  const refreshGeneration = useRef(0);
  const readingRef = useRef(false);
  const capturedSelectionRef = useRef<ComposerEditorSelection | null>(null);

  const refresh = useCallback((): void => {
    const generation = ++refreshGeneration.current;
    void nativeClipboardImageStatus().then((status) => {
      if (generation !== refreshGeneration.current) return;
      setAvailability({
        hasImages: status.supported && status.hasImages,
        imageCount: status.supported ? status.imageCount : 0,
      });
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
    // UIPasteboard change notifications are process-local on some iOS releases:
    // copying an image in another app, an IME clipboard shelf, or the screenshot
    // UI may not emit any event when Cowboy is already foregrounded. Poll only
    // the metadata bridge while this action is mounted and visible; image bytes
    // remain user-gesture gated in readNativeClipboardImages().
    const poll = globalThis.setInterval(refreshVisible, 1000);
    return (): void => {
      refreshGeneration.current += 1;
      globalThis.clearInterval(poll);
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
    if (!selection) return;
    haptic();
    readingRef.current = true;
    try {
      // The callback stages inline placeholders and performs the native
      // textarea -> CM6 focus handoff synchronously. Only then may it invoke the
      // privacy-gated `read` thunk and await provider bytes.
      const completion = onPasteImages({
        expectedCount: Math.max(1, availability.imageCount),
        selection,
        read: readNativeClipboardImages,
      });
      // Staging must happen before this state update can disable a button that
      // WebKit briefly made the accessibility focus owner during the tap.
      setReading(true);
      await completion;
    } finally {
      readingRef.current = false;
      setReading(false);
      refresh();
    }
  };

  const pasteTap = useReliableTouchTap<HTMLButtonElement>(() => {
    void paste();
  });

  return (
    <MobileComposerAccessoryButton
      title="Paste image"
      disabled={!availability.hasImages || reading}
      color={availability.hasImages ? "primary" : "default"}
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
      onClick={pasteTap.onClick}
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
      <MobileClipboardImagePasteButton
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
