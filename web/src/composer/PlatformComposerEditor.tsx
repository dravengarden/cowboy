import {
  type ComponentPropsWithoutRef,
  forwardRef,
  useEffect,
  useRef,
  useState,
} from "react";
import { ComposerEditor, type ComposerEditorHandle } from "../ComposerEditor";
import { ComposerTextarea } from "../ComposerTextarea";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  isDesktopVimRuntimeLoaded,
  preloadDesktopVimRuntime,
} from "../desktop/vim/runtimeLoader";
import {
  type DesktopVimRuntimeState,
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
} from "./desktopVimMountPolicy";
import {
  composerEditorMountSeed,
  shouldFocusPromotedEditor,
  shouldUseNativeCompactEditor,
} from "./mobileCompactEditorPolicy";

type ComposerEditorProps = ComponentPropsWithoutRef<typeof ComposerEditor>;

export interface PlatformComposerEditorProps
  extends Omit<ComposerEditorProps, "vim"> {
  /** Desktop preference. Touch surfaces always force this off. */
  vim?: boolean;
  /**
   * Live React value used only by the native compact touch editor. CM6 keeps
   * the frozen `value` seed so React updates cannot bounce its caret or IME.
   */
  nativeValue?: string;
}

// The only editor gateway used by product shells, including fullscreen/expanded
// pending-message editors. It deliberately does
// not alter the CM6 extension set or controlled/uncontrolled behaviour. On
// compact touch surfaces it gives token-free text to a native textarea (UIKit
// owns the long-press menu), promoting the same document to CM6 when an inline
// image token requires a widget.
export const PlatformComposerEditor = forwardRef<
  ComposerEditorHandle,
  PlatformComposerEditorProps
>(function PlatformComposerEditor(
  { vim = false, nativeValue, ...props },
  ref,
): React.JSX.Element {
  const surface = useSurfaceProfile();
  const touchValue = nativeValue ?? props.value;
  const nativeCompact = shouldUseNativeCompactEditor(
    surface.kind,
    props.expanded ?? false,
    props.fill ?? false,
    touchValue,
  );
  const wasNativeCompactRef = useRef(nativeCompact);
  // The iOS paste-permission alert temporarily owns focus. When the accepted
  // paste event returns, `document.activeElement` can therefore be BODY even
  // though UIKit still considers this one native paste transaction. Preserve
  // that intent through the synchronous native-textarea -> CM6 promotion so
  // the replacement inherits the keyboard in the same React commit.
  const pastePromotionPendingRef = useRef(false);
  // During an image Paste, the token is inserted synchronously and this render
  // replaces the still-focused native textarea. Autofocus the CM6 mount in the
  // same discrete UIKit gesture; a later rAF focus cannot inherit the keyboard.
  // The explicit paste intent covers the system permission alert's transient
  // focus loss without weakening ordinary attachment/file-picker semantics.
  const focusPromotedEditor = shouldFocusPromotedEditor(
    wasNativeCompactRef.current,
    nativeCompact,
    typeof document !== "undefined" &&
      document.activeElement instanceof HTMLTextAreaElement,
    pastePromotionPendingRef.current,
  );
  const cmSeedRef = useRef(props.value);
  cmSeedRef.current = composerEditorMountSeed(
    wasNativeCompactRef.current,
    nativeCompact,
    cmSeedRef.current,
    touchValue,
  );
  wasNativeCompactRef.current = nativeCompact;
  if (focusPromotedEditor) pastePromotionPendingRef.current = false;
  const [runtimeState, setRuntimeState] = useState<DesktopVimRuntimeState>(
    () => isDesktopVimRuntimeLoaded() ? "ready" : "pending",
  );

  useEffect(() => {
    if (!shouldPreloadDesktopVim(surface.kind, vim, runtimeState)) return undefined;
    let alive = true;
    void preloadDesktopVimRuntime().then((loaded) => {
      if (alive) setRuntimeState(loaded ? "ready" : "failed");
    });
    return (): void => {
      alive = false;
    };
  }, [runtimeState, surface.kind, vim]);

  const policy = desktopVimMountPolicy(
    surface.kind,
    vim,
    runtimeState === "ready",
    runtimeState === "failed",
  );
  if (nativeCompact) {
    return (
      <ComposerTextarea
        ref={ref}
        value={touchValue}
        onChange={props.onChange}
        onSubmit={props.onSubmit}
        sessionId={props.sessionId}
        commands={props.commands}
        {...(props.onSaveDraft ? { onSaveDraft: props.onSaveDraft } : {})}
        {...(props.placeholder !== undefined ? { placeholder: props.placeholder } : {})}
        {...(props.disabled !== undefined ? { disabled: props.disabled } : {})}
        {...(props.autoFocus !== undefined ? { autoFocus: props.autoFocus } : {})}
        {...(props.onEscape ? { onEscape: props.onEscape } : {})}
        {...(props.onPasteFiles
          ? {
            onPasteFiles: (files: File[]): void => {
              // Only an image inserts a synchronous inline token and promotes
              // this native control. Non-image attachment work stays async and
              // must not leave a focus claim behind for a later transition.
              pastePromotionPendingRef.current = files.some((file) =>
                (file.type || "").startsWith("image/")
              );
              props.onPasteFiles?.(files);
              queueMicrotask(() => {
                pastePromotionPendingRef.current = false;
              });
            },
          }
          : {})}
        {...(props.endInset !== undefined ? { endInset: props.endInset } : {})}
        {...(props.borderless !== undefined ? { borderless: props.borderless } : {})}
        {...(props.expanded !== undefined ? { expanded: props.expanded } : {})}
      />
    );
  }
  return (
    <ComposerEditor
      {...props}
      value={cmSeedRef.current}
      autoFocus={props.autoFocus || focusPromotedEditor}
      // The loading editor is deliberately a separate, non-interactive CM6
      // lifetime. The real editor mounts only after Vim can be included in its
      // initial EditorState, so native IME composition never spans reconfigure.
      key={policy.awaitingRuntime ? "vim-loading" : "interactive"}
      ref={ref}
      disabled={props.disabled || policy.awaitingRuntime}
      vim={policy.enableVim}
    />
  );
});

export type { ComposerEditorHandle } from "../ComposerEditor";
