import { ScrollEdges } from "@cowboy/app-shell";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import CloseRounded from "@mui/icons-material/CloseRounded";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import OpenInNewRounded from "@mui/icons-material/OpenInNewRounded";
import RestoreFromTrashOutlined from "@mui/icons-material/RestoreFromTrashOutlined";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { mergeAncestorEvents } from "./ancestorHistory";
import { derive } from "./derive";
import { fetchHistoryPage } from "./historyFetch";
import type { Envelope, SessionMeta } from "./protocol";
import { SESSION_LINK_LABEL, type SessionLinkRow } from "./sessionLinks";
import { canonicalTimeline, conn, useStoreSelector } from "./store";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { Sheet } from "./Sheet";
import { ReadOnlyAncestorItemView } from "./ReadOnlyAncestorItemView";
import { ToolDetailsBrowser } from "./ToolDetailsBrowser";
import { toolRuns } from "./tools/runs";
import { shouldPaintTranscriptLifecycle } from "./transcriptLoadingPresentation";
import { Kbd } from "./Kbd";
import { desktopDialogSlotProps, InstantModalTransition, isImeBlockedEscape } from "./desktop/DesktopModal";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";


function useAncestorHistoryPeek(open: boolean, sessionId: string, before: number | undefined) {
  const [events, setEvents] = useState<Envelope[]>([]);
  const [nextBeforeSeq, setNextBeforeSeq] = useState<number | null>(null);
  const [reachedStart, setReachedStart] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const abortRef = useRef<AbortController | null>(null);

  const loadPage = useCallback(
    async (cursor: number, replace: boolean): Promise<void> => {
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      setLoading(true);
      setError("");
      try {
        const page = await fetchHistoryPage(sessionId, cursor, conn.version() ?? "0", controller.signal);
        if (controller.signal.aborted) return;
        if (!page) {
          setError("Could not load this conversation's history.");
          return;
        }
        setEvents((current) => mergeAncestorEvents(replace ? [] : current, page.events, before));
        setNextBeforeSeq(page.next_before_seq);
        setReachedStart(page.reached_start || page.next_before_seq === null);
      } finally {
        if (!controller.signal.aborted) setLoading(false);
      }
    },
    [before, sessionId],
  );

  useEffect(() => {
    if (!open) {
      abortRef.current?.abort();
      return undefined;
    }
    const warm = canonicalTimeline(sessionId);
    const warmFiltered = warm ? mergeAncestorEvents([], warm, before) : [];
    setEvents(warmFiltered);
    setReachedStart(false);
    setNextBeforeSeq(null);
    void loadPage(before ?? Number.MAX_SAFE_INTEGER, warmFiltered.length === 0);
    return () => {
      abortRef.current?.abort();
    };
  }, [before, loadPage, open, sessionId]);

  return { events, nextBeforeSeq, reachedStart, loading, error, loadPage };
}

function HistoryPeekToolbar({
  link,
  accountLabel,
  onClose,
  onOpenSession,
  onRestore,
  restoring,
}: {
  link: SessionLinkRow;
  accountLabel: string;
  onClose: () => void;
  onOpenSession?: (() => void) | undefined;
  onRestore?: (() => void) | undefined;
  restoring: boolean;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      justifyContent="space-between"
      spacing={1}
      sx={{ pb: 1, mb: 1, borderBottom: 1, borderColor: "divider", flexShrink: 0 }}
    >
      <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0, flexWrap: "wrap" }} useFlexGap>
        <Chip
          size="small"
          variant="outlined"
          label={link.state === "trash" ? "In Trash · Read-only" : "Read-only preview"}
          sx={{ height: 22, fontSize: "0.72rem" }}
        />
        {accountLabel && (
          <Typography variant="caption" color="text.secondary" noWrap>
            {accountLabel}
          </Typography>
        )}
        {link.before !== undefined && (
          <Typography variant="caption" color="text.secondary" noWrap>
            Snapshot at handoff
          </Typography>
        )}
      </Stack>
      <Stack direction="row" spacing={0.75} alignItems="center" flexWrap="wrap" useFlexGap>
        {link.state === "open" && onOpenSession && (
          <Button
            size="small"
            variant="outlined"
            startIcon={<OpenInNewRounded fontSize="small" />}
            onClick={(): void => {
              onClose();
              onOpenSession();
            }}
            sx={{ textTransform: "none" }}
          >
            Open session
          </Button>
        )}
        {link.state === "trash" && onRestore && (
          <Button
            size="small"
            variant="outlined"
            disabled={restoring}
            startIcon={<RestoreFromTrashOutlined fontSize="small" />}
            onClick={onRestore}
            sx={{ textTransform: "none" }}
          >
            {restoring ? "Restoring…" : "Restore"}
          </Button>
        )}
      </Stack>
    </Stack>
  );
}

