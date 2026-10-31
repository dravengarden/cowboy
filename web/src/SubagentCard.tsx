// One ACP subagent in the parent transcript: a single live row (title, state,
// a one-line preview and counts) whose own messages and tool calls stay in a
// collapsed, bounded activity list. Shared by Desktop and Mobile.

import { useEffect, useState } from "react";
import { Box, Button, Chip, Collapse, IconButton, Paper, Stack, Typography } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
import {
  AccountTreeOutlined,
  ChatBubbleOutline,
  ExpandMore,
  StopCircleOutlined,
  Terminal,
} from "@mui/icons-material";
import { DESKTOP_INSET_RADIUS } from "./desktop/DesktopEmbeddedControl";
import { glyphSize } from "./surface/desktopSize";
import { cancelSubagent } from "./store";
import {
  type Subagent,
  type SubagentTone,
  subagentCounts,
  subagentPreview,
  subagentStatus,
} from "./subagents";

function toneColor(tone: SubagentTone, theme: Theme): string {
  switch (tone) {
    case "working":
      return theme.palette.info.main;
    case "waiting":
      return theme.palette.warning.main;
    case "done":
      return theme.palette.success.main;
    case "failed":
      return theme.palette.error.main;
    case "stopped":
    case "unknown":
      return theme.palette.text.secondary;
  }
}

function chipColor(tone: SubagentTone): "info" | "warning" | "success" | "error" | "default" {
  switch (tone) {
    case "working":
      return "info";
    case "waiting":
      return "warning";
    case "done":
      return "success";
    case "failed":
      return "error";
    case "stopped":
    case "unknown":
      return "default";
  }
}

/** A tool still open when its subagent stopped working did not finish. */
function toolStatusLabel(status: string, working: boolean): string {
  if (!working && (status === "pending" || status === "in_progress")) return "not finished";
  return status.replace("_", " ");
}

const SECTION_TEXT_SX = {
  whiteSpace: "pre-wrap",
  overflowWrap: "anywhere",
  maxHeight: "30vh",
  overflow: "auto",
  fontSize: "0.8125rem",
} as const;

function Section({ label, children }: { label: string; children: string }): React.JSX.Element {
  return (
    <Box sx={{ mt: 0.75 }}>
      <Typography variant="caption" sx={{ color: "text.secondary", fontWeight: 650 }}>
        {label}
      </Typography>
      <Box sx={SECTION_TEXT_SX}>{children}</Box>
    </Box>
  );
}

