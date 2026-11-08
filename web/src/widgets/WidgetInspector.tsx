// The widget inspector (docs/widgets.md §7.1, §7.4, §7.5): the whole widget,
// no height fold and no preview bounds, with long tables paged. Desktop gets
// a modal (Esc closes and MUI restores focus to Open); phone and iPad get the
// cover sheet, a wide sheet on iPad.

import { useContext } from "react";
import { Box, Typography } from "@mui/material";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import { DesktopModal } from "../desktop/DesktopModal";
import { Sheet } from "../Sheet";
import { SurfaceContext } from "../surface/SurfaceProfile";
import { WidgetModeContext } from "./widgetMode";
import { WidgetNode } from "./WidgetNodes";
import type { WidgetViewProps } from "./WidgetView";

export default function WidgetInspector({
  document,
  handle,
  onClose,
}: WidgetViewProps & { onClose: () => void }): React.JSX.Element {
  const surface = useContext(SurfaceContext);
  const title = handle ? `${document.title} · #${handle}` : document.title;
  const body = (
    <WidgetModeContext.Provider value="inspector">
      <Box data-widget-inspector="" sx={{ display: "grid", gap: 1.5, minWidth: 0 }}>
        <WidgetNode node={document.root} />
      </Box>
    </WidgetModeContext.Provider>
  );
  // Outside a surface provider (fixtures, embedded hosts) the modal is the safe default.
  if (!surface || surface.kind === "desktop") {
    return (
      <DesktopModal
        open
        onClose={onClose}
        title={title}
        {...(document.summary ? { description: document.summary } : {})}
        icon={<WidgetsOutlined fontSize="small" sx={{ color: "text.secondary" }} />}
      >
        <Box sx={{ px: 2.25, py: 2 }}>{body}</Box>
      </DesktopModal>
    );
  }
  return (
    <Sheet open onClose={onClose} title={title} forceSheet cover portal wide>
      <Box sx={{ px: 1.5, pb: 3 }}>
        {document.summary ? (
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            {document.summary}
          </Typography>
        ) : null}
        {body}
      </Box>
    </Sheet>
  );
}
