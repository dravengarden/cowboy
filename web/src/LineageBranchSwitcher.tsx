import {
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import AccountTreeOutlined from "@mui/icons-material/AccountTreeOutlined";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import CloseRounded from "@mui/icons-material/CloseRounded";
import SwapHorizRounded from "@mui/icons-material/SwapHorizRounded";
import VisibilityOutlined from "@mui/icons-material/VisibilityOutlined";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { LineageBranch, LineageHop } from "./ancestorHistory";
import { desktopDialogSlotProps, InstantModalTransition, isImeBlockedEscape } from "./desktop/DesktopModal";
import { desktopSessionActionSx } from "./desktop/DesktopEmbeddedControl";
import { desktopEmbeddedControlIconSx } from "./desktop/DesktopEmbeddedIcon";
import { LeaderKeycap } from "./desktop/commands/DesktopKeycap";
import { useDesktopCommand } from "./desktop/commands/DesktopCommandProvider";
import { DESKTOP_WORKSPACE_KEYS, desktopLeaderSequence } from "./desktop/commands/workspaceShortcuts";
import { Kbd } from "./Kbd";
import { providerPresentation } from "./providerPresentation";
import type { SessionLinkRow } from "./sessionLinks";
import { requestPickSession } from "./sessionPickRequest";
import { requestNewFrom } from "./sessionSettingsOpen";
import { SessionHistoryPeek } from "./SessionHistoryPeek";
import { Sheet } from "./Sheet";
import { useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";
import { useSessionLineage } from "./useSessionLineage";

interface SwitcherEntry {
  key: string;
  section: "ancestor" | "current" | "child" | "sibling";
  sessionId: string;
  title: string;
  provider: string;
  kind: string;
  stateLabel?: string | undefined;
  openable: boolean;
  hop?: LineageHop | undefined;
}

function buildEntries(
  sessionId: string,
  currentTitle: string,
  currentProvider: string,
  hops: readonly LineageHop[],
  branches: readonly LineageBranch[],
): SwitcherEntry[] {
  const entries: SwitcherEntry[] = [];
  for (const hop of [...hops].reverse()) {
    entries.push({
      key: `anc:${hop.session_id}`,
      section: "ancestor",
      sessionId: hop.session_id,
      title: hop.title || hop.session_id,
      provider: hop.provider,
      kind: hop.kind,
      stateLabel: hop.parent_state === "trashed" ? "In Trash" : hop.parent_state === "purged" ? "Archived" : undefined,
      openable: hop.parent_state === "live",
      hop,
    });
  }
  entries.push({
    key: `cur:${sessionId}`,
    section: "current",
    sessionId,
    title: currentTitle || sessionId,
    provider: currentProvider,
    kind: "current",
    stateLabel: "Current",
    openable: false,
  });
  for (const branch of branches) {
    entries.push({
      key: `br:${branch.session_id}`,
      section: branch.relation === "child" ? "child" : "sibling",
      sessionId: branch.session_id,
      title: branch.title || branch.session_id,
      provider: branch.provider,
      kind: branch.kind,
      stateLabel: branch.trashed ? "In Trash" : branch.relation === "child" ? "Branch" : "Sibling",
      openable: !branch.trashed,
    });
  }
  return entries;
}

function hopToLink(hop: LineageHop): SessionLinkRow {
  return {
    id: hop.session_id,
    kind: hop.kind === "replace" ? "continued" : "forked",
    state: hop.parent_state === "live" ? "open" : hop.parent_state === "trashed" ? "trash" : "deleted",
    title: hop.title || hop.session_id,
    ...(hop.before_seq !== null ? { before: hop.before_seq } : {}),
  };
}

function BranchListBody({
  sessionId,
  entries,
  loading,
  activeIndex,
  onSelectIndex,
  onOpen,
  onPeek,
  onNewFork,
}: {
  sessionId: string;
  entries: readonly SwitcherEntry[];
  loading: boolean;
  activeIndex: number;
  onSelectIndex: (index: number) => void;
  onOpen: (entry: SwitcherEntry) => void;
  onPeek: (hop: LineageHop) => void;
  onNewFork: () => void;
}): React.JSX.Element {
  return (
    <Stack spacing={1} data-lineage-branch-list={sessionId}>
      {loading && entries.length <= 1 && (
        <Stack direction="row" spacing={1} alignItems="center" justifyContent="center" sx={{ py: 2, color: "text.secondary" }}>
          <CircularProgress size={14} color="inherit" />
          <Typography variant="caption">Loading branches…</Typography>
        </Stack>
      )}
      {entries.map((entry, idx) => {
        const selected = idx === activeIndex;
        const Icon = entry.kind === "replace" ? SwapHorizRounded : CallSplitOutlined;
        const agent = entry.provider ? providerPresentation(entry.provider).agent : "";
        return (
          <Stack
            key={entry.key}
            direction="row"
            spacing={0.5}
            alignItems="center"
            sx={(theme) => ({
              borderRadius: 2,
              border: 1,
              borderColor: entry.section === "current"
                ? alpha(theme.palette.primary.main, 0.45)
                : selected
                  ? "primary.main"
                  : "divider",
              bgcolor: selected
                ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.08)
                : entry.section === "current"
                  ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.08 : 0.03)
                  : "transparent",
              pl: entry.section === "child" ? 2 : 0.75,
              pr: 0.5,
            })}
          >
            <ButtonBase
              onMouseEnter={(): void => onSelectIndex(idx)}
              onClick={(): void => onOpen(entry)}
              sx={{
                flex: 1,
                minWidth: 0,
                minHeight: 44,
                justifyContent: "flex-start",
                textAlign: "left",
                gap: 1,
                px: 0.75,
                py: 0.5,
                borderRadius: 1.5,
              }}
            >
              <Icon sx={{ fontSize: glyphSize(16), color: entry.section === "current" ? "primary.main" : "text.secondary", flexShrink: 0 }} />
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Typography variant="body2" noWrap sx={{ fontWeight: entry.section === "current" ? 600 : 500 }}>
                  {entry.title}
                </Typography>
                <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block" }}>
                  {[agent, entry.section === "ancestor" ? "Parent" : entry.stateLabel].filter(Boolean).join(" · ")}
                </Typography>
              </Box>
              {entry.stateLabel && (
                <Chip size="small" variant="outlined" label={entry.stateLabel} sx={{ height: 20, fontSize: "0.68rem" }} />
              )}
            </ButtonBase>
            {entry.hop && (entry.hop.parent_state === "live" || entry.hop.parent_state === "trashed") && (
              <Tooltip title="Peek read-only (p)">
                <IconButton
                  size="small"
                  aria-label="Peek ancestor"
                  onClick={(): void => {
                    if (entry.hop) onPeek(entry.hop);
                  }}
                  sx={{ minWidth: 36, minHeight: 36, color: "text.secondary" }}
                >
                  <VisibilityOutlined sx={{ fontSize: glyphSize(16) }} />
                </IconButton>
              </Tooltip>
            )}
          </Stack>
        );
      })}
      <Button
        size="small"
        variant="outlined"
        startIcon={<CallSplitOutlined sx={{ fontSize: glyphSize(16) }} />}
        onClick={onNewFork}
        sx={{ mt: 0.5, minHeight: 40, borderRadius: 2, textTransform: "none" }}
      >
        New fork or switch agent…
      </Button>
    </Stack>
  );
}

