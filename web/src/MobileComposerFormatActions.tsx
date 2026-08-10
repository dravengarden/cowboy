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

interface MobileComposerFormatActionsProps {
  commandIds: readonly string[];
  editorRef: RefObject<ComposerEditorHandle | null>;
  onAttach: () => void;
  onPasteImages: (
    files: File[],
    selection: ComposerEditorSelection,
  ) => void;
  onCustomize?: () => void;
}

function MobileClipboardImagePasteButton({
  editorRef,
  onPasteImages,
}: Pick<
  MobileComposerFormatActionsProps,
  "editorRef" | "onPasteImages"
>): React.JSX.Element {
  const [hasImages, setHasImages] = useState(false);
  const [reading, setReading] = useState(false);
  const refreshGeneration = useRef(0);

  const refresh = useCallback((): void => {
    const generation = ++refreshGeneration.current;
    void nativeClipboardImageStatus().then((status) => {
      if (generation !== refreshGeneration.current) return;
      setHasImages(status.supported && status.hasImages);
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
    if (reading) return;
    const selection = editorRef.current?.getSelection();
    if (!selection) return;
    haptic();
    setReading(true);
    try {
      const files = await readNativeClipboardImages();
      if (files.length > 0) onPasteImages(files, selection);
    } finally {
      setReading(false);
      refresh();
    }
  };

  return (
    <MobileComposerAccessoryButton
      title="Paste image"
      disabled={!hasImages || reading}
      color={hasImages ? "primary" : "default"}
      onClick={(): void => void paste()}
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
