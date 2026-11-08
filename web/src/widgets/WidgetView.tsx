// One widget: header (title, summary, handle, age, Open) and its body within
// the inline height budget (docs/widgets.md §7.1). **Open** shows the whole
// widget in the inspector. Lazy-loaded by the fence and notice dispatchers.

import { lazy, Suspense, useState } from "react";
import { Box, IconButton, Paper, Tooltip, Typography } from "@mui/material";
import { StatusChip } from "../StatusChip";
import OpenInFullRounded from "@mui/icons-material/OpenInFullRounded";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import { Collapsible } from "../tools/Collapsible";
import { relativeAge } from "../syncStatus";
import type { ParsedDocument } from "./widgetDocument";
import { WidgetNode } from "./WidgetNodes";
import { useWidgetSorts, WidgetSortContext } from "./widgetMode";

const WidgetInspector = lazy(() => import("./WidgetInspector"));

const HEIGHT = { xs: 160, sm: 240, md: 400, lg: 600 } as const;

export type WidgetViewProps = {
  document: ParsedDocument;
  /** Published widgets show their public handle (the name taps cite). */
  handle?: string | undefined;
  closed?: boolean | undefined;
  /** When the published widget last changed. */
  updatedAtMs?: number | undefined;
};

function Header({
  document,
  handle,
  closed,
  updatedAtMs,
  onOpen,
}: WidgetViewProps & { onOpen: () => void }): React.JSX.Element {
  const age = relativeAge(updatedAtMs, Date.now());
  const mono = { fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" } as const;
  return (
    <Box sx={{ display: "flex", gap: 1, alignItems: "flex-start", minWidth: 0 }}>
      <WidgetsOutlined fontSize="small" sx={{ color: "text.secondary", mt: "0.1em" }} aria-hidden />
      <Box sx={{ minWidth: 0, flex: "1 1 auto" }}>
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
      <Header {...props} onOpen={(): void => setOpen(true)} />
      {document.presentation?.inline === false ? (
        body
      ) : (
        <Collapsible maxHeight={HEIGHT[document.presentation?.height ?? "md"]}>{body}</Collapsible>
      )}
      {open ? (
        <Suspense fallback={null}>
          <WidgetInspector {...props} onClose={(): void => setOpen(false)} />
        </Suspense>
      ) : null}
    </Paper>
    </WidgetSortContext.Provider>
  );
}
