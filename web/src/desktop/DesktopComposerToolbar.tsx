import { desktopComposerToolbarSx } from "./composerToolbarLayout";
import {
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  alpha,
  Box,
  Button,
  CircularProgress,
  Divider,
  ListItemText,
  Menu,
  MenuItem,
  Tooltip,
} from "@mui/material";
import {
  CleaningServicesOutlined,
  Compress,
  EditNoteOutlined,
  KeyboardArrowUp,
  MoreVert,
  Schedule,
  Send,
  Stop,
} from "@mui/icons-material";
import type { ComposerEditorHandle } from "../ComposerEditor";
import type { Status } from "../protocol";
import { COMPOSER_COMMANDS_BY_ID } from "../composerCommands";
import {
  toggleComposerSourceMode,
  useComposerSourceMode,
} from "../composerSourceMode";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopComposerCommandBindings } from "./commands/DesktopComposerShortcuts";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { HintTooltip } from "../HintTooltip";
import { desktopOverlayOwnsShortcuts } from "./commands/desktopShortcutScope";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
} from "./commands/workspaceShortcuts";
import { useImeStatus } from "./vim/imeStatusStore";
import { desktopToolbarActionSx } from "./desktopToolbarAction";
import {
  DesktopEditorFormatRow,
  DesktopToolbarActionButton,
  type DesktopToolbarButtonSpec,
} from "./DesktopEditorFormatRow";
import {
  type DesktopSessionActions,
  useDesktopSessionActions,
} from "./useDesktopSessionActions";
import { DesktopCacheGlanceButton } from "../SessionCacheGlanceCue";

export interface DesktopComposerToolbarProps {
  sessionId: string;
  status: Status;
  editorRef: RefObject<ComposerEditorHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  canInsert: boolean;
  sendable: boolean;
  canSave?: boolean;
  canSchedule?: boolean;
  canJumpFront: boolean;
  canForce: boolean;
  canCurrentTurn?: boolean;
  onCurrentTurn?: () => void;
  pending: boolean;
  progress: boolean;
  primaryAction?: "submit" | "save";
  sendLabel: "Send" | "Send & switch" | "Queue" | "Send…" | "Enqueue" | "Save" | "Deliver…";
  sendIcon?: ReactNode;
  sendDescription: string;
  saveLabel?: string;
  saveTitle?: string;
  saveIcon?: ReactNode;
  scheduleLabel?: string;
  scheduleTitle?: string;
  unavailableReason: string;
  overlayOpen: boolean;
  onAttach: () => void;
  onSaveDraft: () => void;
  onSchedule: () => void;
  onJumpFront: () => void;
  onForce: () => void;
  onSubmit: () => void;
}

/** Desktop chrome only. The editor remains mounted as the pane is resized. */
export function DesktopComposerToolbar(
  props: DesktopComposerToolbarProps,
): React.JSX.Element {
  const session = useDesktopSessionActions(props.sessionId, props.status);
  return (
    <DesktopComposerToolbarView
      {...props}
      session={session}
      cacheCue={<DesktopCacheGlanceButton sessionId={props.sessionId} />}
    />
  );
}

function hasOverlayMutation(records: readonly MutationRecord[]): boolean {
  const selector = ".MuiModal-root, .MuiPopper-root, [role='dialog'], [role='menu'], [role='listbox']";
  for (const record of records) {
    for (const node of [...record.addedNodes, ...record.removedNodes]) {
      if (node instanceof Element && (node.matches(selector) || node.querySelector(selector) !== null)) {
        return true;
      }
    }
  }
  return false;
}

/** The toolbar for already resolved conversation actions (fixtures render it
 *  directly, with a Compact reading the catalog would otherwise supply). */
