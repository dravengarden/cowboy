import {
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import { EditorPluginToolbar } from "../editorPlugins/EditorPluginToolbar";
import {
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
  Bolt,
  EditNoteOutlined,
  MoreHoriz,
  RawOn,
  Schedule,
  Send,
  VerticalAlignTop,
} from "@mui/icons-material";
import type { ComposerEditorHandle } from "../ComposerEditor";
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
import { desktopOverlayOwnsShortcuts } from "./commands/desktopShortcutScope";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_COMPOSER_FORMAT_KEYS,
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";
import { useImeStatus } from "./vim/imeStatusStore";

const QUICK_FORMATS = ["bold", "italic", "code", "link", "bulletList"];
const FORMAT_COMMANDS = COMPOSER_COMMANDS.filter((command) =>
  !["slash", "mention", "attach", "sourceMode"].includes(command.id)
);

export interface DesktopComposerToolbarProps {
  editorRef: RefObject<ComposerEditorHandle | null>;
  sendButtonRef: RefObject<HTMLButtonElement | null>;
  canInsert: boolean;
  sendable: boolean;
  canJumpFront: boolean;
  canForce: boolean;
  pending: boolean;
  progress: boolean;
  sendLabel: "Send" | "Queue";
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
  const workspace = useDesktopWorkspace();
  const sourceMode = useComposerSourceMode();
  const composing = useImeStatus().phase === "composing";
  const moreRef = useRef<HTMLButtonElement>(null);
  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null);
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
  }): ReactNode => (
    <Tooltip
      key={id}
      title={
        <Box sx={{ display: "grid", gap: 0.5 }}>
          <span>{disabled && reason ? `${title} · ${reason}` : title}</span>
          {(key || shortcut) && (
            <DesktopShortcut
              shortcut={key ? desktopWorkspaceSequence(key) : shortcut!}
              compact
              availability={shortcutAvailability(
                (id === "source" ? sourceScope : scoped) && !disabled,
              )}
            />
          )}
        </Box>
      }
    >
      <Box
        component="span"
        data-format-tier={formatTier}
        sx={{ display: "inline-flex", flexShrink: 0 }}
      >
        <Button
          data-composer-action={id}
          aria-label={title}
          aria-pressed={pressed}
          disabled={disabled}
          onPointerDown={(event) => {
            if (event.button === 0) event.preventDefault();
          }}
          onClick={onClick}
          color={id === "force" ? "warning" : pressed ? "primary" : "inherit"}
          size="small"
          sx={{
            minWidth: 0,
            minHeight: "2rem",
            px: 0.75,
            gap: 0.5,
            borderRadius: 1,
            textTransform: "none",
            whiteSpace: "nowrap",
            fontSize: "0.75rem",
            fontWeight: 500,
            bgcolor: pressed ? "action.selected" : "transparent",
            "& .MuiSvgIcon-root": { fontSize: "1.125rem" },
          }}
        >
          {icon}
          <Box component="span" data-composer-action-label>{label}</Box>
          {key
            ? (
              <LeaderKeycap
                leaderKey={key}
                scopeAvailable={(id === "source" ? sourceScope : scoped) &&
                  !disabled}
              />
            )
            : shortcut && (
              <DesktopShortcut
                shortcut={shortcut}
                compact
                quiet
                availability={shortcutAvailability(scoped && !disabled)}
              />
            )}
        </Button>
      </Box>
    </Tooltip>
  );
  return (
    <Box
      data-desktop-composer-toolbar
      sx={{
        containerType: "inline-size",
        containerName: "composer-tools",
        flexShrink: 0,
        minWidth: 0,
        borderTop: 1,
        borderColor: "divider",
        color: "text.secondary",
        "& [data-composer-action-label]": { display: "none" },
        "& [data-format-tier]": { display: "none" },
        "@container composer-tools (min-width: 32rem)": {
          "& [data-format-tier='primary']": { display: "inline-flex" },
        },
        "@container composer-tools (min-width: 42rem)": {
          "& [data-format-tier='secondary']": { display: "inline-flex" },
          "& [data-composer-delivery] [data-composer-action-label]": {
            display: "inline",
          },
        },
        "@container composer-tools (min-width: 56rem)": {
          "& [data-composer-insert] [data-composer-action-label], & [data-composer-action='source'] [data-composer-action-label]":
            { display: "inline" },
        },
      }}
    >
      <DesktopComposerCommandBindings
        sendable={ready}
        canInsert={props.canInsert}
        canAttach={props.canInsert}
        canJumpFront={props.canJumpFront}
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
            key: DESKTOP_COMPOSER_FORMAT_KEYS[id]!,
            disabled: composing,
            reason: "Finish text composition first",
            formatTier: index < 3 ? "primary" : "secondary",
            onClick: () => format(id),
          });
        })}
        <Tooltip
          title={`More formatting · ${
            desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.composerMore)
          }`}
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
            sx={{
              minWidth: 0,
              minHeight: "2rem",
              px: 0.75,
              gap: 0.5,
              borderRadius: 1,
            }}
          >
            <MoreHoriz sx={{ fontSize: "1.125rem" }} />
            <LeaderKeycap
              leaderKey={DESKTOP_WORKSPACE_KEYS.composerMore}
              scopeAvailable={scoped}
              {...(moreAnchor ? { availability: "active" as const } : {})}
            />
          </Button>
        </Tooltip>
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
          {action({
            id: "next",
            icon: <VerticalAlignTop />,
            label: "Run next",
            title: "Jump to front of queue",
            key: DESKTOP_WORKSPACE_KEYS.composerJumpFront,
            disabled: !ready || !props.canJumpFront,
            reason: !ready
              ? deliveryReason
              : "There are no queued messages to move ahead of",
            onClick: props.onJumpFront,
          })}
          {action({
            id: "force",
            icon: <Bolt />,
            label: "Force push…",
            title: "Force push",
            shortcut: "Alt+Enter",
            disabled: !ready || !props.canForce,
            reason: !ready
              ? deliveryReason
              : "Available during an active turn or a paused queue",
            onClick: props.onForce,
          })}
        </Box>
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
            {DESKTOP_COMPOSER_FORMAT_KEYS[command.id] && (
              <Box sx={{ ml: 2 }} data-shortcut-reference>
                <DesktopShortcut
                  shortcut={desktopWorkspaceSequence(
                    DESKTOP_COMPOSER_FORMAT_KEYS[command.id]!,
                  )}
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
