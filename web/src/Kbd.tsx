import type React from "react";
import { type RefObject, useEffect, useRef } from "react";
import {
  ShortcutKeycap,
  type ShortcutKeycapAvailability,
  type ShortcutKeycapVariant,
} from "./ShortcutKeycap";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { confirmEnterIntent } from "./confirmShortcut";
import { popupOwnsKeys } from "./desktop/commands/hintTargets";
import { desktopCompositionActive } from "./desktop/commands/keyIntent";
import { isTopmostDialog, owningDialog } from "./desktop/commands/modalGuard";

/** Most recently opened active confirm hook sits at the top of the stack. */
const activeConfirmStack: symbol[] = [];

// A keyboard-shortcut keycap (Linear / Raycast style) shown after a modal
// button's label. Desktop-ONLY: use the canonical product surface instead of a
// pointer media query. An iPad with a trackpad remains the touch product, so it
// must not inherit Desktop keyboard promises. `aria-hidden` because the
// button's own label already names it.
export function Kbd(
  { keys, floating = false, variant = "modal", availability = "available" }: {
    keys: string;
    floating?: boolean;
    variant?: ShortcutKeycapVariant;
    availability?: ShortcutKeycapAvailability;
  },
): React.JSX.Element {
  const surface = useSurfaceProfile();
  if (surface.kind !== "desktop") return <></>;
  return (
    <ShortcutKeycap
      keyLabel={keys}
      variant={variant}
      availability={availability}
      sx={floating
        ? { position: "absolute", right: -5, bottom: -4, zIndex: 1 }
        : { ml: 0.75 }}
    />
  );
}

// While `open`, Command/Ctrl+Enter confirms the modal. Capture phase so it beats a
// focused editor's own Enter (which would otherwise insert a newline before the
// dialog saw the key). A bare Enter is deliberately consumed without confirming,
// including when a confirm button owns focus. Esc is intentionally NOT handled here —
// the owning Dialog, Popover, or non-modal Popper closes it, so wiring it again
// would double-fire; this hook only adds the affirmative key.
//
// Ignores auto-REPEAT (`e.repeat`): the force confirm is opened by HOLDING ⌘⏎,
// so the Enter is still down and repeating when the modal appears — without this
// guard it would self-confirm instantly. A repeat-free press (release, press
// again) is the deliberate confirm. Also skips while an IME is composing (so
// confirming a CJK candidate doesn't submit the modal).
export function useConfirmEnter(
  open: boolean,
  onConfirm: () => void,
  {
    suppressBareEnter = true,
    ignoreWithin,
    owner,
  }: {
    suppressBareEnter?: boolean;
    /** A region with its own Mod+Enter: keys from inside it pass through. */
    ignoreWithin?: string;
    /** Dialog or descendant element that owns this confirmation; stands aside
     *  whenever another modal dialog sits above it. */
    owner?: RefObject<Element | null>;
  } = {},
): void {
  const ref = useRef(onConfirm);
  ref.current = onConfirm;
  const ownerRef = useRef(owner);
  ownerRef.current = owner;
  useEffect(() => {
    if (!open) return undefined;
    const token = Symbol("confirm-enter");
    activeConfirmStack.push(token);
    const onKey = (e: KeyboardEvent): void => {
      if (desktopCompositionActive() || popupOwnsKeys()) return;
      const ownElement = ownerRef.current?.current ?? null;
      if (ownElement !== null) {
        if (!isTopmostDialog(ownElement)) return;
        const ownDialog = owningDialog(ownElement);
        const targetDialog = e.target instanceof Element ? owningDialog(e.target) : null;
        if (ownDialog && targetDialog && targetDialog !== ownDialog) return;
      } else if (activeConfirmStack.at(-1) !== token) {
        return;
      }
      if (ignoreWithin && e.target instanceof Element && e.target.closest(ignoreWithin)) return;
      const intent = confirmEnterIntent(e);
      if (intent === "ignore") return;
      if (intent === "suppress" && !suppressBareEnter) return;
      e.preventDefault();
      e.stopPropagation();
      if (intent === "confirm") ref.current();
    };
    globalThis.addEventListener("keydown", onKey, true);
    return (): void => {
      const idx = activeConfirmStack.lastIndexOf(token);
      if (idx >= 0) activeConfirmStack.splice(idx, 1);
      globalThis.removeEventListener("keydown", onKey, true);
    };
  }, [open, suppressBareEnter, ignoreWithin]);
}
