import { createContext, type ReactNode, useContext, useEffect, useRef, useState } from "react";
import { Box, IconButton } from "@mui/material";
import { KeyboardHideOutlined } from "@mui/icons-material";
import { useKeyboardOpen } from "./keyboardInset";
import { SurfaceContext } from "./surface/SurfaceProfile";

const KeyboardActionProvided = createContext(false);

export function SheetActionKeyboardScope({ children }: { children: ReactNode }): React.JSX.Element {
  return <><KeyboardActionProvided.Provider value>{children}</KeyboardActionProvided.Provider><SheetKeyboardDismiss /></>;
}

/** Trailing form utility. Scope to the focused dialog so stacked sheets never
 * offer to dismiss another sheet's editor. Pointer-down retains the responder
 * until click; otherwise the keyboard can move this target before activation. */
export function SheetKeyboardDismiss(props: { keyboardOpen?: boolean | undefined; standalone?: boolean }): React.JSX.Element | null {
  const provided = useContext(KeyboardActionProvided);
  return provided ? null : <KeyboardAction {...props} />;
}

function KeyboardAction({ keyboardOpen, standalone = false }: { keyboardOpen?: boolean | undefined; standalone?: boolean }): React.JSX.Element {
  const inferredOpen = useKeyboardOpen();
  const surface = useContext(SurfaceContext);
  const anchor = useRef<HTMLSpanElement>(null);
  const [ownsInput, setOwnsInput] = useState(false);
  useEffect(() => {
    let disposed = false;
    const update = () => {
      if (disposed) return;
      const active = document.activeElement;
      if (active && anchor.current?.contains(active)) return;
      const dialog = anchor.current?.closest('[role="dialog"]');
      const editable = active instanceof HTMLElement &&
        (active.isContentEditable || active.matches('textarea:not([readonly]):not([disabled]), input:not([readonly]):not([disabled]):not([type="checkbox"]):not([type="radio"]):not([type="button"]):not([type="submit"]):not([type="range"]):not([type="file"]):not([type="color"]):not([inputmode="none"])'));
      setOwnsInput(Boolean(editable && dialog && active?.closest('[role="dialog"]') === dialog));
    };
    update();
    const onBlur = () => queueMicrotask(update);
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", onBlur);
    return () => {
      disposed = true;
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", onBlur);
    };
  }, []);
  const visible = surface?.kind !== "desktop" && (keyboardOpen ?? inferredOpen) && ownsInput;
  return (
    <Box component="span" ref={anchor} sx={{
      display: visible ? "inline-flex" : "none", flexShrink: 0, alignItems: "center",
      ...(standalone ? {
        justifyContent: "flex-end", px: 2.25, pt: 0.5,
        pb: "max(8px, calc(env(safe-area-inset-bottom, 0px) - var(--kb-inset, 0px)))",
        borderTop: 1, borderColor: "divider", bgcolor: "background.default",
      } : {}),
    }}>
      <IconButton
        aria-label="Hide keyboard"
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => {
          const active = document.activeElement;
          if (active instanceof HTMLElement && active.closest('[role="dialog"]') === anchor.current?.closest('[role="dialog"]')) active.blur();
        }}
        sx={{
          width: "max(44px, 2.75rem)", height: "max(44px, 2.75rem)",
          borderRadius: 2, color: "text.secondary",
          "&.Mui-focusVisible": { outline: "2px solid currentColor", outlineOffset: 2 },
        }}
      >
        <KeyboardHideOutlined sx={{ fontSize: "1.5rem" }} />
      </IconButton>
    </Box>
  );
}
