import { type ReactNode, type RefObject } from "react";
import {
  Box,
  Button,
  Divider,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
} from "@mui/material";
import {
  AlternateEmail,
  AttachFile,
  MoreHoriz,
  RawOn,
} from "@mui/icons-material";
import {
  COMPOSER_COMMANDS,
  COMPOSER_COMMANDS_BY_ID,
} from "../composerCommands";
import { EditorPluginToolbar } from "../editorPlugins/EditorPluginToolbar";
import { HintTooltip } from "../HintTooltip";
import { DesktopShortcut, LeaderKeycap } from "./commands/DesktopKeycap";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_WORKSPACE_KEYS,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";
import { desktopToolbarActionSx } from "./desktopToolbarAction";

export const DESKTOP_QUICK_FORMATS = [
  "bold",
  "italic",
  "code",
  "link",
  "bulletList",
] as const;

export const DESKTOP_FORMAT_COMMANDS = COMPOSER_COMMANDS.filter((command) =>
  !["slash", "mention", "attach", "sourceMode"].includes(command.id)
);

export interface DesktopToolbarButtonSpec {
  id: string;
  icon: ReactNode;
  label: string;
  title?: string;
  key?: string;
  shortcut?: string;
  disabled?: boolean;
  reason?: string;
  onClick: () => void;
  formatTier?: "primary" | "secondary" | "extra";
  pressed?: boolean;
  anywhere?: boolean;
  tone?: "error";
  held?: boolean;
}