export function DesktopComposerToolbarView(
  props: Omit<DesktopComposerToolbarProps, "sessionId" | "status"> & {
    session: DesktopSessionActions;
    cacheCue?: ReactNode;
  },
): React.JSX.Element {
  const { session } = props;
  const workspace = useDesktopWorkspace();
  const sourceMode = useComposerSourceMode();
  const composing = useImeStatus().phase === "composing";
  const moreRef = useRef<HTMLButtonElement>(null);
  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null);
  const [sessionAnchor, setSessionAnchor] = useState<HTMLElement | null>(
    null,
  );
  const [overlayOwnsKeys, setOverlayOwnsKeys] = useState(false);
  useEffect(() => {
    const update = (): void => setOverlayOwnsKeys(desktopOverlayOwnsShortcuts(document));
    // MUI portals are body children. Ignore tooltips/non-overlay mutations.
    const observer = new MutationObserver((records) => { if (hasOverlayMutation(records)) update(); });
    observer.observe(document.body, { childList: true });
    document.addEventListener("focusin", update);
    update();
    return () => {
      observer.disconnect();
      document.removeEventListener("focusin", update);
    };
  }, []);
  const prefixAvailable = !overlayOwnsKeys && !props.overlayOpen && !composing;
  const scoped = workspace.focusedRegion === "prompt.composer" &&
    prefixAvailable;
  const sourceScope = workspace.focusedPane === "prompt" && prefixAvailable;
  const ready = props.sendable && !props.pending;
  const deliveryReason = props.pending
    ? "Saving this prompt…"
    : props.unavailableReason;
  const format = (id: string): void => {
    if (composing) return;
    const editor = props.editorRef.current;
    if (editor) {
      COMPOSER_COMMANDS_BY_ID[id]?.run({ editor, attach: props.onAttach });
    }
  };
  const action = (spec: DesktopToolbarButtonSpec): ReactNode => (
    <DesktopToolbarActionButton
      key={spec.id}
      spec={spec}
      scoped={scoped}
      sourceScope={sourceScope}
      prefixAvailable={prefixAvailable}
      overlayOwnsKeys={overlayOwnsKeys}
    />
  );
  const saveReady = (props.canSave ?? ready) && !props.pending;
  const primaryIsSave = props.primaryAction === "save";
  const primaryReady = primaryIsSave ? saveReady : ready;
  const scheduleReady = (props.canSchedule ?? true) && ready;
  return (
    <Box
      data-desktop-composer-toolbar
      sx={desktopComposerToolbarSx}
    >
      <DesktopComposerCommandBindings
        sendable={ready}
        canSave={saveReady}
        canSchedule={props.canSchedule ?? true}
        canInsert={props.canInsert}
        canAttach={props.canInsert}
        canJumpFront={props.canJumpFront}
        canCurrentTurn={props.canCurrentTurn ?? false}
        onCurrentTurn={props.onCurrentTurn ?? (() => {})}
        canForce={props.canForce}
        canMore
        onSubmit={props.onSubmit}
        onSlash={() => props.editorRef.current?.insertTrigger("/")}
        onReference={() => props.editorRef.current?.insertTrigger("@")}
        onAttach={props.onAttach}
        onSaveDraft={props.onSaveDraft}
        onSchedule={props.onSchedule}
        onJumpFront={props.onJumpFront}
        onForce={props.onForce}
        onMore={() => setMoreAnchor(moreRef.current)}
        onFormat={format}
      />
      <DesktopEditorFormatRow
        kind="session"
        ariaLabel="Prompt editing"
        menuId="desktop-composer-formatting"
        canInsert={props.canInsert}
        canAttach={props.canInsert}
        composing={composing}
        sourceMode={sourceMode}
        scoped={scoped}
        sourceScope={sourceScope}
        prefixAvailable={prefixAvailable}
        overlayOwnsKeys={overlayOwnsKeys}
        moreRef={moreRef}
        moreAnchor={moreAnchor}
        onMoreOpen={setMoreAnchor}
        onMoreClose={() => setMoreAnchor(null)}
        onSlash={() => props.editorRef.current?.insertTrigger("/")}
        onReference={() => props.editorRef.current?.insertTrigger("@")}
        onAttach={props.onAttach}
        onFormat={format}
        onToggleSourceMode={() => toggleComposerSourceMode()}
      />
      <Box
        data-composer-delivery
        role="group"
        aria-label="Prompt delivery"
        sx={{
          display: "flex",
          alignItems: "flex-end",
          gap: 1,
          px: 1,
          pt: 0.25,
          pb: 0.75,
        }}
      >
        <Box
          sx={{
            display: "flex",
            flex: 1,
            minWidth: 0,
            flexWrap: "wrap",
            gap: 0.25,
          }}
        >
          {primaryIsSave
            ? action({
              id: "deliver",
              icon: <Send />,
              label: "Deliver…",
              title: "Choose delivery mode (Steer, Queue first)",
              shortcut: "Mod+Enter",
              disabled: !ready,
              reason: deliveryReason,
              onClick: props.onSubmit,
            })
            : action({
              id: "draft",
              icon: props.saveIcon ?? <EditNoteOutlined />,
              label: props.saveLabel ?? "Draft",
              title: props.saveTitle ?? "Save as draft",
              shortcut: DESKTOP_SHORTCUTS.saveDraft,
              disabled: !saveReady,
              reason: deliveryReason,
              onClick: props.onSaveDraft,
            })}
          {action({
            id: "schedule",
            icon: <Schedule />,
            label: props.scheduleLabel ?? "Schedule",
            title: props.scheduleTitle ?? "Schedule send",
            key: DESKTOP_WORKSPACE_KEYS.composerSchedule,
            disabled: !scheduleReady,
            reason: deliveryReason,
            onClick: props.onSchedule,
          })}
          {(session.compact || session.clear) && (
            <Box data-session-inline>
              <Divider
                orientation="vertical"
                flexItem
                sx={{ my: 0.75, mx: 0.5 }}
              />
              {session.compact && action({
                id: "compact",
                icon: <ContextReading {...session.compact} />,
                label: "Compact",
                title: session.compact.title,
                shortcut: DESKTOP_SHORTCUTS.compact,
                disabled: session.compact.running,
                reason: "Compacting…",
                onClick: session.compact.open,
              })}
              {session.clear && action({
                id: "clear",
                icon: <CleaningServicesOutlined />,
                label: "Clear",
                title: "Clear conversation · start with a fresh context",
                shortcut: DESKTOP_SHORTCUTS.clear,
                onClick: session.clear.open,
              })}
            </Box>
          )}
          {(session.compact || session.clear) && (
            <Box data-session-overflow>
              <HintTooltip title="Compact or clear the conversation">
                <Button
                  aria-label="Conversation actions"
                  aria-haspopup="menu"
                  aria-expanded={sessionAnchor ? "true" : undefined}
                  aria-keyshortcuts={conversationActionKeyshortcuts(session)}
                  data-composer-action="conversation"
                  size="small"
                  color="inherit"
                  onPointerDown={(event) => {
                    if (event.button === 0) event.preventDefault();
                  }}
                  onClick={(event) => setSessionAnchor(event.currentTarget)}
                  sx={desktopToolbarActionSx}
                >
                  <ConversationMenuGlyph compact={session.compact} />
                </Button>
              </HintTooltip>
            </Box>
          )}
          {props.cacheCue}
        </Box>
        {
          /* Stop stays mounted and only disables while idle, so the row does
            not move as turns start and finish; Send keeps the edge. */
        }
        {action({
          id: "stop",
          icon: <Stop />,
          label: "Stop",
          title: "Stop current turn",
          shortcut: DESKTOP_SHORTCUTS.stop,
          anywhere: true,
          tone: "error",
          held: !session.stop.busy,
          onClick: session.stop.open,
        })}
        <Tooltip title={primaryReady ? props.sendDescription : deliveryReason}>
          <span>
            <Button
              ref={props.sendButtonRef}
              variant="contained"
              size="small"
              disableElevation
              data-composer-action="send"
              aria-label={primaryIsSave
                ? "save queued changes"
                : props.sendLabel === "Queue"
                ? "queue message"
                : "send"}
              aria-haspopup={!primaryIsSave &&
                  (props.sendLabel === "Send…" || props.sendLabel === "Deliver…")
                ? "menu"
                : undefined}
              disabled={!primaryReady}
              aria-busy={props.pending || undefined}
              onPointerDown={(event) => {
                if (event.button === 0) event.preventDefault();
              }}
              onClick={primaryIsSave ? props.onSaveDraft : props.onSubmit}
              sx={{
                minHeight: "2rem",
                minWidth: "6.5rem",
                px: 1,
                gap: 0.75,
                borderRadius: 1,
                textTransform: "none",
                fontWeight: 650,
                "& kbd": {
                  color: "inherit",
                  borderColor: "currentColor",
                  bgcolor: "transparent",
                  opacity: 0.65,
                },
              }}
            >
              {props.progress
                ? <CircularProgress size="1rem" color="inherit" />
                : (props.sendIcon ?? <Send sx={{ fontSize: "1rem" }} />)}
              {props.sendLabel}
              <DesktopShortcut
                shortcut={primaryIsSave ? DESKTOP_SHORTCUTS.saveDraft : "Mod+Enter"}
                quiet
                compact
                availability={shortcutAvailability(scoped && primaryReady)}
              />
            </Button>
          </span>
        </Tooltip>
      </Box>
      {session.dialogs}
      {sessionAnchor !== null && <Menu
        anchorEl={sessionAnchor}
        open
        onClose={() => setSessionAnchor(null)}
        anchorOrigin={{ vertical: "top", horizontal: "left" }}
        transformOrigin={{ vertical: "bottom", horizontal: "left" }}
      >
        {session.compact && (
          <MenuItem
            disabled={session.compact.running}
            onClick={() => {
              setSessionAnchor(null);
              session.compact?.open();
            }}
          >
            <ListItemText>Compact conversation</ListItemText>
            <Box sx={{ ml: 2 }} data-shortcut-reference>
              <DesktopShortcut
                shortcut={DESKTOP_SHORTCUTS.compact}
                quiet
                compact
                availability="inactive"
              />
            </Box>
          </MenuItem>
        )}
        {session.clear && (
          <MenuItem
            onClick={() => {
              setSessionAnchor(null);
              session.clear?.open();
            }}
          >
            <ListItemText>Clear conversation</ListItemText>
            <Box sx={{ ml: 2 }} data-shortcut-reference>
              <DesktopShortcut
                shortcut={DESKTOP_SHORTCUTS.clear}
                quiet
                compact
                availability="inactive"
              />
            </Box>
          </MenuItem>
        )}
      </Menu>}
    </Box>
  );
}

