import { desktopComposerToolbarSx } from "./composerToolbarLayout";
import {
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import { EditorPluginToolbar } from "../editorPlugins/EditorPluginToolbar";
import {
  alpha,
  Box,
  Button,
  CircularProgress,
  Divider,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Tooltip,
} from "@mui/material";
import {
  AlternateEmail,
  AttachFile,
  CleaningServicesOutlined,
  Compress,
  EditNoteOutlined,
  MoreHoriz,
  MoreVert,
  KeyboardArrowUp,
  RawOn,
  Schedule,
  Send,
  Stop,
} from "@mui/icons-material";
import type { ComposerEditorHandle } from "../ComposerEditor";
import type { Status } from "../protocol";
import {
  COMPOSER_COMMANDS,
  COMPOSER_COMMANDS_BY_ID,
} from "../composerCommands";
import {
  toggleComposerSourceMode,
  useComposerSourceMode,
} from "../composerSourceMode";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopComposerCommandBindings } from "./commands/DesktopComposerShortcuts";
import { DesktopShortcut, LeaderKeycap } from "./commands/DesktopKeycap";
import { HintTooltip } from "../HintTooltip";
import { desktopOverlayOwnsShortcuts } from "./commands/desktopShortcutScope";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";
import { useImeStatus } from "./vim/imeStatusStore";
import { desktopToolbarActionSx } from "./desktopToolbarAction";
import {
  type DesktopSessionActions,
  useDesktopSessionActions,
} from "./useDesktopSessionActions";

const QUICK_FORMATS = ["bold", "italic", "code", "link", "bulletList"];
const FORMAT_COMMANDS = COMPOSER_COMMANDS.filter((command) =>
  !["slash", "mention", "attach", "sourceMode"].includes(command.id)
);

export interface DesktopComposerToolbarProps {
  sessionId: string;
  status: Status;
  editorRef: RefObject<ComposerEditorHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  canInsert: boolean;
  sendable: boolean;
  canJumpFront: boolean;
  canForce: boolean;
  canCurrentTurn?: boolean;
  onCurrentTurn?: () => void;
  pending: boolean;
  progress: boolean;
  sendLabel: "Send" | "Queue" | "Send…" | "Enqueue";
  sendDescription: string;
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
  return <DesktopComposerToolbarView {...props} session={session} />;
}

/** The toolbar for already resolved conversation actions (fixtures render it
 *  directly, with a Compact reading the catalog would otherwise supply). */
