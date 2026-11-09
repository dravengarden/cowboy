// The `app` node (docs/widget-apps.md §3, §9). Only a published widget, on a
// client where apps are enabled, runs the app; the runtime loads lazily so
// widgets without apps never fetch it. Everywhere else the agent's
// `fallback` text stands in.

import { lazy, Suspense, useRef, useState } from "react";
import { Box, Typography } from "@mui/material";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import type { ParsedNode } from "./widgetDocument";
import { useWidgetVerified } from "./widgetMode";
import { appsEnabled } from "./apps/appsEnabled";
import { previewAppMessage, type AppMessageReply, type AppMessageTarget } from "./apps/appMessages";
import { useWidgetInteraction } from "./widgetInteractions";

const AppNode = lazy(() => import("./apps/AppNode"));
const AppMessageConfirm = lazy(() => import("./apps/AppMessageConfirm"));

/** At most one new confirmation per app every 5 s (docs/widget-apps.md §7). */
const CONFIRM_INTERVAL_MS = 5000;

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

/** Turns an app's send(text) into a previewed, product-confirmed message. */
function useAppSend(node: AppNodeData): {
  onSend: ((text: string) => void) | undefined;
  confirm: React.JSX.Element | null;
} {
  const interaction = useWidgetInteraction();
  const [preview, setPreview] = useState<Extract<AppMessageReply, { kind: "preview" }> | null>(
    null,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const last = useRef(0);
  if (!interaction || interaction.closed) return { onSend: undefined, confirm: null };
  const target: AppMessageTarget = {
    sessionId: interaction.sessionId,
    widgetId: interaction.widgetId,
    nodeId: node.id,
    code: node.code,
  };
  const onSend = (text: string): void => {
    const now = Date.now();
    if (preview || now - last.current < CONFIRM_INTERVAL_MS) return;
    last.current = now;
    void previewAppMessage(target, text).then((reply) => {
      if (reply.kind === "preview") setPreview(reply);
      else if (reply.kind === "refused") setNotice(reply.message);
    });
  };
  const confirm = preview ? (
    <Suspense fallback={null}>
      <AppMessageConfirm
        target={target}
        title={node.title}
        preview={preview}
        onClose={(): void => setPreview(null)}
      />
    </Suspense>
  ) : notice ? (
    <Typography variant="caption" color="warning.main" role="alert">
      {notice}
    </Typography>
  ) : null;
  return { onSend, confirm };
}

export function WidgetApp({ node }: { node: AppNodeData }): React.JSX.Element {
  const verified = useWidgetVerified();
  const send = useAppSend(node);
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
      <AppNode code={node.code} props={node.props ?? {}} title={node.title} onSend={send.onSend} />
      {send.confirm}
    </Suspense>
  );
}