function contextPercent(
  { used, size }: { used: number; size: number },
): number | null {
  if (!(size > 0)) return null;
  return used > 0 ? Math.max(1, Math.round(Math.min(100, used / size * 100))) : 0;
}

/** Compact's face at every width: the gauge with its percentage beside it. */
function ContextReading(
  { used, size, stale, running }: {
    used: number;
    size: number;
    stale: boolean;
    running: boolean;
  },
): React.JSX.Element {
  // A reading a compaction left as history is unknown: an empty ring, "—".
  const pct = stale ? null : contextPercent({ used, size });
  return (
    <>
      <ContextGauge
        used={stale ? 0 : used}
        size={stale ? 1 : size}
        running={running}
      />
      {(pct !== null || stale) && (
        <Box
          component="span"
          data-context-reading
          data-context-stale={stale || undefined}
          sx={{
            fontVariantNumeric: "tabular-nums",
            color: stale ? "text.disabled" : undefined,
          }}
        >
          {stale ? "—" : `${pct}%`}
        </Box>
      )}
    </>
  );
}

function conversationActionKeyshortcuts(
  session: DesktopSessionActions,
): string {
  return [
    session.compact ? DESKTOP_SHORTCUTS.compact : "",
    session.clear ? DESKTOP_SHORTCUTS.clear : "",
  ].filter(Boolean).join(" ");
}

