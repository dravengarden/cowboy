import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import {
  ArrowBack,
  DeleteOutline,
  DescriptionOutlined,
  DriveFileMoveOutlined,
  Undo,
} from "@mui/icons-material";
import type { QueueEditHoldStatus } from "../queueEditingHold";
import { desktopSize } from "../surface/desktopSize";
import { desktopToolbarActionSx } from "./desktopToolbarAction";
import { HintTooltip } from "../HintTooltip";

export interface DesktopPromptBufferBarProps {
  kind: "queued" | "draft" | "scheduled" | "new-draft";
  index?: number | undefined;
  total?: number | undefined;
  dirty: boolean;
  saving: boolean;
  savedLabel: string | null;
  holdStatus: QueueEditHoldStatus;
  scheduleLabel?: string | undefined;
  flashHint: string | null;
  onBack: () => void;
  onDiscard?: (() => void) | undefined;
  onDemoteToDraft?: (() => void) | undefined;
  onMoveToSession?: (() => void) | undefined;
  onPromoteToDocument?: (() => void) | undefined;
}

function bufferIdentityLabel(props: DesktopPromptBufferBarProps): string {
  const pos = props.index !== undefined && props.total !== undefined
    ? ` #${String(props.index + 1)} of ${String(props.total)}`
    : "";
  if (props.kind === "queued") return `Queue${pos}`;
  if (props.kind === "scheduled") {
    return props.scheduleLabel
      ? `Scheduled Draft · ${props.scheduleLabel}`
      : `Scheduled Draft${pos}`;
  }
  if (props.kind === "new-draft") return "New Session Draft";
  return `Draft${pos}`;
}

function bufferStateBadge(
  props: DesktopPromptBufferBarProps,
): { text: string; tone: "warning" | "info" | "success" | "muted" } | null {
  if (props.flashHint) return { text: props.flashHint, tone: "warning" };
  if (props.kind === "queued") {
    if (props.holdStatus === "unconfirmed") {
      return { text: "Local edits · hold unconfirmed", tone: "warning" };
    }
    return {
      text: props.dirty ? "Unsaved · Auto-drain held" : "Auto-drain held",
      tone: "warning",
    };
  }
  if (props.kind === "scheduled") {
    return props.dirty
      ? { text: "Unsaved changes", tone: "warning" }
      : { text: props.savedLabel ?? "Armed", tone: "info" };
  }
  if (props.saving) return { text: "Saving…", tone: "muted" };
  if (props.savedLabel) return { text: props.savedLabel, tone: "success" };
  return props.dirty
    ? { text: "Auto-saving…", tone: "muted" }
    : { text: "Auto-saved", tone: "muted" };
}

