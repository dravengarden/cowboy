// One ACP subagent's details: what it was asked, its bounded activity and its
// result. The transcript shows one compact row per subagent (TaskRow); these
// details open in the background-tasks inspector. Shared by Desktop and Mobile.

import {
  Box,
  Button,
  DialogContentText,
  Stack,
  Typography,
} from "@mui/material";
import type { Theme } from "@mui/material/styles";
import { ChatBubbleOutline, Terminal } from "@mui/icons-material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { ConfirmSheet } from "./Sheet";
import { glyphSize } from "./surface/desktopSize";
import {
  type Subagent,
  type SubagentTone,
  subagentCounts,
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

/** Stopping a subagent ends its work for good, so one tap or key must not do
 *  it: the same confirmation as the session's own Stop. */
function SubagentStopConfirm({
  open,
  title,
  noun = "subagent",
  onClose,
  onConfirm,
}: {
  open: boolean;
  title: string;
  /** "subagent" or "call". */
  noun?: string;
  onClose: () => void;
  onConfirm: () => void;
}): React.JSX.Element {
  useConfirmEnter(open, onConfirm);
  return (
    <ConfirmSheet
      open={open}
      onClose={onClose}
      title={`Stop ${noun} ${title}?`}
      actions={
        <>
          <Button color="inherit" onClick={onClose} sx={{ textTransform: "none" }}>
            Keep running
            <Kbd keys="Esc" />
          </Button>
          <Button
            color="error"
            variant="contained"
            data-subagent-stop-confirm
            onClick={onConfirm}
            sx={{ textTransform: "none" }}
          >
            Stop
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </>
      }
    >
      <DialogContentText>
        {noun === "subagent"
          ? "Only this subagent stops; the session and its other subagents keep working. What it did so far stays in its activity."
          : "Only this call stops; the session keeps working. Its conversation so far stays available."}
      </DialogContentText>
    </ConfirmSheet>
  );
}

/** A task's chip color, by its display tone. */
export function taskChipColor(tone: SubagentTone): "info" | "warning" | "success" | "error" | "default" {
  return chipColor(tone);
}

export function taskToneColor(tone: SubagentTone, theme: Theme): string {
  return toneColor(tone, theme);
}

/** Today's subagent card body — what it was asked, its bounded activity, its
 *  result or error — now shown in the background-tasks inspector instead of
 *  an inline expansion (docs/background-tasks.md §6.1). */
export function SubagentDetail({
  subagent,
  resumed = false,
}: {
  subagent: Subagent;
  /** A run after an earlier one was observed: snapshots keep the latest only. */
  resumed?: boolean;
}): React.JSX.Element {
  const status = subagentStatus(subagent);
  const working = status.tone === "working" || status.tone === "waiting";
  const counts = subagentCounts(subagent);
  return (
    <Box data-subagent-activity sx={{ color: "text.primary" }}>
      {(subagent.description || counts) && (
        <Typography variant="body2" sx={{ color: "text.secondary" }}>
          {[subagent.description, counts].filter(Boolean).join(" · ")}
        </Typography>
      )}
      {resumed && (
        <Typography variant="caption" data-subagent-latest-run sx={{ display: "block", mt: 0.75, color: "text.secondary" }}>
          Showing the latest run only; earlier runs are not retained.
        </Typography>
      )}
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
  );
}

export { SubagentStopConfirm };
