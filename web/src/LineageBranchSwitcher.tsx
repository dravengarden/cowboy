import {
  Box,
  Button,
  ButtonBase,
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
import ChevronRightRounded from "@mui/icons-material/ChevronRightRounded";
import CloseRounded from "@mui/icons-material/CloseRounded";
import SwapHorizRounded from "@mui/icons-material/SwapHorizRounded";
import VisibilityOutlined from "@mui/icons-material/VisibilityOutlined";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { LineageBranch, LineageHop } from "./ancestorHistory";
import { releaseMobileComposerFocus } from "./composer/mobileComposerFocus";
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
  badge?: string | undefined;
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
      badge: hop.parent_state === "trashed" ? "In Trash" : hop.parent_state === "purged" ? "Archived" : undefined,
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
    badge: "Current",
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
      badge: branch.trashed ? "In Trash" : undefined,
      openable: !branch.trashed,
    });
  }
  return entries;
}

function isHopPeekable(hop: LineageHop | undefined): hop is LineageHop {
  return Boolean(hop && (hop.history_state === "live" || hop.history_state === "archived"));
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

function entrySubtitle(entry: SwitcherEntry): string {
  const agent = entry.provider ? providerPresentation(entry.provider).agent : "";
  if (entry.section === "current") return agent || "Active session";
  if (entry.section === "ancestor") {
    const role = entry.kind === "replace" ? "Continued from" : "Forked from";
    return [role, agent].filter(Boolean).join(" · ");
  }
  const role = entry.section === "child"
    ? entry.kind === "replace" ? "Continuation" : "Branch"
    : entry.kind === "replace" ? "Sibling handoff" : "Sibling branch";
  return [role, agent].filter(Boolean).join(" · ");
}

function EntryBadge({ label, active }: { label: string; active: boolean }): React.JSX.Element {
  return (
    <Typography
      component="span"
      variant="caption"
      sx={(theme) => ({
        flexShrink: 0,
        px: 0.9,
        borderRadius: "999px",
        lineHeight: "1.35rem",
        fontSize: "0.7rem",
        fontWeight: active ? 600 : 500,
        color: active ? "primary.main" : "text.secondary",
        bgcolor: active
          ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.22 : 0.12)
          : theme.palette.action.hover,
      })}
    >
      {label}
    </Typography>
  );
}

function LineageNodeRail({
  entry,
  hasAbove,
  hasBelow,
}: {
  entry: SwitcherEntry;
  hasAbove: boolean;
  hasBelow: boolean;
}): React.JSX.Element {
  const isCurrent = entry.section === "current";
  const Icon = entry.kind === "replace" ? SwapHorizRounded : CallSplitOutlined;
  return (
    <Box
      aria-hidden
      data-lineage-rail-node
      sx={{
        position: "relative",
        width: 24,
        alignSelf: "stretch",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
      }}
    >
      {hasAbove && (
        <Box
          sx={(theme) => ({
            position: "absolute",
            top: -4,
            bottom: "50%",
            width: "1.5px",
            bgcolor: alpha(theme.palette.text.primary, theme.palette.mode === "dark" ? 0.16 : 0.12),
          })}
        />
      )}
      {hasBelow && (
        <Box
          sx={(theme) => ({
            position: "absolute",
            top: "50%",
            bottom: -4,
            width: "1.5px",
            bgcolor: alpha(theme.palette.text.primary, theme.palette.mode === "dark" ? 0.16 : 0.12),
          })}
        />
      )}
      <Box
        sx={(theme) => ({
          position: "relative",
          zIndex: 1,
          width: 22,
          height: 22,
          borderRadius: "999px",
          display: "grid",
          placeItems: "center",
          bgcolor: isCurrent
            ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.24 : 0.14)
            : theme.palette.background.paper,
          border: isCurrent ? "none" : `1px solid ${theme.palette.divider}`,
        })}
      >
        {isCurrent ? (
          <Box sx={{ width: 8, height: 8, borderRadius: "999px", bgcolor: "primary.main" }} />
        ) : (
          <Icon sx={{ fontSize: glyphSize(13), color: "text.secondary" }} />
        )}
      </Box>
    </Box>
  );
}

