import {
  type ComponentPropsWithoutRef,
  forwardRef,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  ComposerEditor,
  type ComposerEditorHandle,
  type ComposerEditorSelection,
} from "../ComposerEditor";
import { ComposerTextarea } from "../ComposerTextarea";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  isDesktopVimRuntimeLoaded,
  preloadDesktopVimRuntime,
} from "../desktop/vim/runtimeLoader";
import {
  desktopEditorMountFocusPolicy,
  desktopVimMountPolicy,
  type DesktopVimRuntimeState,
  shouldPreloadDesktopVim,
} from "./desktopVimMountPolicy";
import {
  composerEditorMountSeed,
  holdTouchEditorKind,
  nativeDemotionSelection,
  nativePromotionSelection,
  shouldFocusDemotedEditor,
  shouldFocusPromotedEditor,
  shouldUseNativeTouchEditor,
} from "./mobileCompactEditorPolicy";

type ComposerEditorProps = ComponentPropsWithoutRef<typeof ComposerEditor>;

export interface PlatformComposerEditorProps
  extends Omit<ComposerEditorProps, "vim" | "touchInput"> {
  /** Desktop preference. Touch surfaces always force this off. */
  vim?: boolean;
  /** Focus the final interactive CM6 instance at the end of its seed document. */
  focusEndOnMount?: boolean;
  /**
   * Live React value used only by the native touch editor, including fullscreen.
   * CM6 keeps the frozen `value` seed so React updates cannot bounce its caret
   * or IME.
   */
  nativeValue?: string;
}

// The only editor gateway used by product shells, including fullscreen/expanded
// pending-message editors. It deliberately does
// not alter the CM6 extension set or controlled/uncontrolled behaviour. On
// touch surfaces it gives plain token-free prose to a native textarea (UIKit
// owns the long-press menu), promoting the same document to CM6 when any
// complete Obsidian live-preview construct (emphasis, highlight, link, heading,
// list, quote, fence, …) needs marker hiding, or when an inline
// image token requires a widget. This includes fullscreen: WebKit
// contenteditable is not a reliable edit-menu anchor far away from its
// nearest real text line.
export const PlatformComposerEditor = forwardRef<
  ComposerEditorHandle,
  PlatformComposerEditorProps
