import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Box, ButtonBase } from "@mui/material";
import { SheetContextSummary } from "./sheet-context-summary";
import ArrowBackIosNewRounded from "@mui/icons-material/ArrowBackIosNewRounded";
import Close from "@mui/icons-material/Close";
import { FloatingActionIsland, IslandGlyphButton, type IslandAction } from "./bottom-sheet";

export interface SheetDockContext {
  title: string;
  status?: string | undefined;
  meta?: string | undefined;
  tone?: "neutral" | "success" | "warning" | "error" | undefined;
  action?: { label: string; onActivate: () => void } | undefined;
  /** Supporting facts wrap to use the available width. */
  facts?: readonly { label: string; value: string }[] | undefined;
}

/** A shared footer composes an information surface and independent round controls. */
export function MobileSheetContextDock({ context, onBack, onClose, actions = [] }: {
  context: SheetDockContext;
  onBack?: (() => void) | undefined;
  onClose: () => void;
  actions?: readonly IslandAction[] | undefined;
}): ReactNode {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const footer = ref.current?.closest<HTMLElement>("[data-detent-sheet-footer]");
    const dialog = ref.current?.closest<HTMLElement>('[role="dialog"]');
    if (!footer || !dialog) return;
    dialog.style.setProperty("--cowboy-sheet-footer-gap", "4px");
    const measure = (): void => dialog.style.setProperty("--cowboy-sheet-dock-clearance", `${footer.getBoundingClientRect().height}px`);
    const observer = new ResizeObserver(measure);
    observer.observe(footer);
    measure();
    return () => { observer.disconnect(); dialog.style.removeProperty("--cowboy-sheet-dock-clearance"); dialog.style.removeProperty("--cowboy-sheet-footer-gap"); };
  }, []);
  const inlineActions = actions.length === 1 ? actions : [];
  return (
    <Box ref={ref} data-mobile-sheet-footer-shield data-sheet-context-dock sx={{ width: "100%", maxWidth: 560, display: "grid", gridTemplateColumns: "auto minmax(0, 1fr)", alignItems: "center", gap: 0.75, pointerEvents: "none" }}>
      <FloatingActionIsland columns={`repeat(${(onBack ? 1 : 0) + inlineActions.length + 1}, 46px)`} maxWidth={(onBack ? 50 : 0) + inlineActions.length * 50 + 56} minHeight={56}>
        {onBack && <IslandGlyphButton label="Back" onActivate={onBack}><ArrowBackIosNewRounded fontSize="small" /></IslandGlyphButton>}
        {inlineActions.map(action => <IslandGlyphButton key={action.label} label={action.label} onActivate={action.onActivate}>{action.icon}</IslandGlyphButton>)}
        <IslandGlyphButton label="Close" onActivate={onClose}><Close fontSize="small" /></IslandGlyphButton>
      </FloatingActionIsland>
      <SheetContextSummary context={context} />
        {actions.length > 1 && <Box data-context-actions role="toolbar" aria-label="Context actions" sx={{ gridColumn: "1 / -1", width: "100%", maxWidth: 560, minWidth: 0, display: "flex", overflowX: "auto", px: 1, pointerEvents: "auto", bgcolor: "background.paper", borderRadius: "1rem" }}>
          {actions.map(action => <ButtonBase key={action.label} onClick={action.onActivate} sx={{ minHeight: 44, flexShrink: 0, gap: 0.5, px: 1, fontSize: "0.8125rem", borderRadius: 1 }}>{action.icon}{action.label}</ButtonBase>)}
        </Box>}
    </Box>
  );
}