function BranchRow({
  entry,
  idx,
  total,
  desktop,
  selected,
  onSelect,
  onOpen,
  onPeek,
}: {
  entry: SwitcherEntry;
  idx: number;
  total: number;
  desktop: boolean;
  selected: boolean;
  onSelect: (index: number) => void;
  onOpen: (entry: SwitcherEntry) => void;
  onPeek: (hop: LineageHop) => void;
}): React.JSX.Element {
  const isCurrent = entry.section === "current";
  const keyboardSelected = desktop && selected;
  const peekable = isHopPeekable(entry.hop);
  const interactive = isCurrent || entry.openable || peekable;
  const rowInner = (
    <>
      <LineageNodeRail entry={entry} hasAbove={idx > 0} hasBelow={idx < total - 1} />
      <Box sx={{ flex: 1, minWidth: 0, py: 0.75 }}>
        <Typography
          variant="body2"
          noWrap
          sx={{
            fontWeight: isCurrent ? 600 : 500,
            lineHeight: 1.35,
            color: interactive ? "text.primary" : "text.secondary",
          }}
        >
          {entry.title}
        </Typography>
        <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block", lineHeight: 1.35 }}>
          {entrySubtitle(entry)}
        </Typography>
      </Box>
      {entry.badge && <EntryBadge label={entry.badge} active={isCurrent} />}
      {entry.openable && !entry.badge && (
        <ChevronRightRounded sx={{ fontSize: glyphSize(18), color: "text.disabled", flexShrink: 0 }} />
      )}
    </>
  );
  const mainSx = {
    flex: 1,
    minWidth: 0,
    minHeight: desktop ? 46 : 54,
    display: "flex",
    alignItems: "center",
    justifyContent: "flex-start",
    textAlign: "left" as const,
    gap: 1.25,
    pl: 1,
    pr: peekable ? 0.5 : 1.25,
    borderRadius: "12px",
  };
  return (
    <Stack
      component="li"
      direction="row"
      alignItems="center"
      sx={(theme) => ({
        borderRadius: "12px",
        bgcolor: keyboardSelected
          ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.09)
          : isCurrent
            ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.13 : 0.07)
            : "transparent",
        boxShadow: keyboardSelected
          ? `inset 0 0 0 1px ${alpha(theme.palette.primary.main, 0.38)}`
          : "none",
        transition: "background-color 120ms ease",
        ...(interactive && desktop
          ? {
              "&:hover": {
                bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.14 : 0.07),
              },
            }
          : {}),
      })}
    >
      {interactive ? (
        <ButtonBase
          onMouseEnter={desktop ? (): void => onSelect(idx) : undefined}
          onClick={(): void => onOpen(entry)}
          sx={mainSx}
        >
          {rowInner}
        </ButtonBase>
      ) : (
        <Box sx={{ ...mainSx, opacity: 0.78 }}>{rowInner}</Box>
      )}
      {peekable && (
        <Tooltip title="Peek read-only (p)">
          <IconButton
            size="small"
            aria-label="Peek ancestor"
            onClick={(): void => {
              if (entry.hop) onPeek(entry.hop);
            }}
            sx={{
              width: desktop ? 34 : 40,
              height: desktop ? 34 : 40,
              mr: 0.5,
              color: "text.secondary",
              "& .MuiSvgIcon-root": { fontSize: "1.125rem" },
            }}
          >
            <VisibilityOutlined />
          </IconButton>
        </Tooltip>
      )}
    </Stack>
  );
}

