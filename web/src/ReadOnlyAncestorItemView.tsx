import { Box, Stack, Typography } from "@mui/material";
import { CheckCircleOutline, ErrorOutline, WarningAmberRounded } from "@mui/icons-material";
import { memo } from "react";
import type { RenderItem } from "./derive";
import { prettifyCrashDetail } from "./crashDetail";
import { NoticeRow } from "./NoticeRow";
import { ErrorDetails, ItemView, LIFECYCLE_ICON_SX, LIFECYCLE_ROW_SX } from "./TranscriptItems";
import { transcriptLifecycleLabel } from "./transcriptLoadingPresentation";

function ReadOnlyPermission({
  item,
}: {
  item: Extract<RenderItem, { kind: "permission" }>;
}): React.JSX.Element {
  const question = item.requestKind === "question";
  const prefix = item.subagent ? `${item.subagent}: ` : "";
  if (item.resolved) {
    const rejected = item.chosen?.toLowerCase().startsWith("reject") ?? false;
    const verdict = question
      ? rejected ? "Declined" : "Answered"
      : rejected ? "Rejected" : "Allowed";
    const choice = !question && item.chosen ? `: ${item.chosen}` : "";
    return (
      <Stack direction="row" spacing={1} alignItems="center" sx={{ color: "text.secondary", px: 0.5 }}>
        <Typography variant="caption" noWrap sx={{ fontStyle: "italic", minWidth: 0 }}>
          {verdict}{choice} · {prefix}{item.title}
        </Typography>
      </Stack>
    );
  }
  return (
    <Stack direction="row" spacing={1} alignItems="center" sx={{ color: "text.secondary", px: 0.5 }}>
      <Typography variant="caption" noWrap sx={{ fontStyle: "italic", minWidth: 0 }}>
        {prefix}{question ? "Question asked" : `Permission requested · ${item.title}`}
      </Typography>
    </Stack>
  );
}

function ReadOnlyLifecycle({
  item,
}: {
  item: Extract<RenderItem, { kind: "lifecycle" }>;
}): React.JSX.Element | null {
  if (item.recovered) {
    return (
      <Box sx={{ ...LIFECYCLE_ROW_SX, color: "text.secondary" }}>
        <CheckCircleOutline sx={LIFECYCLE_ICON_SX} />
        <Box component="span">Session recovered</Box>
      </Box>
    );
  }
  const label = item.turnFailure && item.detail
    ? prettifyCrashDetail(item.detail)
    : transcriptLifecycleLabel(item.status, item.detail, prettifyCrashDetail);
  if (!label) return null;
  const interrupted = item.status === "interrupted";
  const color = item.status === "crashed"
    ? "error.main"
    : interrupted
    ? "warning.main"
    : "text.secondary";
  return (
    <Box sx={{ ...LIFECYCLE_ROW_SX, color }}>
      {interrupted
        ? <WarningAmberRounded sx={LIFECYCLE_ICON_SX} />
        : <ErrorOutline sx={LIFECYCLE_ICON_SX} />}
      <Box sx={{ minWidth: 0 }}>
        <Box component="span" sx={{ fontWeight: interrupted ? 600 : 400 }}>{label}</Box>
        {item.turnFailure && item.detail && <ErrorDetails detail={item.detail} />}
      </Box>
    </Box>
  );
}

/** Strictly read-only renderer for inherited ancestor rows. Strips every live
 *  write capability: widget submissions, permission prompts, turn recovery
 *  prompts, and live task stop/open controls. */
export const ReadOnlyAncestorItemView = memo(function ReadOnlyAncestorItemView({
  item,
  originSessionId,
  provider,
  providerVersion,
  providerDigest,
  desktop,
  selectedToolKey,
  onOpenTool,
}: {
  item: RenderItem;
  originSessionId: string;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  desktop: boolean;
  selectedToolKey: string | null;
  onOpenTool: (key: string) => void;
}): React.JSX.Element | null {
  switch (item.kind) {
    case "permission":
      return <ReadOnlyPermission item={item} />;
    case "subagent":
      return (
        <Box sx={{ ...LIFECYCLE_ROW_SX, color: "text.secondary" }} data-readonly-task-row="subagent">
          <Typography variant="caption" color="text.secondary" noWrap>
            Subagent · {item.subagent.title || item.subagent.id}
          </Typography>
        </Box>
      );
    case "call":
      return (
        <Box sx={{ ...LIFECYCLE_ROW_SX, color: "text.secondary" }} data-readonly-task-row="call">
          <Typography variant="caption" color="text.secondary" noWrap>
            Delegated call · {item.callId}
          </Typography>
        </Box>
      );
    case "lifecycle":
      return <ReadOnlyLifecycle item={item} />;
    case "notice":
      return <NoticeRow item={item} sessionId={originSessionId} readOnly />;
    case "forked":
      return null;
    default:
      return (
        <ItemView
          item={item}
          sessionId={originSessionId}
          status="exited"
          streaming={false}
          provider={provider}
          providerVersion={providerVersion}
          providerDigest={providerDigest}
          desktop={desktop}
          selectedToolKey={selectedToolKey}
          onOpenTool={onOpenTool}
          readOnly
        />
      );
  }
});
