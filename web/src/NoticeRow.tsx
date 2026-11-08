// One `notice` transcript row: a plain status line, or a published widget
// (docs/widgets.md §6.2) drawn by the widget renderer. A widget document that
// this client cannot validate falls back to the plain line the Controller
// projected for older clients.

import { lazy, Suspense, useMemo } from "react";
import { Box } from "@mui/material";
import { ErrorOutline, InfoOutlined, WarningAmberRounded } from "@mui/icons-material";
import type { RenderItem } from "./derive";
import { LIFECYCLE_ICON_SX, LIFECYCLE_ROW_SX } from "./TranscriptItems";
import { parseWidgetValue } from "./widgets/widgetDocument";

const WidgetView = lazy(() => import("./widgets/WidgetView"));

type Notice = Extract<RenderItem, { kind: "notice" }>;

function PlainNotice({ item }: { item: Notice }): React.JSX.Element {
  const color =
    item.severity === "error"
      ? "error.main"
      : item.severity === "warning"
        ? "warning.main"
        : "text.secondary";
  return (
    <Box role="status" sx={{ ...LIFECYCLE_ROW_SX, color }}>
      {item.severity === "error" ? (
        <ErrorOutline sx={LIFECYCLE_ICON_SX} />
      ) : item.severity === "warning" ? (
        <WarningAmberRounded sx={LIFECYCLE_ICON_SX} />
      ) : (
        <InfoOutlined sx={LIFECYCLE_ICON_SX} />
      )}
      <Box sx={{ minWidth: 0, overflowWrap: "anywhere" }}>
        <Box component="span" sx={{ fontWeight: 600 }}>
          {item.title}
        </Box>
        {item.description && (
          <Box sx={{ color: "text.secondary", whiteSpace: "pre-wrap" }}>{item.description}</Box>
        )}
      </Box>
    </Box>
  );
}

export function NoticeRow({ item }: { item: Notice }): React.JSX.Element {
  const widget = item.widget;
  const parsed = useMemo(() => (widget ? parseWidgetValue(widget.document) : null), [widget]);
  if (!widget || !parsed?.ok) return <PlainNotice item={item} />;
  return (
    <Box data-widget-row={widget.handle} data-widget-state={widget.stateVersion}>
      <Suspense fallback={<PlainNotice item={item} />}>
        <WidgetView document={parsed.document} handle={widget.handle} closed={widget.closed} />
      </Suspense>
    </Box>
  );
}
