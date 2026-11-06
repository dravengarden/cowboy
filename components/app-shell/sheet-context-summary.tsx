import { alpha, Box, Typography } from "@mui/material";
import type { ReactNode } from "react";
import type { SheetDockContext } from "./sheet-context-dock";

/** A compact information surface, independently reusable beside navigation. */
export function SheetContextSummary({ context }: { context: SheetDockContext }): ReactNode {
  return (
    <Box data-sheet-context-summary sx={{ width: "100%", maxWidth: 560, minWidth: 0, px: 1.25, py: 0.75, borderRadius: "1rem", bgcolor: theme => alpha(theme.palette.background.paper, 0.94), pointerEvents: "auto", maxHeight: "min(30dvh, 12rem)", overflowY: "auto", overscrollBehavior: "contain", scrollbarWidth: "thin" }}>
      <Box sx={{ display: "flex", alignItems: "baseline", flexWrap: "wrap", columnGap: 1, rowGap: 0.25 }}>
        <Typography sx={{ minWidth: 0, flex: "1 1 12rem", overflowWrap: "anywhere", fontSize: "1rem", lineHeight: 1.3, fontWeight: 600 }}>{context.title}</Typography>
        {(context.status || context.meta) && <Typography sx={{ fontSize: "0.8125rem", lineHeight: 1.4, color: "text.secondary", fontVariantNumeric: "tabular-nums" }}>
          <Box component="span" aria-live="polite" sx={{ color: context.tone && context.tone !== "neutral" ? `${context.tone}.main` : "inherit" }}>{context.status}</Box>{context.status && context.meta ? " · " : ""}{context.meta}
        </Typography>}
      </Box>
      {Boolean(context.facts?.length) && <Box data-context-facts role="group" aria-label="Context information" sx={{ display: "flex", flexWrap: "wrap", columnGap: 1.5, rowGap: 0.25, mt: 0.5, minWidth: 0 }}>
        {context.facts?.map(fact => <Typography key={fact.label} component="span" sx={{ minWidth: 0, overflowWrap: "anywhere", fontSize: "0.8125rem", lineHeight: 1.5, color: "text.secondary" }}>
          <Box component="span" sx={{ opacity: 0.7, mr: 0.5 }}>{fact.label}</Box>{fact.value}
        </Typography>)}
      </Box>}
    </Box>
  );
}
