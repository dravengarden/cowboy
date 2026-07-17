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
import {
  type DesktopVimRuntimeState,
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
  shouldRestoreDesktopComposerFocus,
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

  useEffect(() => {
    if (runtimeState !== "ready") return undefined;
    const restore = (): void => {
      const region = document.querySelector<HTMLElement>(
        "[data-desktop-region='prompt.composer'][data-desktop-focused='true']",
      );
      const active = document.activeElement;
      const focusIsUnownedOrTemporary = active === document.body || active === region;
      if (
        !shouldRestoreDesktopComposerFocus(
          surface.kind,
          vim,
          runtimeState,
          region !== null,
          focusIsUnownedOrTemporary,
        )
      ) return;
      region?.querySelector<HTMLElement>("[data-vim-command-sink]")?.focus({
        preventScroll: true,
      });
    };
    // CM6 normally installs the sink before parent effects run. Repeat once on
    // the next frame for browsers that commit the child ref/plugin a beat later;
    // every attempt rechecks ownership so it cannot steal a newer user focus.
    restore();
    const frame = requestAnimationFrame(restore);
    return (): void => cancelAnimationFrame(frame);
  }, [runtimeState, surface.kind, vim]);

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