function HistoryPeekBody({
  link,
  meta,
  desktop,
  onClose,
  onOpenSession,
  onRestore,
  restoring,
  peek,
}: {
  link: SessionLinkRow;
  meta: SessionMeta | undefined;
  desktop: boolean;
  onClose: () => void;
  onOpenSession?: (() => void) | undefined;
  onRestore?: (() => void) | undefined;
  restoring: boolean;
  peek: ReturnType<typeof useAncestorHistoryPeek>;
}): React.JSX.Element {
  const { data: accounts } = useSubscriptionAccounts();
  const [selectedToolKey, setSelectedToolKey] = useState<string | null>(null);
  const items = useMemo(() => derive(peek.events), [peek.events]);
  const visibleItems = useMemo(
    () =>
      items.filter(
        (item) =>
          item.kind !== "forked" &&
          !(item.kind === "lifecycle" && !shouldPaintTranscriptLifecycle(item.status)),
      ),
    [items],
  );
  const runs = useMemo(() => toolRuns(visibleItems), [visibleItems]);
  const provider = meta?.provider ?? "";
  const accountLabel = meta ? sessionAccountIdentity(accounts, meta).short : "";

  return (
    <Box
      data-session-history-peek={link.id}
      sx={{
        display: "flex",
        flexDirection: "column",
        height: desktop ? "min(72vh, 680px)" : "75dvh",
        minHeight: 240,
        minWidth: 0,
      }}
    >
      <HistoryPeekToolbar
        link={link}
        accountLabel={accountLabel}
        onClose={onClose}
        onOpenSession={onOpenSession}
        onRestore={onRestore}
        restoring={restoring}
      />
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overscrollBehavior: "contain",
          px: desktop ? 1 : 0.5,
          py: 0.75,
          display: "flex",
          flexDirection: "column",
          gap: 1.25,
          position: "relative",
          ...(desktop ? desktopScrollbarSx : {}),
        }}
      >
        {!peek.reachedStart && peek.nextBeforeSeq !== null && (
          <Box sx={{ display: "flex", justifyContent: "center", py: 0.5 }}>
            <Button
              size="small"
              variant="text"
              disabled={peek.loading}
              data-history-peek-load-older
              onClick={(): void => void peek.loadPage(peek.nextBeforeSeq!, false)}
              sx={{ textTransform: "none" }}
            >
              {peek.loading ? "Loading earlier…" : "Load earlier messages"}
            </Button>
          </Box>
        )}
        {peek.error && (
          <Alert
            severity="warning"
            action={
              <Button
                size="small"
                color="inherit"
                onClick={(): void =>
                  void peek.loadPage(
                    peek.nextBeforeSeq ?? link.before ?? Number.MAX_SAFE_INTEGER,
                    peek.events.length === 0,
                  )
                }
              >
                Retry
              </Button>
            }
          >
            {peek.error}
          </Alert>
        )}
        {peek.loading && visibleItems.length === 0 && (
          <Stack direction="row" spacing={1} alignItems="center" justifyContent="center" sx={{ py: 4, color: "text.secondary" }}>
            <CircularProgress size={16} thickness={4} color="inherit" />
            <Typography variant="caption">Loading previous conversation…</Typography>
          </Stack>
        )}
        {!peek.loading && !peek.error && visibleItems.length === 0 && (
          <Typography variant="body2" color="text.secondary" sx={{ py: 4, textAlign: "center" }}>
            No messages were recorded in this session before the handoff.
          </Typography>
        )}
        {visibleItems.map((item) => (
          <Box key={item.key} sx={{ display: "flex", flexDirection: "column" }}>
            <ReadOnlyAncestorItemView
              item={item}
              originSessionId={link.id}
              provider={provider}
              providerVersion={meta?.provider_version}
              providerDigest={meta?.provider_generation_digest}
              desktop={desktop}
              selectedToolKey={selectedToolKey}
              onOpenTool={setSelectedToolKey}
            />
          </Box>
        ))}
        <ScrollEdges />
      </Box>

      <ToolDetailsBrowser
        items={visibleItems}
        runs={runs}
        selectedKey={selectedToolKey}
        desktop={desktop}
        provider={provider}
        providerVersion={meta?.provider_version}
        providerDigest={meta?.provider_generation_digest}
        onSelect={setSelectedToolKey}
        onClose={(): void => setSelectedToolKey(null)}
        onLocate={(): void => undefined}
        historyComplete={peek.reachedStart}
      />
    </Box>
  );
}

