import { Box, ButtonBase, Typography } from "@mui/material";
import ChevronRight from "@mui/icons-material/ChevronRight";
import type { ReactNode } from "react";
import type { SheetDockContext } from "./sheet-context-dock";

/** A compact information surface, independently reusable beside navigation. */
export function SheetContextSummary({ context }: { context: SheetDockContext }): ReactNode {
  return (
    <Box data-sheet-context-summary sx={{ display: "block", textAlign: "left", width: "100%", maxWidth: 560, minWidth: 0, borderRadius: "1rem", bgcolor: "background.paper", pointerEvents: "auto", maxHeight: "min(24dvh, 7rem)", overflowY: "auto", overscrollBehavior: "contain", scrollbarWidth: "thin" }}>
      <Box component={context.action ? ButtonBase : "div"} aria-label={context.action?.label} title={context.title} onClick={context.action?.onActivate} sx={{ display: "block", textAlign: "left", width: "100%", minWidth: 0, minHeight: context.action ? 44 : undefined, px: 1, py: 0.5 }}>
      <Box sx={{ display: "flex", width: "100%", alignItems: "center", textAlign: "left", gap: 0.25 }}>
        <Typography sx={{ minWidth: 0, flex: 1, overflowWrap: "anywhere", fontSize: "0.9375rem", lineHeight: 1.3, fontWeight: 600, display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 2, overflow: "hidden" }}>{context.title}</Typography>
        {context.action && <ChevronRight sx={{ fontSize: "1rem", flexShrink: 0, color: "text.secondary" }} />}
      </Box>
      {(context.status || context.meta || Boolean(context.facts?.length)) && <Box data-context-facts role="group" aria-label="Context information" sx={{ display: "flex", flexWrap: "wrap", columnGap: 1, rowGap: 0.125, minWidth: 0 }}>
        {(context.status || context.meta) && <Typography sx={{ fontSize: "0.8125rem", lineHeight: 1.4, color: "text.secondary", fontVariantNumeric: "tabular-nums" }}>
          <Box component="span" aria-live="polite" sx={{ color: context.tone && context.tone !== "neutral" ? `${context.tone}.main` : "inherit" }}>{context.status}</Box>{context.status && context.meta ? " · " : ""}{context.meta}
        </Typography>}
        {context.facts?.map(fact => <Typography key={fact.label} component="span" sx={{ minWidth: 0, overflowWrap: "anywhere", fontSize: "0.8125rem", lineHeight: 1.5, color: "text.secondary" }}>
          <Box component="span" sx={{ opacity: 0.7, mr: 0.5 }}>{fact.label}</Box>{fact.value}
        </Typography>)}
      </Box>}
      </Box>
    </Box>
  );
}
