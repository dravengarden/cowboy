import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Box, Typography } from "@mui/material";
import ArrowBackIosNewRounded from "@mui/icons-material/ArrowBackIosNewRounded";
import Close from "@mui/icons-material/Close";
import { FloatingActionIsland, IslandGlyphButton, type IslandAction } from "./bottom-sheet";

export interface SheetDockContext {
  title: string;
  status?: string | undefined;
  meta?: string | undefined;
  tone?: "neutral" | "success" | "warning" | "error" | undefined;
}

/** A reader's identity shares the navigation island, never a second overlay. */
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
    const measure = (): void => dialog.style.setProperty("--cowboy-sheet-dock-clearance", `${footer.getBoundingClientRect().height}px`);
    const observer = new ResizeObserver(measure);
    observer.observe(footer);
    measure();
    return () => { observer.disconnect(); dialog.style.removeProperty("--cowboy-sheet-dock-clearance"); };
  }, []);
  if (actions.length > 1) throw new Error("A context dock supports one contextual action");
  return (
    <Box ref={ref} data-mobile-sheet-footer-shield data-sheet-context-dock sx={{ width: "100%", display: "flex", justifyContent: "center", pointerEvents: "none" }}>
      <FloatingActionIsland columns={`${onBack ? "46px " : ""}minmax(0, 1fr) ${actions.map(() => "46px").join(" ")} 46px`} maxWidth={560} minHeight={56}>
        {onBack && <IslandGlyphButton label="Back" onActivate={onBack}><ArrowBackIosNewRounded fontSize="small" /></IslandGlyphButton>}
        <Box sx={{ minWidth: 0, px: 1, py: 0.5 }} title={context.title}>
          <Typography noWrap sx={{ fontSize: "0.8125rem", fontWeight: 600 }}>{context.title}</Typography>
          <Typography noWrap sx={{ fontSize: "0.6875rem", color: "text.secondary", fontVariantNumeric: "tabular-nums" }}>
            <Box component="span" aria-live="polite" sx={{ color: context.tone && context.tone !== "neutral" ? `${context.tone}.main` : "inherit" }}>{context.status}</Box>{context.status && context.meta ? " · " : ""}{context.meta}
          </Typography>
        </Box>
        {actions.map(action => <IslandGlyphButton key={action.label} label={action.label} onActivate={action.onActivate}>{action.icon}</IslandGlyphButton>)}
        <IslandGlyphButton label="Close" onActivate={onClose}><Close fontSize="small" /></IslandGlyphButton>
      </FloatingActionIsland>
    </Box>
  );
}