export function DesktopComposerToolbarView(
  props: Omit<DesktopComposerToolbarProps, "sessionId" | "status"> & {
    session: DesktopSessionActions;
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
    const update = (): void =>
      setOverlayOwnsKeys(desktopOverlayOwnsShortcuts(document));
    // MUI portals are body children. Do not observe editor mutations/keystrokes.
    const observer = new MutationObserver(update);
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
  const action = ({
    id,
    icon,
    label,
    title = label,
    key,
    shortcut,
    disabled = false,
    reason,
    onClick,
    formatTier,
    pressed,
    anywhere = false,
    tone,
    reading,
    held = false,
  }: {
    id: string;
    icon: ReactNode;
    label: string;
    title?: string;
    key?: string;
    shortcut?: string;
    disabled?: boolean;
    reason?: string;
    onClick: () => void;
    formatTier?: "primary" | "secondary";
    pressed?: boolean;
    /** The key runs from any focus (a Prompt-group or global command). */
    anywhere?: boolean;
    /** Error marks Stop, and Clear on hover. */
    tone?: "error" | "error-hover";
    /** A short value that stays visible at every width (Compact's 51%). */
    reading?: string;
    /** Keep the slot's width but show nothing: idle Stop, so the row does
     *  not move when a turn starts. */
    held?: boolean;
  }): ReactNode => (
    <HintTooltip
      key={id}
      title={title}
      shortcut={key ? desktopWorkspaceSequence(key) : shortcut}
      detail={disabled ? reason : undefined}
    >
      <Box
        component="span"
        data-format-tier={formatTier}
        aria-hidden={held || undefined}
        sx={{
          display: "inline-flex",
          flexShrink: 0,
          ...(held && { visibility: "hidden" }),
        }}
      >
        <Button
          data-composer-action={id}
          aria-label={title}
          aria-pressed={pressed}
          disabled={disabled || held}
          tabIndex={held ? -1 : undefined}
          onPointerDown={(event) => {
            if (event.button === 0) event.preventDefault();
          }}
          onClick={onClick}
          color={id === "force"
            ? "warning"
            : tone === "error"
            ? "error"
            : pressed
            ? "primary"
            : "inherit"}
          size="small"
          sx={{
            ...desktopToolbarActionSx,
            bgcolor: pressed ? "action.selected" : "transparent",
            ...(tone === "error-hover" && {
              "&:hover": { color: "error.main" },
            }),
          }}
        >
          {icon}
          {reading && (
            <Box
              component="span"
              data-composer-action-reading
              sx={{ fontVariantNumeric: "tabular-nums" }}
            >
              {reading}
            </Box>
          )}
          <Box component="span" data-composer-action-label>{label}</Box>
          {key
            ? (
              <LeaderKeycap
                leaderKey={key}
                scopeAvailable={(id === "source"
                  ? sourceScope
                  : anywhere
                  ? prefixAvailable
                  : scoped) &&
                  !disabled}
              />
            )
            : shortcut && (
              <DesktopShortcut
                shortcut={shortcut}
                compact
                quiet
                availability={shortcutAvailability(
                  (anywhere ? !overlayOwnsKeys : scoped) && !disabled,
                )}
              />
            )}
        </Button>
      </Box>
    </HintTooltip>
  );
  return (
    <Box
      data-desktop-composer-toolbar
      sx={desktopComposerToolbarSx}
    >
      <DesktopComposerCommandBindings
        sendable={ready}
        canInsert={props.canInsert}
        canAttach={props.canInsert}
        canJumpFront={props.canJumpFront}
        canCurrentTurn={props.canCurrentTurn ?? false}
        onCurrentTurn={props.onCurrentTurn ?? (() => {})}
        canForce={props.canForce}
        canMore
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
      <Box
        role="group"
        aria-label="Prompt editing"
        sx={{
          display: "flex",
          alignItems: "center",
          flexWrap: "wrap",
          gap: 0.25,
          px: 1,
          pt: 0.5,
          pb: 0.25,
        }}
      >
        <Box data-composer-insert sx={{ display: "flex", gap: 0.25 }}>
          {action({
            id: "slash",
            icon: (
              <Box
                component="span"
                sx={{
                  fontSize: "1.125rem",
                  width: "1.125rem",
                  fontWeight: 700,
                }}
              >
                /
              </Box>
            ),
            label: "Commands",
            title: "Slash command / skill",
            key: DESKTOP_WORKSPACE_KEYS.composerSlash,
            disabled: !props.canInsert || composing,
            reason: composing
              ? "Finish text composition first"
              : "Resume this session to use completions",
            onClick: () => props.editorRef.current?.insertTrigger("/"),
          })}
          {action({
            id: "reference",
            icon: <AlternateEmail />,
            label: "Files",
            title: "Reference a file",
            key: DESKTOP_WORKSPACE_KEYS.composerReference,
            disabled: !props.canInsert || composing,
            reason: composing
              ? "Finish text composition first"
              : "Resume this session to use completions",
            onClick: () => props.editorRef.current?.insertTrigger("@"),
          })}
          {action({
            id: "attach",
            icon: <AttachFile />,
            label: "Attach",
            title: "Attach image or file",
            key: DESKTOP_WORKSPACE_KEYS.composerAttach,
            disabled: !props.canInsert,
            reason: "Resume this session to attach files",
            onClick: props.onAttach,
          })}
        </Box>
        <Divider orientation="vertical" flexItem sx={{ my: 0.75, mx: 0.5 }} />
        {QUICK_FORMATS.map((id, index) => {
          const command = COMPOSER_COMMANDS_BY_ID[id]!;
          return action({
            id,
            icon: command.icon,
            label: command.label,
            shortcut: DESKTOP_COMPOSER_FORMAT_CHORDS[id]!,
            disabled: composing,
            reason: "Finish text composition first",
            formatTier: index < 3 ? "primary" : "secondary",
            onClick: () => format(id),
          });
        })}
        <HintTooltip
          title="More formatting"
          shortcut={desktopWorkspaceSequence(
            DESKTOP_WORKSPACE_KEYS.composerMore,
          )}
        >
          <Button
            ref={moreRef}
            aria-label="More formatting"
            aria-haspopup="menu"
            aria-controls={moreAnchor
              ? "desktop-composer-formatting"
              : undefined}
            aria-expanded={moreAnchor ? "true" : undefined}
            data-composer-action="more"
            size="small"
            color="inherit"
            onPointerDown={(event) => {
              if (event.button === 0) event.preventDefault();
            }}
            onClick={(event) => setMoreAnchor(event.currentTarget)}
            sx={desktopToolbarActionSx}
          >
            <MoreHoriz />
            <LeaderKeycap
              leaderKey={DESKTOP_WORKSPACE_KEYS.composerMore}
              scopeAvailable={scoped}
              {...(moreAnchor ? { availability: "active" as const } : {})}
            />
          </Button>
        </HintTooltip>
        <EditorPluginToolbar kind="session" disabled={composing} />
        <Box sx={{ flex: 1 }} />
        {action({
          id: "source",
          icon: <RawOn />,
          label: "Source",
          title: sourceMode
            ? "Switch to live preview"
            : "Switch to Source mode",
          key: DESKTOP_WORKSPACE_KEYS.toggleSourceMode,
          pressed: sourceMode,
          disabled: composing,
          reason: "Finish text composition first",
          onClick: () => {
            toggleComposerSourceMode();
          },
        })}
      </Box>
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
          {action({
            id: "draft",
            icon: <EditNoteOutlined />,
            label: "Draft",
            title: "Save as draft",
            shortcut: DESKTOP_SHORTCUTS.saveDraft,
            disabled: !ready,
            reason: deliveryReason,
            onClick: props.onSaveDraft,
          })}
          {action({
            id: "schedule",
            icon: <Schedule />,
            label: "Schedule",
            title: "Schedule send",
            key: DESKTOP_WORKSPACE_KEYS.composerSchedule,
            disabled: !ready,
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
                icon: (
                  <ContextGauge
                    used={session.compact.used}
                    size={session.compact.size}
                    running={session.compact.running}
                  />
                ),
                ...(contextPercent(session.compact) !== null && {
                  reading: `${String(contextPercent(session.compact))}%`,
                }),
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
                tone: "error-hover",
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
                  data-composer-action="conversation"
                  size="small"
                  color="inherit"
                  onPointerDown={(event) => {
                    if (event.button === 0) event.preventDefault();
                  }}
                  onClick={(event) => setSessionAnchor(event.currentTarget)}
                  sx={desktopToolbarActionSx}
                >
                  {
                    /* The context reading travels with the folded actions. */
                  }
                  {session.compact
                    ? (
                      <>
                        <ContextGauge
                          used={session.compact.used}
                          size={session.compact.size}
                          running={session.compact.running}
                        />
                        {contextPercent(session.compact) !== null && (
                          <Box
                            component="span"
                            data-context-reading
                            sx={{ fontVariantNumeric: "tabular-nums" }}
                          >
                            {contextPercent(session.compact)}%
                          </Box>
                        )}
                        <KeyboardArrowUp sx={{ ml: "-0.125rem" }} />
                      </>
                    )
                    : <MoreVert />}
                </Button>
              </HintTooltip>
            </Box>
          )}
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
        <Tooltip title={ready ? props.sendDescription : deliveryReason}>
          <span>
            <Button
              ref={props.sendButtonRef}
              variant="contained"
              size="small"
              disableElevation
              data-composer-action="send"
              aria-label={props.sendLabel === "Queue"
                ? "queue message"
                : "send"}
              aria-haspopup={props.sendLabel === "Send…" ? "menu" : undefined}
              disabled={!ready}
              aria-busy={props.pending || undefined}
              onPointerDown={(event) => {
                if (event.button === 0) event.preventDefault();
              }}
              onClick={props.onSubmit}
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
                : <Send sx={{ fontSize: "1rem" }} />}
              {props.sendLabel}
              <DesktopShortcut
                shortcut="Mod+Enter"
                quiet
                compact
                availability={shortcutAvailability(scoped && ready)}
              />
            </Button>
          </span>
        </Tooltip>
      </Box>
      {session.dialogs}
      <Menu
        anchorEl={sessionAnchor}
        open={sessionAnchor !== null}
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
      </Menu>
      <Menu
        id="desktop-composer-formatting"
        anchorEl={moreAnchor}
        open={moreAnchor !== null}
        onClose={() => setMoreAnchor(null)}
        anchorOrigin={{ vertical: "top", horizontal: "left" }}
        transformOrigin={{ vertical: "bottom", horizontal: "left" }}
      >
        {FORMAT_COMMANDS.map((command) => (
          <MenuItem
            key={command.id}
            disabled={composing}
            onClick={() => {
              setMoreAnchor(null);
              format(command.id);
            }}
          >
            <ListItemIcon>{command.icon}</ListItemIcon>
            <ListItemText>{command.label}</ListItemText>
            {DESKTOP_COMPOSER_FORMAT_CHORDS[command.id] && (
              <Box sx={{ ml: 2 }} data-shortcut-reference>
                <DesktopShortcut
                  shortcut={DESKTOP_COMPOSER_FORMAT_CHORDS[command.id]!}
                  quiet
                  compact
                  availability="inactive"
                />
              </Box>
            )}
          </MenuItem>
        ))}
      </Menu>
    </Box>
  );
}

function contextPercent(
  { used, size }: { used: number; size: number },
): number | null {
  if (!(size > 0)) return null;
  return used > 0 ? Math.max(1, Math.round(Math.min(100, used / size * 100))) : 0;
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