function useBranchRelationCount(sessionId: string): number {
  return useStoreSelector((state) => {
    const self = state.sessions.find((entry) => entry.id === sessionId);
    if (!self || self.system) return 0;
    let count = self.forked_from || self.continued_from ? 1 : 0;
    for (const entry of state.sessions) {
      if (entry.forked_from === sessionId || entry.continued_from === sessionId) count += 1;
    }
    return count;
  });
}

function BranchSwitcherModal({
  sessionId,
  open,
  desktop,
  onClose,
}: {
  sessionId: string;
  open: boolean;
  desktop: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const session = useStoreSelector((state) => state.sessions.find((entry) => entry.id === sessionId));
  const sessions = useStoreSelector((state) => state.sessions);
  const { lineage, loadingLineage } = useSessionLineage(sessionId, open);
  const [activeIndex, setActiveIndex] = useState(0);
  const [peekHop, setPeekHop] = useState<LineageHop | null>(null);

  const entries = useMemo(
    () =>
      buildEntries(
        sessionId,
        session?.title ?? sessionId,
        session?.provider ?? "",
        lineage?.hops ?? [],
        lineage?.branches ?? [],
      ),
    [lineage, session?.provider, session?.title, sessionId],
  );

  useEffect(() => {
    if (!open) return;
    const cur = entries.findIndex((entry) => entry.section === "current");
    setActiveIndex(cur >= 0 ? cur : 0);
  }, [entries.length, open]);

  const openEntry = useCallback(
    (entry: SwitcherEntry): void => {
      if (entry.section === "current") {
        onClose();
        return;
      }
      if (entry.openable) {
        onClose();
        requestPickSession(entry.sessionId, sessions.find((s) => s.id === entry.sessionId));
        return;
      }
      if (entry.hop) setPeekHop(entry.hop);
    },
    [onClose, sessions],
  );

  useEffect(() => {
    if (!open || !desktop || peekHop !== null) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.isComposing || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "j" || event.key === "ArrowDown") {
        event.preventDefault();
        setActiveIndex((i) => Math.min(entries.length - 1, i + 1));
      } else if (event.key === "k" || event.key === "ArrowUp") {
        event.preventDefault();
        setActiveIndex((i) => Math.max(0, i - 1));
      } else if (event.key === "Enter") {
        const chosen = entries[activeIndex];
        if (chosen) {
          event.preventDefault();
          openEntry(chosen);
        }
      } else if (event.key === "p") {
        const chosen = entries[activeIndex];
        if (chosen?.hop) {
          event.preventDefault();
          setPeekHop(chosen.hop);
        }
      } else if (event.key === "f") {
        event.preventDefault();
        onClose();
        requestNewFrom(sessionId);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeIndex, desktop, entries, onClose, open, openEntry, peekHop, sessionId]);

  const body = (
    <BranchListBody
      sessionId={sessionId}
      entries={entries}
      loading={loadingLineage}
      activeIndex={activeIndex}
      onSelectIndex={setActiveIndex}
      onOpen={openEntry}
      onPeek={setPeekHop}
      onNewFork={(): void => {
        onClose();
        requestNewFrom(sessionId);
      }}
    />
  );

  return (
    <>
      {desktop ? (
        <Dialog
          open={open}
          onClose={(e, r): void => {
            if (!isImeBlockedEscape(e, r)) onClose();
          }}
          maxWidth="xs"
          fullWidth
          disableScrollLock
          slots={{ transition: InstantModalTransition }}
          slotProps={desktopDialogSlotProps({ onClose })}
        >
          <DialogTitle sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", py: 1.5 }}>
            <Stack direction="row" spacing={1} alignItems="center">
              <AccountTreeOutlined fontSize="small" color="primary" />
              <Typography variant="subtitle1" sx={{ fontWeight: 600 }}>
                Branches & lineage
              </Typography>
            </Stack>
            <Stack direction="row" spacing={0.5} alignItems="center">
              <Kbd keys="j/k" />
              <Kbd keys="Enter" />
              <IconButton size="small" aria-label="Close" onClick={onClose}>
                <CloseRounded fontSize="small" />
              </IconButton>
            </Stack>
          </DialogTitle>
          <DialogContent sx={{ pt: 0.5, pb: 2 }}>{body}</DialogContent>
        </Dialog>
      ) : (
        <Sheet open={open} onClose={onClose} title="Branches & lineage">
          <Box sx={{ pb: 1 }}>{body}</Box>
        </Sheet>
      )}
      {peekHop && (
        <SessionHistoryPeek
          open
          onClose={(): void => setPeekHop(null)}
          link={hopToLink(peekHop)}
          session={sessions.find((s) => s.id === peekHop.session_id)}
        />
      )}
    </>
  );
}

/** Desktop top-bar branch trigger + `␣SB` command palette registration.
 *  Hidden on ordinary unbranched sessions so the toolbar stays uncluttered. */
export function DesktopBranchSwitcherTrigger({
  sessionId,
}: {
  sessionId: string;
}): React.JSX.Element | null {
  const relationCount = useBranchRelationCount(sessionId);
  const [open, setOpen] = useState(false);

  useDesktopCommand(
    useMemo(
      () => ({
        id: "session.branches",
        title: "Switch Branch or Lineage",
        description: "Jump between ancestor conversations and sibling forks",
        group: "Session",
        sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.sessionsBranches),
        surface: "session" as const,
        when: () => Boolean(sessionId),
        run: () => setOpen(true),
      }),
      [sessionId],
    ),
  );

  if (relationCount === 0) {
    return (
      <BranchSwitcherModal
        sessionId={sessionId}
        open={open}
        desktop
        onClose={(): void => setOpen(false)}
      />
    );
  }

  return (
    <>
      <Tooltip title="Switch branch or inspect conversation lineage (␣SB)">
        <span>
          <Button
            data-desktop-item="topbar-branches"
            data-desktop-topbar-action="branches"
            data-desktop-branch-switcher
            size="small"
            color="inherit"
            variant="outlined"
            startIcon={<AccountTreeOutlined sx={{ ...desktopEmbeddedControlIconSx(), color: "primary.main" }} />}
            onClick={(): void => setOpen(true)}
            sx={desktopSessionActionSx({ open, minWidth: 96 })}
          >
            <Stack direction="row" spacing={0.5} alignItems="center">
              <Typography component="span" variant="caption" sx={{ fontWeight: 600, lineHeight: 1 }}>
                Branches
              </Typography>
              <LeaderKeycap leaderKey={DESKTOP_WORKSPACE_KEYS.sessionsBranches} />
            </Stack>
          </Button>
        </span>
      </Tooltip>
      <BranchSwitcherModal
        sessionId={sessionId}
        open={open}
        desktop
        onClose={(): void => setOpen(false)}
      />
    </>
  );
}

/** Mobile bottom-workbench branch trigger (docs/ux-principles.md: operate
 *  below, >= 44px touch target). Opens a bottom Sheet preserving draft/scroll. */
export function MobileBranchSwitcherButton({
  sessionId,
}: {
  sessionId: string;
}): React.JSX.Element | null {
  const relationCount = useBranchRelationCount(sessionId);
  const [open, setOpen] = useState(false);
  if (relationCount === 0) return null;
  return (
    <>
      <Tooltip title="Branches & lineage">
        <IconButton
          aria-label="branches and lineage"
          data-mobile-branch-switcher
          onClick={(): void => setOpen(true)}
          sx={{ minWidth: 44, minHeight: 44, color: "primary.main" }}
        >
          <AccountTreeOutlined fontSize="small" />
        </IconButton>
      </Tooltip>
      <BranchSwitcherModal
        sessionId={sessionId}
        open={open}
        desktop={false}
        onClose={(): void => setOpen(false)}
      />
    </>
  );
}
