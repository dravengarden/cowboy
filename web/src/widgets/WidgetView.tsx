// One widget: header (title, summary) and its body within the inline height
// budget (docs/widgets.md §7.1). Lazy-loaded by the fence dispatcher.

import { Box, Paper, Typography } from "@mui/material";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import { Collapsible } from "../tools/Collapsible";
import type { ParsedDocument } from "./widgetDocument";
import { WidgetNode } from "./WidgetNodes";

const HEIGHT = { xs: 160, sm: 240, md: 400, lg: 600 } as const;

export default function WidgetView({ document }: { document: ParsedDocument }): React.JSX.Element {
  const body = (
    <Box sx={{ display: "grid", gap: 1.25, minWidth: 0 }}>
      <WidgetNode node={document.root} />
    </Box>
  );
  return (
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
      <Box sx={{ display: "flex", gap: 1, alignItems: "flex-start", minWidth: 0 }}>
        <WidgetsOutlined
          fontSize="small"
          sx={{ color: "text.secondary", mt: "0.1em" }}
          aria-hidden
        />
        <Box sx={{ minWidth: 0 }}>
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
      </Box>
      {document.presentation?.inline === false ? (
        body
      ) : (
        <Collapsible maxHeight={HEIGHT[document.presentation?.height ?? "md"]}>{body}</Collapsible>
      )}
    </Paper>
  );
}
