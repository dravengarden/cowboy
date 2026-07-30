import {
  type ComponentPropsWithoutRef,
  forwardRef,
  useEffect,
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

type ComposerEditorProps = ComponentPropsWithoutRef<typeof ComposerEditor>;

export interface PlatformComposerEditorProps
  extends Omit<ComposerEditorProps, "vim"> {
  /** Desktop preference. Touch surfaces always force this off. */
  vim?: boolean;
}

// The only editor gateway used by product shells, including fullscreen/expanded
// pending-message editors. It deliberately does
// not alter the CM6 extension set, controlled/uncontrolled behaviour, or iOS
// event handling; it only enforces platform capabilities at the boundary.
export const PlatformComposerEditor = forwardRef<
  ComposerEditorHandle,
  PlatformComposerEditorProps
>(function PlatformComposerEditor(
  { vim = false, ...props },
  ref,
): React.JSX.Element {
  const surface = useSurfaceProfile();
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
  // Compact touch composition deliberately uses a real textarea. iOS owns its
  // caret, selection, and edit menu end-to-end, so a long press on an empty
  // prompt reliably exposes Paste/AutoFill instead of going through WKWebView's
  // incomplete contenteditable interaction path. Fullscreen/fill editors keep
  // CM6 because they need the markdown toolbar and inline widgets.
  if (surface.kind !== "desktop" && !props.expanded && !props.fill) {
    return (
      <ComposerTextarea
        ref={ref}
        value={props.value}
        onChange={props.onChange}
        onSubmit={props.onSubmit}
        sessionId={props.sessionId}
        commands={props.commands}
        {...(props.onSaveDraft ? { onSaveDraft: props.onSaveDraft } : {})}
        {...(props.placeholder ? { placeholder: props.placeholder } : {})}
        {...(props.disabled ? { disabled: true } : {})}
        {...(props.autoFocus ? { autoFocus: true } : {})}
        {...(props.onEscape ? { onEscape: props.onEscape } : {})}
        {...(props.onPasteFiles ? { onPasteFiles: props.onPasteFiles } : {})}
        {...(props.endInset !== undefined ? { endInset: props.endInset } : {})}
        {...(props.borderless !== undefined ? { borderless: props.borderless } : {})}
      />
    );
  }
  return (
    <ComposerEditor
      {...props}
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
