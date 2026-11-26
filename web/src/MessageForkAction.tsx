import { Box, Button, CircularProgress, Tooltip, Typography } from "@mui/material";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import { useEffect, useRef, useState } from "react";
import { notifyLineageChanged } from "./ancestorHistory";
import type { RenderItem } from "./derive";
import { duplicateSession, forkAvailability, ForkSessionError, forkSession } from "./sessionFork";
import { requestPickSession } from "./sessionPickRequest";
import { useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";
import { newUuid } from "./uuid";

/** Quiet "Fork from here" affordance beneath completed assistant responses.
 *  Uses native fork when eligible on the originating session, and cleanly
 *  falls back to Linked Fork (`duplicate` with exact `before_seq`) ONLY on a
 *  definitive unsupported refusal — preserving operation IDs across retries. */
export function MessageForkAction({
  originSessionId,
  messageId,
  beforeSeq,
  desktop,
}: {
  originSessionId: string;
  messageId?: string | undefined;
  beforeSeq?: number | undefined;
  desktop: boolean;
}): React.JSX.Element | null {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === originSessionId),
  );
  const machines = useStoreSelector((state) => state.machines);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const nativeOpRef = useRef<string | null>(null);
  const linkedOpRef = useRef<string | null>(null);

  useEffect(() => {
    nativeOpRef.current = null;
    linkedOpRef.current = null;
  }, [originSessionId, messageId, beforeSeq]);

  if (!session || session.system || !session.workspace_id) return null;
  const nativeAvail = forkAvailability(session, machines);
  const supportsWorkspaceMode = machines.some(
    (machine) => machine.id === session.machine_id && machine.capabilities?.fork_workspace_mode === true,
  );
  const canNative = nativeAvail !== null && Boolean(messageId);
  const canLinked = beforeSeq !== undefined && beforeSeq > 0;
  if (!canNative && !canLinked) return null;

  const runLinked = (): ReturnType<typeof duplicateSession> => {
    const op = (linkedOpRef.current ??= `fork-turn-${newUuid()}`);
    return duplicateSession(
      originSessionId,
      {
        provider: session.provider,
        provider_account_id: session.provider_account_id || "default",
        before_seq: beforeSeq,
        ...(supportsWorkspaceMode ? { include_uncommitted: false } : {}),
      },
      op,
    );
  };

  const runFork = async (): Promise<void> => {
    setBusy(true);
    setError("");
    try {
      let receipt;
      if (canNative && messageId) {
        try {
          const op = (nativeOpRef.current ??= `fork-msg-${newUuid()}`);
          receipt = await forkSession(originSessionId, { messageId, operation: op });
        } catch (nativeErr) {
          if (!(nativeErr instanceof ForkSessionError && nativeErr.definitiveUnsupported && canLinked)) {
            throw nativeErr;
          }
          nativeOpRef.current = null;
          receipt = await runLinked();
        }
      } else {
        receipt = await runLinked();
      }
      nativeOpRef.current = null;
      linkedOpRef.current = null;
      notifyLineageChanged(receipt.session_id);
      notifyLineageChanged(originSessionId);
      requestPickSession(receipt.session_id, receipt.session ?? undefined);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not fork from this message");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Box
      data-message-fork-bar
      sx={{
        display: "flex",
        alignItems: "center",
        gap: 0.75,
        mt: 0.25,
        px: 0.25,
        opacity: desktop ? 0 : 0.78,
        transition: "opacity 140ms ease",
        ".cowboy-msg-wrap:hover &, .cowboy-msg-wrap:focus-within &": {
          opacity: 1,
        },
      }}
    >
      <Tooltip title="Create a new branch from this response">
        <Button
          size="small"
          variant="text"
          color="inherit"
          data-fork-from-here
          disabled={busy}
          startIcon={
            busy
              ? <CircularProgress size={12} color="inherit" />
              : <CallSplitOutlined sx={{ fontSize: glyphSize(14) }} />
          }
          onClick={(): void => {
            void runFork();
          }}
          sx={{
            minWidth: 0,
            minHeight: desktop ? 24 : 32,
            px: 0.75,
            py: 0,
            borderRadius: 999,
            fontSize: "0.72rem",
            color: "text.secondary",
            textTransform: "none",
            "&:hover": { color: "primary.main", bgcolor: "action.hover" },
          }}
        >
          {busy ? "Forking…" : "Fork from here"}
        </Button>
      </Tooltip>
      {error && (
        <Typography variant="caption" color="error" noWrap title={error}>
          {error}
        </Typography>
      )}
    </Box>
  );
}

/** Wraps an assistant MessageBubble with the quiet in-transcript "Fork from here"
 *  action when the message is settled (non-streaming). Keeps TranscriptItems.tsx
 *  clean and strictly within its structural line budget. */
export function MessageWithFork({
  item,
  sessionId,
  streaming,
  desktop,
  children,
}: {
  item: Extract<RenderItem, { kind: "message" }>;
  sessionId: string;
  streaming: boolean;
  desktop: boolean;
  children: React.JSX.Element;
}): React.JSX.Element {
  if (item.role !== "assistant" || streaming) return children;
  const seq = Number(item.key);
  const lastSeq = item.lastSeq ?? (Number.isFinite(seq) ? seq : undefined);
  const beforeSeq = lastSeq !== undefined ? lastSeq + 1 : undefined;
  return (
    <Box className="cowboy-msg-wrap" sx={{ display: "flex", flexDirection: "column", alignItems: "flex-start", maxWidth: "100%" }}>
      {children}
      <MessageForkAction
        originSessionId={sessionId}
        messageId={item.messageId}
        beforeSeq={beforeSeq}
        desktop={desktop}
      />
    </Box>
  );
}
