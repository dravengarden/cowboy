import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Box, ButtonBase, Typography } from "@mui/material";
import ChevronRight from "@mui/icons-material/ChevronRight";
import ArrowBackIosNewRounded from "@mui/icons-material/ArrowBackIosNewRounded";
import Close from "@mui/icons-material/Close";
import { FloatingActionIsland, IslandGlyphButton, type IslandAction } from "./bottom-sheet";

export interface SheetDockContext {
  title: string;
  status?: string | undefined;
  meta?: string | undefined;
  tone?: "neutral" | "success" | "warning" | "error" | undefined;
  action?: { label: string; onActivate: () => void } | undefined;
  /** Always-visible supporting facts; overflow stays inside a horizontal rail. */
  facts?: readonly { label: string; value: string }[] | undefined;
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
  const inlineActions = actions.length === 1 ? actions : [];
  return (
    <Box ref={ref} data-mobile-sheet-footer-shield data-sheet-context-dock sx={{ width: "100%", display: "flex", justifyContent: "center", pointerEvents: "none" }}>
      <FloatingActionIsland compact rim="soft" columns={`${onBack ? "46px " : ""}minmax(0, 1fr) ${inlineActions.map(() => "46px").join(" ")} 46px`} maxWidth={560} minHeight={48}>
        {onBack && <IslandGlyphButton label="Back" onActivate={onBack}><ArrowBackIosNewRounded fontSize="small" /></IslandGlyphButton>}
        <Box component={context.action ? ButtonBase : "div"} onClick={context.action?.onActivate} aria-label={context.action ? `${context.action.label}: ${context.title}` : undefined} data-context-action={context.action?.label} sx={{ minWidth: 0, minHeight: 46, display: "flex", gap: 0.25, textAlign: "left", px: 0.5, py: 0.25, borderRadius: 1, "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main" } }} title={context.title}>
          <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography noWrap sx={{ fontSize: "1rem", lineHeight: 1.3, fontWeight: 600 }}>{context.title}</Typography>
          <Typography noWrap sx={{ fontSize: "0.8125rem", lineHeight: 1.3, color: "text.secondary", fontVariantNumeric: "tabular-nums" }}>
            <Box component="span" aria-live="polite" sx={{ color: context.tone && context.tone !== "neutral" ? `${context.tone}.main` : "inherit" }}>{context.status}</Box>{context.status && context.meta ? " · " : ""}{context.meta}
          </Typography>
          </Box>
          {context.action && <ChevronRight sx={{ fontSize: "1rem", flexShrink: 0, color: "text.secondary" }} />}
        </Box>
        {inlineActions.map(action => <IslandGlyphButton key={action.label} label={action.label} onActivate={action.onActivate}>{action.icon}</IslandGlyphButton>)}
        <IslandGlyphButton label="Close" onActivate={onClose}><Close fontSize="small" /></IslandGlyphButton>
        {Boolean(context.facts?.length) && <Box data-context-facts role="group" aria-label="Context information" tabIndex={0} sx={{ gridColumn: "1 / -1", minWidth: 0, display: "flex", gap: 1.25, overflowX: "auto", overscrollBehaviorX: "contain", px: 1.5, pb: 0.75, scrollbarWidth: "thin" }}>
          {context.facts?.map(fact => <Typography key={fact.label} component="span" title={`${fact.label}: ${fact.value}`} aria-label={`${fact.label}: ${fact.value}`} sx={{ flexShrink: 0, whiteSpace: "nowrap", fontSize: "0.8125rem", lineHeight: 1.5, color: "text.secondary" }}>
            <Box component="span" sx={{ opacity: 0.7, mr: 0.5 }}>{fact.label}</Box>{fact.value}
          </Typography>)}
        </Box>}
        {actions.length > 1 && <Box data-context-actions role="toolbar" aria-label="Context actions" sx={{ gridColumn: "1 / -1", minWidth: 0, display: "flex", overflowX: "auto", px: 1 }}>
          {actions.map(action => <ButtonBase key={action.label} onClick={action.onActivate} sx={{ minHeight: 44, flexShrink: 0, gap: 0.5, px: 1, fontSize: "0.8125rem", borderRadius: 1 }}>{action.icon}{action.label}</ButtonBase>)}
        </Box>}
      </FloatingActionIsland>
    </Box>
  );
}
