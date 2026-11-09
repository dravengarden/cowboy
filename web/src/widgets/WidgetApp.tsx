// The `app` node (docs/widget-apps.md §3, §9). Only a published widget, on a
// client where apps are enabled, runs the app; the runtime loads lazily so
// widgets without apps never fetch it. Everywhere else the agent's
// `fallback` text stands in.

import { lazy, Suspense } from "react";
import { Box, Typography } from "@mui/material";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import type { ParsedNode } from "./widgetDocument";
import { useWidgetVerified } from "./widgetMode";
import { appsEnabled } from "./apps/appsEnabled";

const AppNode = lazy(() => import("./apps/AppNode"));

type AppNodeData = Extract<ParsedNode, { type: "app" }>;

function Fallback({ node, note }: { node: AppNodeData; note: string | null }): React.JSX.Element {
  return (
    <Box
      data-widget-app-fallback=""
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        px: 1.5,
        py: 1,
        display: "grid",
        gap: 0.5,
      }}
    >
      <Typography variant="body2" sx={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
        {node.fallback}
      </Typography>
      {note ? (
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: "flex", gap: 0.5, alignItems: "center" }}
        >
          <WidgetsOutlined sx={{ fontSize: "1em" }} aria-hidden />
          {note}
        </Typography>
      ) : null}
    </Box>
  );
}

export function WidgetApp({ node }: { node: AppNodeData }): React.JSX.Element {
  const verified = useWidgetVerified();
  if (!verified) return <Fallback node={node} note={null} />;
  if (!appsEnabled()) {
    return (
      <Fallback
        node={node}
        note={`“${node.title}” is an app: open it on Desktop Chrome or update the Cowboy app.`}
      />
    );
  }
  return (
    <Suspense fallback={<Fallback node={node} note="Loading the app…" />}>
      <AppNode code={node.code} props={node.props ?? {}} title={node.title} />
    </Suspense>
  );
}