function BufferBarActions({
  kind,
  dirty,
  onBack,
  onDiscard,
  onDemoteToDraft,
  onMoveToSession,
  onPromoteToDocument,
}: Pick<
  DesktopPromptBufferBarProps,
  | "kind"
  | "dirty"
  | "onBack"
  | "onDiscard"
  | "onDemoteToDraft"
  | "onMoveToSession"
  | "onPromoteToDocument"
>): React.JSX.Element {
  return (
    <>
      {onDemoteToDraft && (
        <HintTooltip title="Save and return this queued message to Session drafts">
          <Button
            size="small"
            color="inherit"
            data-buffer-action="demote"
            onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
            onClick={onDemoteToDraft}
            sx={{ ...desktopToolbarActionSx, minHeight: "1.625rem" }}
          >
            <Undo />
            <span>To drafts</span>
          </Button>
        </HintTooltip>
      )}
      {onMoveToSession && (
        <HintTooltip title="Move draft to another session">
          <Button
            size="small"
            color="inherit"
            aria-label="Move to another session"
            data-buffer-action="move"
            onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
            onClick={onMoveToSession}
            sx={{ ...desktopToolbarActionSx, minHeight: "1.625rem" }}
          >
            <DriveFileMoveOutlined />
          </Button>
        </HintTooltip>
      )}
      {onPromoteToDocument && (
        <HintTooltip title="Move to independent Drafts">
          <Button
            size="small"
            color="inherit"
            aria-label="Move to independent Drafts"
            data-buffer-action="promote"
            onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
            onClick={onPromoteToDocument}
            sx={{ ...desktopToolbarActionSx, minHeight: "1.625rem" }}
          >
            <DescriptionOutlined />
          </Button>
        </HintTooltip>
      )}
      {dirty && onDiscard && (
        <HintTooltip title="Discard unsaved edits" shortcut="ZQ">
          <Button
            size="small"
            color="error"
            data-buffer-action="discard"
            onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
            onClick={onDiscard}
            sx={{ ...desktopToolbarActionSx, minHeight: "1.625rem" }}
          >
            <DeleteOutline />
            <span>Discard</span>
          </Button>
        </HintTooltip>
      )}
      <HintTooltip
        title={kind === "queued" || kind === "scheduled"
          ? "Close editor"
          : "Save and return"}
        shortcut="Esc"
      >
        <Button
          size="small"
          color="inherit"
          data-buffer-action="back"
          onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
          onClick={onBack}
          sx={{ ...desktopToolbarActionSx, minHeight: "1.625rem" }}
        >
          <ArrowBack />
          <span>Back</span>
        </Button>
      </HintTooltip>
    </>
  );
}

/** Slim context header directly above the unified Prompt canvas whenever
 *  editing a Queue item or Session Draft. Remains visible even when focus
 *  moves to Conversation so the writer always knows which buffer is active. */
export function DesktopPromptBufferBar(
  props: DesktopPromptBufferBarProps,
): React.JSX.Element {
  const identity = bufferIdentityLabel(props);
  const badge = bufferStateBadge(props);
  const accent = props.kind === "queued"
    ? "warning.main"
    : props.kind === "scheduled"
    ? "info.main"
    : "primary.main";

  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.5}
      data-desktop-buffer-bar={props.kind}
      data-desktop-buffer-dirty={props.dirty ? "true" : "false"}
      sx={(theme) => ({
        flexShrink: 0,
        minHeight: desktopSize(32),
        px: 1,
        py: 0.25,
        borderBottom: 1,
        borderColor: "divider",
        bgcolor: alpha(
          props.kind === "queued"
            ? theme.palette.warning.main
            : props.kind === "scheduled"
            ? theme.palette.info.main
            : theme.palette.primary.main,
          props.flashHint ? 0.14 : 0.06,
        ),
        transition: "background-color 150ms ease",
      })}
    >
      <Box
        sx={{
          width: 3,
          height: 14,
          borderRadius: 99,
          bgcolor: accent,
          flexShrink: 0,
        }}
      />
      <Typography
        variant="caption"
        sx={{ fontWeight: 700, whiteSpace: "nowrap", flexShrink: 0 }}
      >
        {identity}
      </Typography>
      {badge && (
        <Typography
          variant="caption"
          data-desktop-buffer-status
          sx={{
            ml: 0.5,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            fontWeight: props.flashHint ? 650 : 500,
            color: badge.tone === "warning"
              ? "warning.main"
              : badge.tone === "info"
              ? "info.main"
              : badge.tone === "success"
              ? "success.main"
              : "text.secondary",
          }}
        >
          · {badge.text}
        </Typography>
      )}
      <Box sx={{ flex: 1, minWidth: 0 }} />
      <BufferBarActions
        kind={props.kind}
        dirty={props.dirty}
        onBack={props.onBack}
        onDiscard={props.onDiscard}
        onDemoteToDraft={props.onDemoteToDraft}
        onMoveToSession={props.onMoveToSession}
        onPromoteToDocument={props.onPromoteToDocument}
      />
    </Stack>
  );
}
