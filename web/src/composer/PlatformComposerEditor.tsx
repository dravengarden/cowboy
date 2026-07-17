import {
  type ComponentPropsWithoutRef,
  forwardRef,
  useEffect,
  useState,
} from "react";
import { ComposerEditor, type ComposerEditorHandle } from "../ComposerEditor";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  isDesktopVimRuntimeLoaded,
  preloadDesktopVimRuntime,
} from "../desktop/vim/runtimeLoader";
import { desktopVimMountPolicy } from "./desktopVimMountPolicy";

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
  const wantsDesktopVim = surface.kind === "desktop" && vim;
  const [runtimeState, setRuntimeState] = useState<
    "idle" | "loading" | "ready" | "failed"
  >(() => isDesktopVimRuntimeLoaded() ? "ready" : "idle");

  useEffect(() => {
    if (
      !wantsDesktopVim || runtimeState === "ready" ||
      runtimeState === "loading" || runtimeState === "failed"
    ) {
      return undefined;
    }
    let alive = true;
    setRuntimeState("loading");
    void preloadDesktopVimRuntime().then((loaded) => {
      if (alive) setRuntimeState(loaded ? "ready" : "failed");
    });
    return (): void => {
      alive = false;
    };
  }, [runtimeState, wantsDesktopVim]);

  const policy = desktopVimMountPolicy(
    surface.kind,
    vim,
    runtimeState === "ready",
    runtimeState === "failed",
  );
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
