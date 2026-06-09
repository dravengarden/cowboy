import { Box } from "@mui/material";
import type React from "react";
import { useEffect, useRef } from "react";

// A keyboard-shortcut keycap (Linear / Raycast style) shown after a modal
// button's label. Desktop-ONLY: hidden unless the device has a fine pointer +
// hover (a real mouse/trackpad, which implies a physical keyboard). A phone or
// touch tablet — where there's no key to press — never shows it, so the hint is
// never a lie. `aria-hidden` because the button's own label already names it.
export function Kbd({ keys }: { keys: string }): React.JSX.Element {
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        display: "none",
        "@media (hover: hover) and (pointer: fine)": { display: "inline-flex" },
        alignItems: "center",
        justifyContent: "center",
        ml: 0.75,
        px: 0.5,
        minWidth: 17,
        height: 17,
        borderRadius: 0.75,
        border: 1,
        borderColor: "divider",
        bgcolor: "action.hover",
        color: "text.secondary",
        fontSize: 11,
        fontWeight: 600,
        lineHeight: 1,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      }}
    >
      {keys}
    </Box>
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
      if (e.key !== "Enter" || e.shiftKey || e.repeat || e.isComposing) return;
      e.preventDefault();
      e.stopPropagation();
      ref.current();
    };
    globalThis.addEventListener("keydown", onKey, true);
    return (): void => globalThis.removeEventListener("keydown", onKey, true);
  }, [open]);
}
