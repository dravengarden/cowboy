import type React from "react";
import { useEffect, useRef } from "react";
import { ShortcutKeycap, type ShortcutKeycapVariant } from "./ShortcutKeycap";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { isImeKeyEvent } from "./imeKey";

// A keyboard-shortcut keycap (Linear / Raycast style) shown after a modal
// button's label. Desktop-ONLY: use the canonical product surface instead of a
// pointer media query. An iPad with a trackpad remains the touch product, so it
// must not inherit Desktop keyboard promises. `aria-hidden` because the
// button's own label already names it.
export function Kbd(
  { keys, floating = false, variant = "modal" }: {
    keys: string;
    floating?: boolean;
    variant?: ShortcutKeycapVariant;
  },
): React.JSX.Element {
  const surface = useSurfaceProfile();
  if (surface.kind !== "desktop") return <></>;
  return (
    <ShortcutKeycap
      keyLabel={keys}
      variant={variant}
      sx={floating
        ? { position: "absolute", right: -5, bottom: -4, zIndex: 1 }
        : { ml: 0.75 }}
    />
  );
}

// While `open`, a bare Enter confirms the modal. Capture phase so it beats a
// focused editor's own Enter (which would otherwise insert a newline before the
// dialog saw the key). Esc is intentionally NOT handled here — MUI's Dialog /
// Popover already close on Escape via their `onClose`, so wiring it again would
// double-fire; this hook only adds the affirmative key.
//
// Ignores auto-REPEAT (`e.repeat`): the force confirm is opened by HOLDING ⌘⏎,
// so the Enter is still down and repeating when the modal appears — without this
// guard it would self-confirm instantly. A repeat-free press (release, press
// again) is the deliberate confirm. Also skips while an IME is composing (so
// confirming a CJK candidate doesn't submit the modal).
export function useConfirmEnter(open: boolean, onConfirm: () => void): void {
  const ref = useRef(onConfirm);
  ref.current = onConfirm;
  useEffect(() => {
    if (!open) return undefined;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== "Enter" || e.shiftKey || e.repeat || isImeKeyEvent(e)) return;
      e.preventDefault();
      e.stopPropagation();
      ref.current();
    };
    globalThis.addEventListener("keydown", onKey, true);
    return (): void => globalThis.removeEventListener("keydown", onKey, true);
  }, [open]);
}