export function DesktopToolbarActionButton({
  spec,
  scoped,
  sourceScope,
  prefixAvailable,
  overlayOwnsKeys,
}: {
  spec: DesktopToolbarButtonSpec;
  scoped: boolean;
  sourceScope: boolean;
  prefixAvailable: boolean;
  overlayOwnsKeys: boolean;
}): React.JSX.Element {
  const {
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
    held = false,
  } = spec;
  return (
    <HintTooltip
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
          }}
        >
          {icon}
          <Box component="span" data-composer-action-label>{label}</Box>
          {key
            ? (
              <LeaderKeycap
                leaderKey={key}
                scopeAvailable={(id === "source"
                  ? sourceScope
                  : anywhere
                  ? prefixAvailable
                  : scoped) && !disabled}
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
}

function DesktopEditorInsertCluster({
  canInsert,
  canAttach,
  writable,
  composing,
  onSlash,
  onReference,
  onAttach,
  renderAction,
}: {
  canInsert: boolean;
  canAttach: boolean;
  writable: boolean;
  composing: boolean;
  onSlash?: (() => void) | undefined;
  onReference?: (() => void) | undefined;
  onAttach: () => void;
  renderAction: (spec: DesktopToolbarButtonSpec) => ReactNode;
}): React.JSX.Element {
  return (
    <Box data-composer-insert sx={{ display: "flex", gap: 0.25 }}>
      {onSlash && renderAction({
        id: "slash",
        icon: (
          <Box
            component="span"
            sx={{ fontSize: "1.125rem", width: "1.125rem", fontWeight: 700 }}
          >
            /
          </Box>
        ),
        label: "Commands",
        title: "Slash command / skill",
        key: DESKTOP_WORKSPACE_KEYS.composerSlash,
        disabled: !canInsert || composing,
        reason: composing
          ? "Finish text composition first"
          : "Resume this session to use completions",
        onClick: onSlash,
      })}
      {onReference && renderAction({
        id: "reference",
        icon: <AlternateEmail />,
        label: "Files",
        title: "Reference a file",
        key: DESKTOP_WORKSPACE_KEYS.composerReference,
        disabled: !canInsert || composing,
        reason: composing
          ? "Finish text composition first"
          : "Resume this session to use completions",
        onClick: onReference,
      })}
      {renderAction({
        id: "attach",
        icon: <AttachFile />,
        label: "Attach",
        title: "Attach image or file",
        key: DESKTOP_WORKSPACE_KEYS.composerAttach,
        disabled: !canAttach || !writable,
        reason: !writable
          ? "This draft cannot be edited"
          : "Resume this session to attach files",
        onClick: onAttach,
      })}
    </Box>
  );
}

function DesktopEditorMoreMenu({
  kind,
  menuId,
  anchorEl,
  composing,
  writable,
  onClose,
  onFormat,
  extraMenuItems,
}: {
  kind: "session" | "document";
  menuId: string;
  anchorEl: HTMLElement | null;
  composing: boolean;
  writable: boolean;
  onClose: () => void;
  onFormat: (id: string) => void;
  extraMenuItems?: ReactNode;
}): React.JSX.Element | null {
  if (anchorEl === null) return null;
  return (
    <Menu
      id={menuId}
      anchorEl={anchorEl}
      open
      slotProps={kind === "document" ? { transition: { timeout: 0 } } : undefined}
      onClose={onClose}
      anchorOrigin={{ vertical: "top", horizontal: "left" }}
      transformOrigin={{ vertical: "bottom", horizontal: "left" }}
    >
      {extraMenuItems}
      {DESKTOP_FORMAT_COMMANDS.map((command) => (
        <MenuItem
          key={command.id}
          disabled={composing || !writable}
          onClick={() => {
            onClose();
            onFormat(command.id);
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
  );
}

export interface DesktopEditorFormatRowProps {
  kind: "session" | "document";
  ariaLabel: string;
  menuId: string;
  formatIds?: readonly string[] | undefined;
  canInsert: boolean;
  canAttach: boolean;
  writable?: boolean;
  composing: boolean;
  sourceMode: boolean;
  scoped: boolean;
  sourceScope: boolean;
  prefixAvailable: boolean;
  overlayOwnsKeys: boolean;
  moreRef: RefObject<HTMLButtonElement | null>;
  moreAnchor: HTMLElement | null;
  onMoreOpen: (el: HTMLElement) => void;
  onMoreClose: () => void;
  onSlash?: (() => void) | undefined;
  onReference?: (() => void) | undefined;
  onAttach: () => void;
  onFormat: (id: string) => void;
  onToggleSourceMode: () => void;
  extraMenuItems?: ReactNode;
  trailing?: ReactNode;
}

/** Shared Desktop editing row used by New Prompt, Session Drafts, Queue item
 *  editing, and independent Draft documents so every writing surface exposes
 *  the same formatting controls, plugins, attachments, and Source toggle. */
export function DesktopEditorFormatRow(
  props: DesktopEditorFormatRowProps,
): React.JSX.Element {
  const {
    kind,
    ariaLabel,
    menuId,
    formatIds = DESKTOP_QUICK_FORMATS,
    canInsert,
    canAttach,
    writable = true,
    composing,
    sourceMode,
    scoped,
    sourceScope,
    prefixAvailable,
    overlayOwnsKeys,
    moreRef,
    moreAnchor,
    onMoreOpen,
    onMoreClose,
    onSlash,
    onReference,
    onAttach,
    onFormat,
    onToggleSourceMode,
    extraMenuItems,
    trailing,
  } = props;
  const renderAction = (spec: DesktopToolbarButtonSpec): ReactNode => (
    <DesktopToolbarActionButton
      key={spec.id}
      spec={spec}
      scoped={scoped}
      sourceScope={sourceScope}
      prefixAvailable={prefixAvailable}
      overlayOwnsKeys={overlayOwnsKeys}
    />
  );

  return (
    <>
      <Box
        role="group"
        aria-label={ariaLabel}
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
        <DesktopEditorInsertCluster
          canInsert={canInsert}
          canAttach={canAttach}
          writable={writable}
          composing={composing}
          onSlash={onSlash}
          onReference={onReference}
          onAttach={onAttach}
          renderAction={renderAction}
        />
        <Divider orientation="vertical" flexItem sx={{ my: 0.75, mx: 0.5 }} />
        {formatIds.map((id) => {
          const command = COMPOSER_COMMANDS_BY_ID[id];
          if (!command) return null;
          const chord = DESKTOP_COMPOSER_FORMAT_CHORDS[id];
          const tier = ["bold", "italic", "code"].includes(id)
            ? "primary"
            : ["undo", "redo", "link", "bulletList"].includes(id)
            ? "secondary"
            : "extra";
          return renderAction({
            id,
            icon: command.icon,
            label: command.label,
            ...(chord ? { shortcut: chord } : {}),
            disabled: composing || !writable,
            reason: composing
              ? "Finish text composition first"
              : "This draft cannot be edited",
            formatTier: tier,
            onClick: () => onFormat(id),
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
            aria-controls={moreAnchor ? menuId : undefined}
            aria-expanded={moreAnchor ? "true" : undefined}
            data-composer-action="more"
            size="small"
            color="inherit"
            onPointerDown={(event) => {
              if (event.button === 0) event.preventDefault();
            }}
            onClick={(event) => onMoreOpen(event.currentTarget)}
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
        <EditorPluginToolbar kind={kind} disabled={composing || !writable} />
        <Box sx={{ flex: 1 }} />
        {trailing}
        {renderAction({
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
          onClick: onToggleSourceMode,
        })}
      </Box>
      <DesktopEditorMoreMenu
        kind={kind}
        menuId={menuId}
        anchorEl={moreAnchor}
        composing={composing}
        writable={writable}
        onClose={onMoreClose}
        onFormat={onFormat}
        extraMenuItems={extraMenuItems}
      />
    </>
  );
}
