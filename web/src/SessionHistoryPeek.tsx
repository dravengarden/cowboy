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
import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  cachedAncestorSegment,
  fetchLineageHistoryPage,
  fetchSessionLineage,
  mergeAncestorEvents,
  setCachedAncestorSegment,
} from "./ancestorHistory";
import { derive } from "./derive";
import { fetchHistoryPage } from "./historyFetch";
import type { Envelope, SessionMeta } from "./protocol";
import { SESSION_LINK_LABEL, type SessionLinkRow } from "./sessionLinks";
import { conn, useStoreSelector } from "./store";
import { SurfaceContext } from "./surface/SurfaceProfile";
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

export interface LineagePeekTarget {
  childSessionId: string;
  hopId: string;
  edgeVersion: number;
  beforeSeq: number | null;
}

const MAX_PEEK_UNSEALED_RETRIES = 5;

function useAncestorHistoryPeek(
  open: boolean,
  sessionId: string,
  before: number | undefined,
  lineage?: LineagePeekTarget,
) {
  const [events, setEvents] = useState<Envelope[]>([]);
  const [nextBeforeSeq, setNextBeforeSeq] = useState<number | null>(null);
  const [reachedStart, setReachedStart] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const abortRef = useRef<AbortController | null>(null);
  const edgeRef = useRef<{ edgeVersion: number; beforeSeq: number | null }>({
    edgeVersion: lineage?.edgeVersion ?? 1,
    beforeSeq: lineage?.beforeSeq ?? before ?? null,
  });

  useEffect(() => {
    edgeRef.current = {
      edgeVersion: lineage?.edgeVersion ?? 1,
      beforeSeq: lineage?.beforeSeq ?? before ?? null,
    };
  }, [before, lineage?.beforeSeq, lineage?.edgeVersion]);

  const loadPage = useCallback(
    async (cursor: number, replace: boolean, attempt = 0): Promise<void> => {
      if (attempt === 0) {
        abortRef.current?.abort();
        abortRef.current = new AbortController();
        setLoading(true);
        setError("");
      }
      const controller = abortRef.current;
      if (!controller || controller.signal.aborted) return;
      try {
        if (lineage) {
          const cutoff = edgeRef.current.beforeSeq;
          if (cutoff === null || cutoff === 0) {
            setReachedStart(true);
            setNextBeforeSeq(null);
            return;
          }
          const result = await fetchLineageHistoryPage(
            lineage.childSessionId,
            lineage.hopId,
            cursor,
            edgeRef.current.edgeVersion,
            conn.version() ?? "0",
            controller.signal,
          );
          if (controller.signal.aborted) return;
          if (result.kind === "conflict") {
            const fresh = await fetchSessionLineage(lineage.childSessionId, controller.signal);
            if (controller.signal.aborted) return;
            const freshHop = fresh?.hops.find((h) => h.session_id === lineage.hopId);
            if (!freshHop || freshHop.before_seq === null || freshHop.before_seq === 0) {
              setEvents([]);
              setNextBeforeSeq(null);
              setReachedStart(true);
              return;
            }
            edgeRef.current = { edgeVersion: freshHop.edge_version, beforeSeq: freshHop.before_seq };
            await loadPage(freshHop.before_seq, true, 1);
            return;
          }
          if (result.kind === "error") {
            setError("Could not load this conversation's history.");
            return;
          }
          const { page } = result;
          const sealed = page.is_sealed !== false;
          setEvents((current) => {
            const merged = mergeAncestorEvents(replace ? [] : current, page.events, cutoff);
            setCachedAncestorSegment(
              lineage.childSessionId,
              lineage.hopId,
              edgeRef.current.edgeVersion,
              {
                events: merged,
                nextBeforeSeq: sealed ? page.next_before_seq : cursor,
                reachedStart: sealed && (page.reached_start || page.next_before_seq === null),
                isSealed: sealed,
              },
            );
            return merged;
          });
          setNextBeforeSeq(sealed ? page.next_before_seq : cursor);
          setReachedStart(sealed && (page.reached_start || page.next_before_seq === null));
          if (!sealed && attempt < MAX_PEEK_UNSEALED_RETRIES) {
            const delayMs = Math.min(250 * 2 ** attempt, 2000);
            const timer = setTimeout(() => {
              if (!controller.signal.aborted) void loadPage(cursor, false, attempt + 1);
            }, delayMs);
            controller.signal.addEventListener("abort", () => clearTimeout(timer), { once: true });
          }
          return;
        }
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
    [before, lineage, sessionId],
  );

  useEffect(() => {
    if (!open) {
      abortRef.current?.abort();
      return undefined;
    }
    if (lineage) {
      const cached = cachedAncestorSegment(
        lineage.childSessionId,
        lineage.hopId,
        edgeRef.current.edgeVersion,
      );
      if (cached) {
        setEvents(cached.events);
        setNextBeforeSeq(cached.nextBeforeSeq);
        setReachedStart(cached.reachedStart);
        if (cached.reachedStart && cached.isSealed) return undefined;
        if (cached.nextBeforeSeq !== null && cached.nextBeforeSeq > 0) {
          void loadPage(cached.nextBeforeSeq, false);
        }
        return () => abortRef.current?.abort();
      }
    }
    setEvents([]);
    setReachedStart(false);
    setNextBeforeSeq(null);
    const startCursor = lineage ? (edgeRef.current.beforeSeq ?? 0) : (before ?? Number.MAX_SAFE_INTEGER);
    void loadPage(startCursor, true);
    return () => {
      abortRef.current?.abort();
    };
  }, [before, lineage, loadPage, open, sessionId]);

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
          label={
            link.state === "trash"
              ? "In Trash · Read-only"
              : link.state === "deleted"
                ? "Archived · Read-only"
                : "Read-only preview"
          }
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
          <Box sx={{ display: "flex", justifyContent: "center", pb: 0.5 }}>
            <Button
              size="small"
              variant="text"
              disabled={peek.loading}
              onClick={(): void => void peek.loadPage(peek.nextBeforeSeq!, false)}
              sx={{ textTransform: "none" }}
            >
              {peek.loading ? "Loading earlier messages…" : "Load earlier messages"}
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

/** Read-only transcript preview for an ancestor session (live, in Trash, or
 *  archived after Purge), allowing inspection of previous conversation without
 *  restoring Trash, waking workers, or leaving the current session. */
export function SessionHistoryPeek({
  open,
  onClose,
  link,
  session,
  lineage,
  onOpenSession,
  onRestore,
  restoring = false,
}: {
  open: boolean;
  onClose: () => void;
  link: SessionLinkRow;
  session: SessionMeta | undefined;
  lineage?: LineagePeekTarget | undefined;
  onOpenSession?: (() => void) | undefined;
  onRestore?: (() => void) | undefined;
  restoring?: boolean | undefined;
}): React.JSX.Element | null {
  const desktop = useContext(SurfaceContext)?.kind === "desktop";
  const trashedSession = useStoreSelector((state) =>
    state.trashedSessions.find((entry) => entry.id === link.id),
  );
  const peek = useAncestorHistoryPeek(open, link.id, link.before, lineage);
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
      <Sheet open={open} onClose={onClose} cover portal title={`${SESSION_LINK_LABEL[link.kind]} · ${link.title}`}>
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
