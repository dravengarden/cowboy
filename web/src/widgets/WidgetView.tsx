// One widget: header (title, summary, handle, age, Open) and its body within
// the inline height budget (docs/widgets.md §7.1). **Open** shows the whole
// widget in the inspector. Lazy-loaded by the fence and notice dispatchers.

import { lazy, Suspense, useContext, useState } from "react";
import { Box, IconButton, Paper, Tooltip, Typography } from "@mui/material";
import { StatusChip } from "../StatusChip";
import OpenInFullRounded from "@mui/icons-material/OpenInFullRounded";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import { Collapsible } from "../tools/Collapsible";
import { relativeAge } from "../syncStatus";
import { shortcutKey } from "../desktop/commands/imeShortcut";
import { SurfaceContext } from "../surface/SurfaceProfile";
import type { ParsedDocument } from "./widgetDocument";
import { WidgetNode } from "./WidgetNodes";
import { useWidgetSorts, WidgetSortContext, WidgetVerifiedContext } from "./widgetMode";
import { WidgetInteractionContext, type WidgetInteraction } from "./widgetInteractions";

const WidgetInspector = lazy(() => import("./WidgetInspector"));

const HEIGHT = { xs: 160, sm: 240, md: 400, lg: 600 } as const;

export type WidgetViewProps = {
  document: ParsedDocument;
  /** Published widgets show their public handle (the name taps cite). */
  handle?: string | undefined;
  closed?: boolean | undefined;
  /** When the published widget last changed. */
  updatedAtMs?: number | undefined;
  /** Published through the Controller, which verified its image sources. */
  verified?: boolean | undefined;
  /** A published widget's inputs act through the Controller. */
  interaction?: WidgetInteraction | undefined;
};

function Header({
  document,
  handle,
  closed,
  updatedAtMs,
  open,
  onOpen,
}: WidgetViewProps & { open: boolean; onOpen: () => void }): React.JSX.Element {
  const age = relativeAge(updatedAtMs, Date.now());
  // Desktop: the title is the row's conversation toggle, like tool and task
  // rows. Tab selects it; Enter or `l` opens the inspector, whose modal
  // grammar operates the controls; Esc returns focus here.
  const desktop = useContext(SurfaceContext)?.kind === "desktop";
  const toggle = desktop
    ? {
        role: "button",
        tabIndex: -1,
        "data-desktop-widget-toggle": "widget",
        "aria-expanded": open,
        "aria-haspopup": "dialog" as const,
        "aria-label": `${document.title}. Open widget`,
        onClick: onOpen,
        onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>): void => {
          const key = shortcutKey(event.nativeEvent);
          if (key === "Enter" || key === " ") {
            event.preventDefault();
            onOpen();
          }
        },
      }
    : {};
  const mono = { fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" } as const;
  return (
    <Box sx={{ display: "flex", gap: 1, alignItems: "flex-start", minWidth: 0 }}>
      <WidgetsOutlined fontSize="small" sx={{ color: "text.secondary", mt: "0.1em" }} aria-hidden />
      <Box
        {...toggle}
        sx={{
          minWidth: 0,
          flex: "1 1 auto",
          ...(desktop
            ? {
                cursor: "pointer",
                borderRadius: 1,
                mx: -0.5,
                px: 0.5,
                outline: "none",
                "&:focus-visible": { bgcolor: "action.focus" },
              }
            : {}),
        }}
      >
        <Typography variant="subtitle2" sx={{ fontWeight: 600, overflowWrap: "anywhere" }}>
          {document.title}
        </Typography>
        {document.summary ? (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", overflowWrap: "anywhere" }}
          >
            {document.summary}
          </Typography>
        ) : null}
      </Box>
      {/* Centre the controls on the title's first line, whatever the target size. */}
      <Box
        sx={{
          display: "flex",
          gap: 0.75,
          alignItems: "center",
          flexShrink: 0,
          mt: "-6px",
          "@media (pointer: coarse)": { mt: "-11px" },
        }}
      >
        {handle ? (
          <Typography variant="caption" color="text.secondary" sx={mono}>
            {`#${handle}`}
          </Typography>
        ) : null}
        {age && updatedAtMs !== undefined ? (
          <Typography
            variant="caption"
            color="text.secondary"
            title={new Date(updatedAtMs).toLocaleString()}
            sx={{ display: { xs: "none", sm: "inline" } }}
          >
            {age}
          </Typography>
        ) : null}
        {closed ? <StatusChip label="closed" /> : null}
        <Tooltip title="Open">
          <IconButton
            size="small"
            aria-label={`Open ${document.title}`}
            data-widget-open=""
            onClick={onOpen}
            // Touch keeps the 44 px target (§7.3).
            sx={{ "@media (pointer: coarse)": { minWidth: 44, minHeight: 44 } }}
          >
            <OpenInFullRounded fontSize="small" />
          </IconButton>
        </Tooltip>
      </Box>
    </Box>
  );
}

export default function WidgetView(props: WidgetViewProps): React.JSX.Element {
  const { document } = props;
  const [open, setOpen] = useState(false);
  const sorts = useWidgetSorts();
  const body = (
    <Box sx={{ display: "grid", gap: 1.25, minWidth: 0 }}>
      <WidgetNode node={document.root} />
    </Box>
  );
  return (
    <WidgetVerifiedContext.Provider value={props.verified === true}>
      <WidgetInteractionContext.Provider value={props.interaction ?? null}>
        <WidgetSortContext.Provider value={sorts}>
          <Paper
            variant="outlined"
            data-cowboy-widget=""
            sx={{
              my: 1,
              px: 1.5,
              py: 1.25,
              display: "grid",
              gap: 1,
              minWidth: 0,
              bgcolor: "background.paper",
              contain: "layout paint",
            }}
          >
            <Header {...props} open={open} onOpen={(): void => setOpen(true)} />
            {document.presentation?.inline === false ? (
              body
            ) : (
              <Collapsible maxHeight={HEIGHT[document.presentation?.height ?? "md"]}>
                {body}
              </Collapsible>
            )}
            {open ? (
              <Suspense fallback={null}>
                <WidgetInspector {...props} onClose={(): void => setOpen(false)} />
              </Suspense>
            ) : null}
          </Paper>
        </WidgetSortContext.Provider>
      </WidgetInteractionContext.Provider>
    </WidgetVerifiedContext.Provider>
  );
}