function BranchListBody({
  sessionId,
  desktop,
  entries,
  loading,
  activeIndex,
  onSelectIndex,
  onOpen,
  onPeek,
  onNewFork,
}: {
  sessionId: string;
  desktop: boolean;
  entries: readonly SwitcherEntry[];
  loading: boolean;
  activeIndex: number;
  onSelectIndex: (index: number) => void;
  onOpen: (entry: SwitcherEntry) => void;
  onPeek: (hop: LineageHop) => void;
  onNewFork: () => void;
}): React.JSX.Element {
  return (
    <Box data-lineage-branch-list={sessionId}>
      {loading && entries.length <= 1 && (
        <Stack direction="row" spacing={1} alignItems="center" justifyContent="center" sx={{ py: 2, color: "text.secondary" }}>
          <CircularProgress size={14} color="inherit" />
          <Typography variant="caption">Loading branches…</Typography>
        </Stack>
      )}
      <Stack component="ul" spacing={0.5} sx={{ listStyle: "none", m: 0, p: 0 }}>
        {entries.map((entry, idx) => (
          <BranchRow
            key={entry.key}
            entry={entry}
            idx={idx}
            total={entries.length}
            desktop={desktop}
            selected={idx === activeIndex}
            onSelect={onSelectIndex}
            onOpen={onOpen}
            onPeek={onPeek}
          />
        ))}
      </Stack>
      <Box sx={{ mt: 1.25, pt: 1.25, borderTop: 1, borderColor: "divider" }}>
        <Button
          fullWidth
          variant="text"
          startIcon={<CallSplitOutlined />}
          onClick={onNewFork}
          sx={(theme) => ({
            minHeight: 44,
            borderRadius: "12px",
            textTransform: "none",
            fontWeight: 600,
            color: "primary.main",
            bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.14 : 0.08),
            "&:hover": {
              bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.2 : 0.13),
            },
          })}
        >
          New fork or switch agent…
        </Button>
      </Box>
    </Box>
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

function useBranchModalKeys({
  active,
  entries,
  activeIndex,
  setActiveIndex,
  onOpen,
  onPeek,
  onNewFork,
}: {
  active: boolean;
  entries: readonly SwitcherEntry[];
  activeIndex: number;
  setActiveIndex: React.Dispatch<React.SetStateAction<number>>;
  onOpen: (entry: SwitcherEntry) => void;
  onPeek: (hop: LineageHop) => void;
  onNewFork: () => void;
}): void {
  useEffect(() => {
    if (!active) return undefined;
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
        if (chosen) { event.preventDefault(); onOpen(chosen); }
      } else if (event.key === "p") {
        const chosen = entries[activeIndex];
        if (isHopPeekable(chosen?.hop)) { event.preventDefault(); onPeek(chosen.hop); }
      } else if (event.key === "f") {
        event.preventDefault();
        onNewFork();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, activeIndex, entries, onNewFork, onOpen, onPeek, setActiveIndex]);
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
      if (entry.section === "current") { onClose(); return; }
      if (entry.openable) {
        onClose();
        requestPickSession(entry.sessionId, sessions.find((s) => s.id === entry.sessionId));
        return;
      }
      if (isHopPeekable(entry.hop)) setPeekHop(entry.hop);
    },
    [onClose, sessions],
  );

  const handleNewFork = useCallback((): void => {
    onClose();
    requestNewFrom(sessionId);
  }, [onClose, sessionId]);

  useBranchModalKeys({
    active: open && desktop && peekHop === null,
    entries,
    activeIndex,
    setActiveIndex,
    onOpen: openEntry,
    onPeek: setPeekHop,
    onNewFork: handleNewFork,
  });

  const body = (
    <BranchListBody
      sessionId={sessionId}
      desktop={desktop}
      entries={entries}
      loading={loadingLineage}
      activeIndex={activeIndex}
      onSelectIndex={setActiveIndex}
      onOpen={openEntry}
      onPeek={setPeekHop}
      onNewFork={handleNewFork}
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
        <Sheet open={open} onClose={onClose} title="Branches & lineage" forceSheet mobileDismiss="footer" portal>
          <Box sx={{ pt: 0.5, pb: 0.5 }}>{body}</Box>
        </Sheet>
      )}
      {peekHop && (
        <SessionHistoryPeek
          open
          onClose={(): void => setPeekHop(null)}
          link={hopToLink(peekHop)}
          session={sessions.find((s) => s.id === peekHop.session_id)}
          lineage={{ childSessionId: sessionId, hopId: peekHop.session_id, edgeVersion: peekHop.edge_version, beforeSeq: peekHop.before_seq }}
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
 *  below, >= 44px touch target). Opens a portaled bottom Sheet with footer
 *  dismiss while preserving draft/scroll. */
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
          onPointerDown={(event): void => {
            if (event.pointerType === "touch") releaseMobileComposerFocus();
          }}
          onClick={(event): void => {
            releaseMobileComposerFocus();
            setOpen(true);
            event.currentTarget.blur();
          }}
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