/** Read-only transcript preview for an ancestor session (live OR in Trash),
 *  allowing inspection of previous conversation without restoring Trash,
 *  waking workers, or leaving the current session. */
export function SessionHistoryPeek({
  open,
  onClose,
  link,
  session,
  onOpenSession,
  onRestore,
  restoring = false,
}: {
  open: boolean;
  onClose: () => void;
  link: SessionLinkRow;
  session: SessionMeta | undefined;
  onOpenSession?: (() => void) | undefined;
  onRestore?: (() => void) | undefined;
  restoring?: boolean | undefined;
}): React.JSX.Element | null {
  const desktop = useSurfaceProfile().kind === "desktop";
  const trashedSession = useStoreSelector((state) =>
    state.trashedSessions.find((entry) => entry.id === link.id),
  );
  const peek = useAncestorHistoryPeek(open, link.id, link.before);
  if (!open) return null;

  const meta = session ?? trashedSession;
  const Icon = link.kind === "forked" ? CallSplitOutlined : LinkOutlined;
  const content = (
    <HistoryPeekBody
      link={link}
      meta={meta}
      desktop={desktop}
      onClose={onClose}
      onOpenSession={onOpenSession}
      onRestore={onRestore}
      restoring={restoring}
      peek={peek}
    />
  );

  if (!desktop) {
    return (
      <Sheet open={open} onClose={onClose} cover title={`${SESSION_LINK_LABEL[link.kind]} · ${link.title}`}>
        <Box sx={{ pt: 1 }}>{content}</Box>
      </Sheet>
    );
  }
  return (
    <Dialog
      open={open}
      onClose={(event, reason): void => {
        if (!isImeBlockedEscape(event, reason)) onClose();
      }}
      maxWidth="md"
      fullWidth
      disableScrollLock
      slots={{ transition: InstantModalTransition }}
      slotProps={desktopDialogSlotProps({ onClose })}
    >
      <DialogTitle sx={{ display: "flex", alignItems: "center", gap: 1, pr: 1.5, py: 1.5 }}>
        <Icon fontSize="small" sx={{ color: "primary.main", flexShrink: 0 }} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", lineHeight: 1.2 }}>
            {SESSION_LINK_LABEL[link.kind]}
          </Typography>
          <Typography variant="subtitle1" noWrap sx={{ fontWeight: 650, lineHeight: 1.3 }}>
            {link.title}
          </Typography>
        </Box>
        <Kbd keys="Esc" />
        <IconButton size="small" aria-label="Close preview" onClick={onClose}>
          <CloseRounded fontSize="small" />
        </IconButton>
      </DialogTitle>
      <DialogContent sx={{ pt: 0, pb: 2 }}>{content}</DialogContent>
    </Dialog>
  );
}
