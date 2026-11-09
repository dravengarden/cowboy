// The `app` node (docs/widget-apps.md §3, §7, §9). Only an open, published
// widget, on a client where apps are enabled, runs the app; the runtime loads
// lazily so widgets without apps never fetch it. Everywhere else the agent's
// `fallback` text stands in. An app's send(text) becomes a product
// confirmation of that exact text before anything leaves the device.

import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Box, Typography } from "@mui/material";
import WidgetsOutlined from "@mui/icons-material/WidgetsOutlined";
import type { ParsedNode } from "./widgetDocument";
import { useWidgetVerified } from "./widgetMode";
import { appsEnabled } from "./apps/appsEnabled";
import {
  appMessageProblem,
  pendingAppMessages,
  prepareAppMessage,
  type AppMessageTarget,
  type PendingAppMessage,
} from "./apps/appMessages";
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

/** Turns an app's send(text) into a confirmation; one at a time per app. */
function useAppSend(node: AppNodeData, target: AppMessageTarget | null) {
  const [pending, setPending] = useState<PendingAppMessage | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Reserved synchronously, so a request that arrives while one is being
  // prepared or shown is refused instead of replacing it.
  const slot = useRef(false);
  const last = useRef(0);
  useEffect(() => {
    if (!target) return;
    const [uncertain] = pendingAppMessages(target.sessionId, target.widgetId, target.nodeId).filter(
      (entry) => entry.state === "sending",
    );
    if (uncertain && !slot.current) {
      slot.current = true;
      setPending(uncertain);
    }
  }, [target?.sessionId, target?.widgetId, target?.nodeId]);
  const onSend = target
    ? (text: string): void => {
        const now = Date.now();
        if (slot.current || now - last.current < CONFIRM_INTERVAL_MS) return;
        const problem = appMessageProblem(text);
        if (problem) {
          setNotice(problem);
          return;
        }
        slot.current = true;
        last.current = now;
        void prepareAppMessage(target, text).then(setPending, () => {
          slot.current = false;
        });
      }
    : undefined;
  const close = (): void => {
    slot.current = false;
    setPending(null);
  };
  const confirm = pending ? (
    <Suspense fallback={null}>
      <AppMessageConfirm pending={pending} title={node.title} onClose={close} />
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
  const interaction = useWidgetInteraction();
  const target: AppMessageTarget | null =
    interaction && !interaction.closed
      ? {
          sessionId: interaction.sessionId,
          widgetId: interaction.widgetId,
          nodeId: node.id,
          code: node.code,
        }
      : null;
  const send = useAppSend(node, target);
  if (!verified) return <Fallback node={node} note={null} />;
  if (interaction?.closed)
    return <Fallback node={node} note={`“${node.title}” was closed by the agent.`} />;
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
      <AppNode
        code={node.code}
        props={node.props ?? {}}
        title={node.title}
        identity={target ? `${target.sessionId}:${target.widgetId}:${node.id}` : null}
        onSend={send.onSend}
      />
      {send.confirm}
    </Suspense>
  );
}