/** The folded menu keeps the context reading it took over from Compact. */
function ConversationMenuGlyph(
  { compact }: { compact: DesktopSessionActions["compact"] },
): React.JSX.Element {
  if (!compact) return <MoreVert />;
  return (
    <>
      <ContextReading {...compact} />
      <KeyboardArrowUp sx={{ ml: "-0.125rem" }} />
    </>
  );
}

/** Context fullness at the toolbar's icon size and weight: a thin neutral
 *  ring that turns amber at 70% and red at 90% (the auto-compaction zone).
 *  The number sits beside it as text, not inside it. */
function ContextGauge(
  { used, size, running }: { used: number; size: number; running: boolean },
): React.JSX.Element {
  const pct = contextPercent({ used, size });
  if (running) {
    return <CircularProgress size="0.875rem" thickness={5} color="inherit" />;
  }
  if (pct === null) return <Compress />;
  const color = pct >= 90
    ? "error.main"
    : pct >= 70
    ? "warning.main"
    : "text.secondary";
  return (
    <Box
      component="svg"
      viewBox="0 0 16 16"
      aria-hidden="true"
      sx={(theme) => ({
        width: "0.875rem",
        height: "0.875rem",
        flexShrink: 0,
        color,
        "& .context-gauge-track": {
          stroke: alpha(theme.palette.text.secondary, 0.42),
        },
      })}
    >
      <circle
        className="context-gauge-track"
        cx="8"
        cy="8"
        r="6.25"
        fill="none"
        strokeWidth="1.75"
      />
      {pct > 0 && (
        <circle
          cx="8"
          cy="8"
          r="6.25"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap={pct < 100 ? "round" : undefined}
          pathLength="100"
          strokeDasharray={pct < 100 ? `${pct} ${100 - pct}` : undefined}
          transform="rotate(-90 8 8)"
        />
      )}
    </Box>
  );
}
