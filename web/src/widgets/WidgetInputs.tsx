// `button` and `choice` nodes (docs/widgets.md §5.2, §8). In a published
// widget a `prompt` button or a choice's Send asks the Controller for the
// exact message, shows it for confirmation and queues it as the person's
// message; a decided input shows what was sent and stays inert. `copy` and
// `open` act locally. Inline fences have no interaction context, so their
// decision inputs stay inert.

import { lazy, Suspense, useState } from "react";
import { Box, Button, Checkbox, FormControlLabel, Radio, Typography } from "@mui/material";
import CheckRounded from "@mui/icons-material/CheckRounded";
import { WIDGET_LIMITS } from "../../../contracts/widget-v1.ts";
import { openExternalUrl, safeExternalUrl } from "../openExternal";
import type { ParsedNode } from "./widgetDocument";
import type { ConfirmTarget } from "./WidgetConfirm";
import { requestInteraction, useWidgetInteraction } from "./widgetInteractions";

const WidgetConfirm = lazy(() => import("./WidgetConfirm"));

type Of<T extends ParsedNode["type"]> = Extract<ParsedNode, { type: T }>;

/** Preview, confirm and send one decision of input `nodeId`. */
function useDecision(nodeId: string, agentNote: string | undefined, destructive: boolean) {
  const interaction = useWidgetInteraction();
  const [target, setTarget] = useState<ConfirmTarget | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const decided = interaction?.decided[nodeId];
  const live = interaction !== null && !interaction.closed && !decided && !sent;
  const start = async (values: string[]): Promise<void> => {
    if (!interaction || !live) return;
    setPending(true);
    setError(null);
    const request = { revision: interaction.revision, node_id: nodeId, values };
    const reply = await requestInteraction(interaction.sessionId, interaction.widgetId, request);
    setPending(false);
    if (reply.kind === "preview" || reply.kind === "changed") {
      setTarget({
        sessionId: interaction.sessionId,
        widgetId: interaction.widgetId,
        request,
        message: reply.message,
        messageSha256: reply.messageSha256,
        agentNote,
        destructive,
      });
    } else if (reply.kind === "refused") {
      setError(reply.message);
    }
  };
  const confirm = target ? (
    <Suspense fallback={null}>
      <WidgetConfirm
        target={target}
        onClose={(done): void => {
          setTarget(null);
          if (done) setSent(true);
        }}
      />
    </Suspense>
  ) : null;
  return { interaction, decided, sent, live, pending, error, start, confirm };
}

function SentNote({ labels }: { labels: readonly string[] | null }): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="success.main"
      data-widget-sent=""
      sx={{ display: "inline-flex", gap: 0.5, alignItems: "center" }}
    >
      <CheckRounded sx={{ fontSize: "1.1em" }} aria-hidden />
      {labels && labels.length > 0 ? `Sent: ${labels.join("; ")}` : "Sent"}
    </Typography>
  );
}

function ErrorNote({ text }: { text: string | null }): React.JSX.Element | null {
  return text ? (
    <Typography variant="caption" color="error.main" role="alert">
      {text}
    </Typography>
  ) : null;
}

export function ButtonNode({ node }: { node: Of<"button"> }): React.JSX.Element {
  const flow = useDecision(node.id, node.confirm, node.variant === "destructive");
  const [copied, setCopied] = useState(false);
  const action = node.action;
  const openUrl =
    action.kind === "open" && action.to.kind === "url" && action.to.url.startsWith("https:")
      ? safeExternalUrl(action.to.url)
      : null;
  const local = action.kind === "copy" || openUrl !== null;
  const enabled = !node.disabled && (local || (action.kind === "prompt" && flow.live));
  const chosen = flow.decided?.chosen === true || flow.sent;
  const onClick = (): void => {
    if (action.kind === "copy") {
      void navigator.clipboard?.writeText(action.text).then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1800);
      });
    } else if (openUrl) {
      openExternalUrl(openUrl);
    } else {
      void flow.start([]);
    }
  };
  return (
    <Box sx={{ display: "grid", gap: 0.5, justifyItems: "start" }}>
      <Button
        size="small"
        data-widget-button={node.id}
        disabled={!enabled || flow.pending}
        variant={
          node.variant === "primary" ? "contained" : node.variant === "text" ? "text" : "outlined"
        }
        color={node.variant === "destructive" ? "error" : "primary"}
        startIcon={chosen ? <CheckRounded /> : undefined}
        title={
          enabled || chosen || flow.decided ? undefined : "Read-only widget: reply in the composer"
        }
        onClick={onClick}
        sx={{ textTransform: "none", minHeight: { xs: 44, sm: 32 } }}
      >
        {copied ? "Copied" : node.label}
      </Button>
      {chosen ? <SentNote labels={null} /> : null}
      <ErrorNote text={flow.error} />
      {flow.confirm}
    </Box>
  );
}

export function ChoiceNode({ node }: { node: Of<"choice"> }): React.JSX.Element {
  const multiple = node.multiple === true;
  const max = multiple ? (node.max_selected ?? WIDGET_LIMITS.maxSelected) : 1;
  const initial =
    node.value === undefined ? [] : Array.isArray(node.value) ? node.value : [node.value];
  const [selected, setSelected] = useState<string[]>(initial.slice(0, max));
  const flow = useDecision(node.id, node.submit?.confirm, false);
  const decidedLabels = flow.decided?.labels ?? null;
  const shown = decidedLabels
    ? node.options.filter((o) => decidedLabels.includes(o.label)).map((o) => o.value)
    : selected;
  const editable = flow.live && !flow.pending;
  const toggle = (value: string): void => {
    if (!multiple) return setSelected([value]);
    setSelected((current) =>
      current.includes(value)
        ? current.filter((v) => v !== value)
        : current.length < max
          ? [...current, value]
          : current,
    );
  };
  return (
    <Box sx={{ display: "grid", gap: 0.25 }} data-widget-choice={node.id}>
      <Typography variant="body2" sx={{ fontWeight: 600 }}>
        {node.label}
      </Typography>
      <Box
        role={multiple ? "group" : "radiogroup"}
        aria-label={node.label}
        sx={{ display: "grid" }}
      >
        {node.options.map((option) => {
          const checked = shown.includes(option.value);
          const Control = multiple ? Checkbox : Radio;
          return (
            <FormControlLabel
              key={option.value}
              sx={{ mr: 0, alignItems: "flex-start", "& .MuiFormControlLabel-label": { pt: 1 } }}
              control={
                <Control
                  size="small"
                  checked={checked}
                  disabled={!editable || (multiple && !checked && selected.length >= max)}
                  onChange={(): void => toggle(option.value)}
                />
              }
              label={
                <Box component="span" sx={{ display: "grid" }}>
                  <Typography variant="body2" component="span">
                    {option.label}
                  </Typography>
                  {option.detail ? (
                    <Typography variant="caption" color="text.secondary" component="span">
                      {option.detail}
                    </Typography>
                  ) : null}
                </Box>
              }
            />
          );
        })}
      </Box>
      {decidedLabels || flow.sent ? (
        <SentNote labels={decidedLabels} />
      ) : flow.interaction ? (
        <Box>
          <Button
            size="small"
            variant="contained"
            data-widget-choice-send={node.id}
            disabled={!editable || selected.length === 0}
            onClick={(): void => void flow.start(selected)}
            sx={{ textTransform: "none", minHeight: { xs: 44, sm: 32 } }}
          >
            {node.submit?.label ?? "Send"}
          </Button>
        </Box>
      ) : null}
      <ErrorNote text={flow.error} />
      {flow.confirm}
    </Box>
  );
}
