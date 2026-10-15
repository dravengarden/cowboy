import { ExtensionOutlined, SettingsOutlined } from "@mui/icons-material";
import { Stack } from "@mui/material";
import { WorkspaceExtensionsButton } from "../extensions/WorkspaceExtensionsButton";
import { DESKTOP_SHORTCUTS } from "./commands/workspaceShortcuts";
import { DesktopSidebarAction, sidebarSlotFitSx } from "./DesktopSidebarAction";

/**
 * The window-wide entries, VS Code style: Extensions (the installed Plugins'
 * repository resources) and Settings live with Sessions, not with the open
 * session's top bar. The collapsed rail stacks them at its foot like VS
 * Code's activity bar; the expanded sidebar's bar draws them through
 * `DesktopExtensionsActivity` and `DesktopSettingsAction`. Only one placement
 * is mounted at a time, so `␣X` and `␣,` have one owner.
 */
export function DesktopRailGlobalActions({
  sessionId,
  machineId,
  onOpenSettings,
}: {
  sessionId?: string | undefined;
  machineId?: string | undefined;
  onOpenSettings: () => void;
}): React.JSX.Element {
  return (
    <Stack
      data-desktop-global-actions="rail"
      alignItems="center"
      spacing={0.5}
      // The 56 px rail fits a glyph and its slot up to about 3.25rem.
      sx={{ alignSelf: "stretch", ...sidebarSlotFitSx(3.25) }}
    >
      <WorkspaceExtensionsButton
        context={sessionId}
        machineId={machineId}
        renderTrigger={({ disabled, open }) => (
          <DesktopSidebarAction
            data-desktop-global-action="extensions"
            label="Extensions"
            shortcut={DESKTOP_SHORTCUTS.extensions}
            detail={disabled ? "Open a session to browse its repository resources" : undefined}
            disabled={disabled}
            onClick={open}
            placement="right"
          >
            <ExtensionOutlined />
          </DesktopSidebarAction>
        )}
      />
      <DesktopSidebarAction
        data-desktop-global-action="settings"
        label="Settings"
        shortcut={DESKTOP_SHORTCUTS.settings}
        onClick={onOpenSettings}
        placement="right"
      >
        <SettingsOutlined />
      </DesktopSidebarAction>
    </Stack>
  );
}

/** Extensions as an activity beside Sessions in the expanded bar. */
export function DesktopExtensionsActivity({
  sessionId,
  machineId,
}: {
  sessionId?: string | undefined;
  machineId?: string | undefined;
}): React.JSX.Element {
  return (
    <WorkspaceExtensionsButton
      context={sessionId}
      machineId={machineId}
      renderTrigger={({ disabled, open }) => (
        <DesktopSidebarAction
          data-desktop-global-action="extensions"
          label="Extensions"
          shortcut={DESKTOP_SHORTCUTS.extensions}
          detail={disabled ? "Open a session to browse its repository resources" : undefined}
          disabled={disabled}
          onClick={open}
        >
          <ExtensionOutlined />
        </DesktopSidebarAction>
      )}
    />
  );
}

/** Settings, VS Code's Manage gear, at the bar's right end. */
export function DesktopSettingsAction(
  { onOpenSettings }: { onOpenSettings: () => void },
): React.JSX.Element {
  return (
    <DesktopSidebarAction
      data-desktop-global-action="settings"
      label="Settings"
      shortcut={DESKTOP_SHORTCUTS.settings}
      onClick={onOpenSettings}
    >
      <SettingsOutlined />
    </DesktopSidebarAction>
  );
}
