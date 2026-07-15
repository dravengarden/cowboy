import { type ComponentPropsWithoutRef, forwardRef } from "react";
import { ComposerEditor, type ComposerEditorHandle } from "../ComposerEditor";
import { useSurfaceProfile } from "../surface/SurfaceProfile";

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
  return (
    <ComposerEditor
      {...props}
      ref={ref}
      vim={surface.kind === "desktop" && vim}
    />
  );
});

export type { ComposerEditorHandle } from "../ComposerEditor";