export function SubagentCard({
  subagent,
  sessionId,
  desktop,
  stop = cancelSubagent,
}: {
  subagent: Subagent;
  sessionId: string;
  desktop: boolean;
  /** Sends the stop; returns whether it was sent. Fixtures capture it. */
  stop?: (sessionId: string, subagentSessionId: string) => boolean;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  // A stop is a request: the subagent reports `idle` once it actually stopped.
  const [stopping, setStopping] = useState(false);
  useEffect(() => {
    if (!subagent.cancellable) setStopping(false);
  }, [subagent.cancellable]);
  const status = subagentStatus(subagent);
  const counts = subagentCounts(subagent);
  const preview = subagentPreview(subagent);
  const working = status.tone === "working" || status.tone === "waiting";
  const details = [subagent.description, counts].filter(Boolean).join(" · ");
  const activityId = `subagent-activity-${subagent.id}`;
  return (
    <Paper
      elevation={0}
      data-subagent-row={subagent.id}
      data-subagent-state={subagent.state}
      sx={(theme) => ({
        alignSelf: "stretch",
        overflow: "hidden",
        bgcolor: "background.paper",
        backgroundImage: `linear-gradient(105deg, ${alpha(
          toneColor(status.tone, theme),
          theme.palette.mode === "dark" ? 0.12 : 0.06,
        )}, transparent 52%)`,
        boxShadow: `0 1px 3px ${alpha(
          theme.palette.common.black,
          theme.palette.mode === "dark" ? 0.16 : 0.06,
        )}`,
        borderRadius: desktop ? `${DESKTOP_INSET_RADIUS}px` : 1,
      })}
    >
      <Stack direction="row" spacing={1} alignItems="center" sx={{ p: 1 }}>
        <AccountTreeOutlined
          sx={(theme) => ({ fontSize: glyphSize(18), color: toneColor(status.tone, theme) })}
        />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="body2"
            noWrap
            sx={{ fontSize: "inherit", fontWeight: 650, fontFamily: "var(--cowboy-reading-font, inherit)" }}
          >
            {subagent.parentId ? "↳ " : ""}
            {subagent.title}
          </Typography>
          {details && (
            <Typography variant="caption" noWrap sx={{ display: "block", color: "text.secondary" }}>
              {details}
            </Typography>
          )}
          {preview && (
            <Typography
              variant="caption"
              noWrap
              data-subagent-preview
              sx={{ display: "block", color: working ? "text.primary" : "text.secondary" }}
            >
              {preview}
            </Typography>
          )}
        </Box>
        <Chip size="small" variant="outlined" color={chipColor(status.tone)} label={status.label} />
        {subagent.cancellable && (
          <Button
            size="small"
            color="error"
            data-subagent-stop
            disabled={stopping}
            startIcon={<StopCircleOutlined />}
            aria-label={`Stop subagent ${subagent.title}`}
            onClick={(): void => {
              if (stop(sessionId, subagent.id)) setStopping(true);
            }}
            sx={{ minWidth: 0, minHeight: { xs: 40, sm: 30 }, textTransform: "none" }}
          >
            {stopping ? "Stopping…" : "Stop"}
          </Button>
        )}
        <IconButton
          size={desktop ? "small" : "medium"}
          data-desktop-item-action="default"
          aria-expanded={open}
          aria-controls={activityId}
          aria-label={open ? "Hide subagent activity" : "Show subagent activity"}
          onClick={(): void => setOpen((value) => !value)}
        >
          <ExpandMore
            sx={{ transition: "transform 120ms ease", transform: open ? "rotate(180deg)" : "none" }}
          />
        </IconButton>
      </Stack>
      <Collapse in={open} unmountOnExit>
        <Box id={activityId} data-subagent-activity sx={{ px: 1.5, pb: 1.25 }}>
          {subagent.prompt && <Section label="Asked">{subagent.prompt}</Section>}
          {subagent.omittedActivity > 0 && (
            <Typography variant="caption" sx={{ display: "block", mt: 0.75, color: "text.secondary" }}>
              {subagent.omittedActivity === 1
                ? "1 earlier step not shown"
                : `${String(subagent.omittedActivity)} earlier steps not shown`}
            </Typography>
          )}
          {subagent.activity.length > 0 && (
            <Stack component="ol" spacing={0.5} sx={{ listStyle: "none", m: 0, mt: 0.75, p: 0 }}>
              {subagent.activity.map((entry, index) => (
                <Stack
                  component="li"
                  key={entry.id ?? `message-${String(index)}`}
                  direction="row"
                  spacing={0.75}
                  alignItems="flex-start"
                  data-subagent-activity-entry={entry.kind}
                  sx={{ fontSize: "0.8125rem", minWidth: 0 }}
                >
                  {entry.kind === "tool"
                    ? <Terminal sx={{ fontSize: glyphSize(15), mt: "2px", color: "text.secondary" }} />
                    : <ChatBubbleOutline sx={{ fontSize: glyphSize(15), mt: "2px", color: "text.secondary" }} />}
                  <Box
                    sx={{
                      flex: 1,
                      minWidth: 0,
                      overflowWrap: "anywhere",
                      whiteSpace: entry.kind === "message" ? "pre-wrap" : "normal",
                    }}
                  >
                    {entry.text}
                  </Box>
                  {entry.kind === "tool" && (
                    <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>
                      {toolStatusLabel(entry.status, working)}
                    </Typography>
                  )}
                </Stack>
              ))}
            </Stack>
          )}
          {subagent.result && !working && <Section label="Result">{subagent.result}</Section>}
          {subagent.error && <Section label="Error">{subagent.error}</Section>}
          {!subagent.prompt && subagent.activity.length === 0 && !subagent.result && (
            <Typography variant="caption" sx={{ display: "block", mt: 0.75, color: "text.secondary" }}>
              No activity yet.
            </Typography>
          )}
        </Box>
      </Collapse>
    </Paper>
  );
}
