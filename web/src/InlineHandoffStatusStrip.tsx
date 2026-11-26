import { useState } from "react";
import { Alert, Box, Button, CircularProgress, keyframes, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import ErrorOutline from "@mui/icons-material/ErrorOutline";
import { prettifyCrashDetail } from "./crashDetail";
import { reloadSession } from "./sessionReload";
import { handoffOperation, SessionHandoffCancel } from "./SessionHandoffCancel";
import { useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";

const sheen = keyframes`
  0% { transform: translateX(-100%); }
  100% { transform: translateX(260%); }
`;

function focusComposer(): void {
  document.querySelector<HTMLElement>(
    "[data-mobile-primary-composer] [data-mobile-editor-area] [contenteditable='true'], " +
      "[data-desktop-region='prompt.composer'] [contenteditable='true']",
  )?.focus();
}

function BlockedStripContent({
  sessionId,
  location,
  detail,
}: {
  sessionId: string;
  location: string;
  detail: string | null;
}): React.JSX.Element {
  const [retrying, setRetrying] = useState(false);
  const [error, setError] = useState("");
  const retry = (): void => {
    setRetrying(true);
    setError("");
    void reloadSession(sessionId)
      .catch((failure: unknown) =>
        setError(failure instanceof Error ? failure.message : "Retry did not start"),
      )
      .finally(() => setRetrying(false));
  };
  return (
    <Stack role="alert" spacing={1} sx={{ flex: 1, minWidth: 0 }}>
      <Stack direction="row" spacing={1} alignItems="center" justifyContent="space-between" flexWrap="wrap" useFlexGap>
        <Stack direction="row" spacing={0.85} alignItems="center" sx={{ minWidth: 0 }}>
          <ErrorOutline color="error" sx={{ fontSize: glyphSize(18), flexShrink: 0 }} />
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="body2" sx={{ fontWeight: 600, lineHeight: 1.3 }}>
              Session didn't start
              {location ? ` · ${location}` : ""}
            </Typography>
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", overflowWrap: "anywhere" }}>
              {detail ? prettifyCrashDetail(detail) : "The agent stopped before its first turn."}
            </Typography>
          </Box>
        </Stack>
        <Button
          data-session-start-retry
          variant="contained"
          size="small"
          disableElevation
          disabled={retrying}
          onClick={retry}
          sx={{ borderRadius: 999, textTransform: "none", px: 1.5, minHeight: 32 }}
        >
          {retrying ? "Retrying…" : "Retry"}
        </Button>
      </Stack>
      {error && <Alert severity="error" sx={{ py: 0.25 }}>{error}</Alert>}
    </Stack>
  );
}

/** Compact in-flow status strip anchored below inherited ancestor history when
 *  a forked or replaced session is preparing, blocked, or awaiting its first
 *  prompt. Replaces the jarring full-page blank spinner with calm continuity. */
export function InlineHandoffStatusStrip({
  sessionId,
  kind,
  location,
  agent,
  sleeping = false,
  crashDetail = null,
}: {
  sessionId: string;
  kind: "preparing" | "blocked" | "ready";
  location: string;
  agent: string;
  sleeping?: boolean | undefined;
  crashDetail?: string | null | undefined;
}): React.JSX.Element {
  const { operation, queuedCount } = useStoreSelector((state) => {
    const continued = state.sessions.some((s) => s.id === sessionId && Boolean(s.continued_from));
    const queue = state.queues.get(sessionId) ?? [];
    let op: string | undefined;
    if (continued) {
      for (const row of queue) {
        const found = row.status === undefined ? handoffOperation(row.cmid) : undefined;
        if (found !== undefined) {
          op = found;
          break;
        }
      }
    }
    return { operation: op, queuedCount: queue.length };
  }, (a, b) => a.operation === b.operation && a.queuedCount === b.queuedCount);

  if (kind === "ready") {
    return (
      <Box
        role="region"
        aria-label={`Forked conversation ready for ${agent}`}
        onClick={focusComposer}
        sx={(theme) => ({
          mt: 1.5,
          px: 1.75,
          py: 1.25,
          borderRadius: "12px",
          border: 1,
          borderStyle: "dashed",
          borderColor: alpha(theme.palette.divider, 0.9),
          bgcolor: alpha(theme.palette.action.hover, theme.palette.mode === "dark" ? 0.35 : 0.45),
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 1.25,
          cursor: "text",
          transition: "border-color 160ms ease, background-color 160ms ease",
          "&:hover": {
            borderColor: alpha(theme.palette.primary.main, 0.45),
            bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.08 : 0.04),
          },
        })}
      >
        <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
          <CallSplitOutlined sx={{ fontSize: glyphSize(16), color: "primary.main", flexShrink: 0 }} />
          <Typography variant="body2" color="text.secondary" sx={{ minWidth: 0 }}>
            <Box component="span" sx={{ color: "text.primary", fontWeight: 500 }}>
              {agent}
            </Box>
            {sleeping
              ? " starts when you send your next message."
              : " is ready on this branch — write a message to continue."}
            {location ? ` (${location})` : ""}
          </Typography>
        </Stack>
      </Box>
    );
  }

  const isBlocked = kind === "blocked";
  return (
    <Box
      sx={(theme) => ({
        position: "relative",
        overflow: "hidden",
        mt: 1.5,
        px: 1.75,
        py: 1.35,
        borderRadius: "12px",
        border: 1,
        borderColor: isBlocked
          ? alpha(theme.palette.error.main, theme.palette.mode === "dark" ? 0.45 : 0.3)
          : alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.3 : 0.2),
        bgcolor: isBlocked
          ? alpha(theme.palette.error.main, theme.palette.mode === "dark" ? 0.1 : 0.04)
          : alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.09 : 0.04),
      })}
    >
      {!isBlocked && (
        <Box
          aria-hidden
          sx={(theme) => ({
            position: "absolute",
            top: 0,
            left: 0,
            width: "40%",
            height: 2,
            borderRadius: 999,
            background: `linear-gradient(90deg, transparent, ${theme.palette.primary.main}, transparent)`,
            animation: `${sheen} 1.8s ease-in-out infinite`,
          })}
        />
      )}
      <Stack
        direction={{ xs: "column", sm: "row" }}
        spacing={1.25}
        alignItems={{ xs: "stretch", sm: "center" }}
        justifyContent="space-between"
      >
        {isBlocked ? (
          <BlockedStripContent sessionId={sessionId} location={location} detail={crashDetail} />
        ) : (
          <Stack role="status" direction="row" spacing={1.25} alignItems="center" sx={{ minWidth: 0, flex: 1 }}>
            <CircularProgress size="0.95rem" thickness={4} sx={{ color: "primary.main", flexShrink: 0 }} />
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="body2" sx={{ fontWeight: 600, lineHeight: 1.3 }}>
                Preparing {agent}
                {location ? ` · ${location}` : ""}
              </Typography>
              <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
                {queuedCount > 0
                  ? "Workspace and session are starting — your message will send automatically."
                  : "Creating an isolated workspace and connecting the agent."}
              </Typography>
            </Box>
          </Stack>
        )}
        {operation !== undefined && (
          <Box sx={{ flexShrink: 0, alignSelf: { xs: "flex-start", sm: "center" } }}>
            <SessionHandoffCancel sessionId={sessionId} operation={operation} />
          </Box>
        )}
      </Stack>
    </Box>
  );
}