>(function PlatformComposerEditor(
  { vim = false, nativeValue, focusEndOnMount = false, ...props },
  ref,
): React.JSX.Element {
  const surface = useSurfaceProfile();
  const touchValue = nativeValue ?? props.value;
  // This ref describes the LAST COMMITTED editor, not the last render attempt.
  // React can replay a render before commit; mutating it during render consumed
  // the one-shot native -> CM6 transition and reset image-paste selection to 0.
  const committedNativeTouchRef = useRef(
    shouldUseNativeTouchEditor(surface.kind, touchValue),
  );
  const promotionCaretRef = useRef<number | null>(null);
  const demotionSelectionRef = useRef<ComposerEditorSelection | null>(null);
  const demotionFocusPendingRef = useRef(false);
  const childEditorRef = useRef<ComposerEditorHandle | null>(null);
  const forwardedRefRef = useRef(ref);
  forwardedRefRef.current = ref;
  const bindEditorRef = useCallback(
    (handle: ComposerEditorHandle | null): void => {
      childEditorRef.current = handle;
      const forwarded = forwardedRefRef.current;
      if (typeof forwarded === "function") forwarded(handle);
      else if (forwarded !== null) forwarded.current = handle;
    },
    [],
  );
  const surfaceKindRef = useRef(surface.kind);
  surfaceKindRef.current = surface.kind;
  const onChangeRef = useRef(props.onChange);
  onChangeRef.current = props.onChange;
  // Native Pinyin candidate confirmation is still one composition. Swapping
  // the textarea for CM6 (or back) in that window wipes marked text — the
  // candidate tap looks like it did nothing and composition dies. Hold the
  // committed host until compositionend; Obsidian never remounts mid-IME.
  const composingRef = useRef(false);
  useEffect(() => {
    const start = (): void => {
      composingRef.current = true;
    };
    const end = (): void => {
      composingRef.current = false;
    };
    document.addEventListener("compositionstart", start, true);
    document.addEventListener("compositionend", end, true);
    return (): void => {
      document.removeEventListener("compositionstart", start, true);
      document.removeEventListener("compositionend", end, true);
    };
  }, []);
  const nativeTouch = holdTouchEditorKind(
    committedNativeTouchRef.current,
    composingRef.current,
    shouldUseNativeTouchEditor(surface.kind, touchValue),
  );
  // The iOS paste-permission alert temporarily owns focus. When the accepted
  // paste event returns, `document.activeElement` can therefore be BODY even
  // though UIKit still considers this one native paste transaction. Preserve
  // that intent through the synchronous native-textarea -> CM6 promotion so
  // the replacement inherits the keyboard in the same React commit.
  const pastePromotionPendingRef = useRef(false);
  const handleChange = useCallback((next: string): void => {
    const nextNative = holdTouchEditorKind(
      committedNativeTouchRef.current,
      composingRef.current,
      shouldUseNativeTouchEditor(surfaceKindRef.current, next),
    );
    const demoting = !committedNativeTouchRef.current && nextNative;
    const promoting = committedNativeTouchRef.current && !nextNative;
    if (demoting) {
      // Capture the post-edit selection before the parent mirrors `next` and
      // replaces CM6. The refs survive React replay until the native child's
      // layout commit confirms that it consumed this one-shot handoff.
      demotionSelectionRef.current = childEditorRef.current?.getSelection() ?? null;
      demotionFocusPendingRef.current = childEditorRef.current?.hasFocus() ?? false;
    }
    if (promoting) {
      const selection = childEditorRef.current?.getSelection();
      promotionCaretRef.current = selection?.head ?? next.length;
      if (childEditorRef.current?.hasFocus()) {
        pastePromotionPendingRef.current = true;
      }
    }
    onChangeRef.current(next);
  }, []);
  // During an image Paste, the token is inserted synchronously and this render
  // replaces the still-focused native textarea. Autofocus the CM6 mount in the
  // same discrete UIKit gesture; a later rAF focus cannot inherit the keyboard.
  // The explicit paste intent covers the system permission alert's transient
  // focus loss without weakening ordinary attachment/file-picker semantics.
  const wasNativeTouch = committedNativeTouchRef.current;
  const demotingToNative = !wasNativeTouch && nativeTouch;
  const liveDemotionSelection = demotingToNative
    ? demotionSelectionRef.current ?? childEditorRef.current?.getSelection() ?? null
    : null;
  const demotionSelection = nativeDemotionSelection(
    wasNativeTouch,
    nativeTouch,
    liveDemotionSelection,
  );
  const focusDemotedEditor = shouldFocusDemotedEditor(
    wasNativeTouch,
    nativeTouch,
    demotingToNative &&
      (demotionFocusPendingRef.current ||
        (childEditorRef.current?.hasFocus() ?? false)),
  );
  const focusPromotedEditor = shouldFocusPromotedEditor(
    wasNativeTouch,
    nativeTouch,
    typeof document !== "undefined" &&
      document.activeElement instanceof HTMLTextAreaElement,
    pastePromotionPendingRef.current,
  );
  const cmSeedRef = useRef(props.value);
  cmSeedRef.current = composerEditorMountSeed(
    wasNativeTouch,
    nativeTouch,
    cmSeedRef.current,
    touchValue,
  );
  const initialSelection = nativePromotionSelection(
    wasNativeTouch,
    nativeTouch,
    promotionCaretRef.current,
  );
  const [runtimeState, setRuntimeState] = useState<DesktopVimRuntimeState>(
    () => isDesktopVimRuntimeLoaded() ? "ready" : "pending",
  );

  useLayoutEffect(() => {
    const previousNativeTouch = committedNativeTouchRef.current;
    const promoted = previousNativeTouch && !nativeTouch;
    const demoted = !previousNativeTouch && nativeTouch;
    committedNativeTouchRef.current = nativeTouch;
    if (promoted) {
      // The child CM6 state has now committed with the supplied selection. Only
      // now is it safe to release the one-shot paste/focus claim.
      promotionCaretRef.current = null;
      pastePromotionPendingRef.current = false;
    }
    if (demoted) {
      demotionSelectionRef.current = null;
      demotionFocusPendingRef.current = false;
    }
  }, [nativeTouch]);

  useEffect(() => {
    if (!shouldPreloadDesktopVim(surface.kind, vim, runtimeState)) {
      return undefined;
    }
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
  const mountFocus = desktopEditorMountFocusPolicy(
    focusEndOnMount,
    policy.awaitingRuntime,
    cmSeedRef.current.length,
  );
  const cmInitialSelection = mountFocus.initialSelection ?? initialSelection;
  if (nativeTouch) {
    return (
      <ComposerTextarea
        ref={bindEditorRef}
        value={touchValue}
        onChange={handleChange}
        onSubmit={props.onSubmit}
        sessionId={props.sessionId}
        commands={props.commands}
        {...(props.onSaveDraft ? { onSaveDraft: props.onSaveDraft } : {})}
        {...(props.placeholder !== undefined
          ? { placeholder: props.placeholder }
          : {})}
        {...(props.disabled !== undefined ? { disabled: props.disabled } : {})}
        autoFocus={props.autoFocus || focusDemotedEditor}
        {...(demotionSelection !== undefined
          ? { initialSelection: demotionSelection }
          : {})}
        {...(props.onEscape ? { onEscape: props.onEscape } : {})}
        {...(props.onSelectionChange
          ? { onSelectionChange: props.onSelectionChange }
          : {})}
        onInlineImageInsertion={(caret, preserveFocus): void => {
          promotionCaretRef.current = caret;
          // The dedicated native Paste button does not pass through the
          // textarea's browser `paste` event. Its synchronously staged pending
          // placeholder therefore claims the same one-commit focus transfer
          // here, while ordinary file-picker inserts remain unclaimed.
          pastePromotionPendingRef.current = preserveFocus;
        }}
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
                // A successful synchronous image insert records its caret and
                // keeps this claim through the committed CM6 mount. Clear only
                // no-op/error paste attempts that never started promotion.
                if (promotionCaretRef.current === null) {
                  pastePromotionPendingRef.current = false;
                }
              });
            },
          }
          : {})}
        {...(props.endInset !== undefined ? { endInset: props.endInset } : {})}
        {...(props.borderless !== undefined
          ? { borderless: props.borderless }
          : {})}
        expanded={(props.expanded ?? false) || (props.fill ?? false)}
      />
    );
  }
  return (
    <ComposerEditor
      {...props}
      value={cmSeedRef.current}
      onChange={handleChange}
      autoFocus={props.autoFocus || focusPromotedEditor ||
        mountFocus.focusOnMount}
      {...(cmInitialSelection !== undefined
        ? { initialSelection: cmInitialSelection }
        : {})}
      // The loading editor is deliberately a separate, non-interactive CM6
      // lifetime. The real editor mounts only after Vim can be included in its
      // initial EditorState, so native IME composition never spans reconfigure.
      key={policy.awaitingRuntime ? "vim-loading" : "interactive"}
      ref={bindEditorRef}
      disabled={props.disabled || policy.awaitingRuntime}
      vim={policy.enableVim}
      touchInput={surface.kind !== "desktop"}
    />
  );
});

export type {
  ComposerEditorHandle,
  ComposerEditorSelection,
} from "../ComposerEditor";
