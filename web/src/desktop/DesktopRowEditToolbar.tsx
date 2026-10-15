import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { Box, Button } from "@mui/material";
import { AlternateEmail, AttachFile, Check, OpenInFull } from "@mui/icons-material";
import { HintTooltip } from "../HintTooltip";
import { DesktopShortcut, LeaderKeycap } from "./commands/DesktopKeycap";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_WORKSPACE_KEYS,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";
import { useOptionalDesktopWorkspace } from "./DesktopWorkspaceController";
import { desktopToolbarActionSx } from "./desktopToolbarAction";

/** Desktop Queue/Draft row editor commands. Same compact action family as the
 * prompt toolbar: icon plus an inline keycap, root-relative geometry, and the
 * commit action as the one contained button. The earlier touch bar stacked
 * keycaps under 40px icon buttons whose frame ignored the global font size. */
export function DesktopRowEditToolbar({
  dead,
  sendable,
  submitLabel,
  submitShortcut,
  submitIcon,
  onTrigger,
  onAttach,
  onSubmit,
  onExpand,
}: {
  dead: boolean;
  sendable: boolean;
  submitLabel: string;
  submitShortcut: "send" | "save";
  submitIcon?: ReactNode;
  onTrigger: (trigger: string) => void;
  onAttach?: (() => void) | undefined;
  onSubmit: () => void;
  onExpand?: (() => void) | undefined;
}): React.JSX.Element {
  const ownerRef = useRef<HTMLDivElement>(null);
  const [ownerRegion, setOwnerRegion] = useState<string | null>(null);
  useLayoutEffect(() => {
    setOwnerRegion(
      ownerRef.current?.closest<HTMLElement>("[data-desktop-region]")?.dataset
        .desktopRegion ?? null,
    );
  }, []);
  const workspace = useOptionalDesktopWorkspace();
  const scoped = ownerRegion !== null &&
    workspace?.focusedRegion === ownerRegion;
  const action = ({
    id,
    icon,
    title,
    ariaLabel,
    leaderKey,
    disabled,
    onClick,
  }: {
    id: string;
    icon: ReactNode;
    title: string;
    ariaLabel: string;
    leaderKey: string;
    disabled: boolean;
    onClick: () => void;
  }): ReactNode => (
    <HintTooltip
      key={id}
      title={title}
      shortcut={desktopWorkspaceSequence(leaderKey)}
    >
      <span>
        <Button
          data-row-edit-action={id}
          aria-label={ariaLabel}
          color="inherit"
          size="small"
          disabled={disabled}
          onPointerDown={(event) => {
            if (event.button === 0) event.preventDefault();
          }}
          onClick={onClick}
          sx={desktopToolbarActionSx}
        >
          {icon}
          <LeaderKeycap leaderKey={leaderKey} scopeAvailable={scoped && !disabled} />
        </Button>
      </span>
    </HintTooltip>
  );
  const commitShortcut = submitShortcut === "save" ? "Mod+S" : "Mod+Enter";
  return (
    <Box
      ref={ownerRef}
      data-desktop-row-edit-toolbar
      role="group"
      aria-label="Edit commands"
      sx={{
        display: "flex",
        alignItems: "center",
        flexWrap: "wrap",
        gap: "0.125rem",
        color: "text.secondary",
      }}
    >
      {action({
        id: "slash",
        icon: (
          <Box
            component="span"
            sx={{ fontSize: "1.125rem", width: "1.125rem", fontWeight: 700, lineHeight: 1 }}
          >
            /
          </Box>
        ),
        title: "Slash command / skill",
        ariaLabel: "slash command",
        leaderKey: DESKTOP_WORKSPACE_KEYS.composerSlash,
        disabled: dead,
        onClick: () => onTrigger("/"),
      })}
      {action({
        id: "reference",
        icon: <AlternateEmail />,
        title: "Reference a file (@)",
        ariaLabel: "reference a file",
        leaderKey: DESKTOP_WORKSPACE_KEYS.composerReference,
        disabled: dead,
        onClick: () => onTrigger("@"),
      })}
      {onAttach && action({
        id: "attach",
        icon: <AttachFile />,
        title: "Attach image or file",
        ariaLabel: "attach image or file",
        leaderKey: DESKTOP_WORKSPACE_KEYS.composerAttach,
        disabled: dead,
        onClick: onAttach,
      })}
      {onExpand && action({
        id: "expand",
        icon: <OpenInFull />,
        title: "Expand editor",
        ariaLabel: "expand editor",
        leaderKey: DESKTOP_WORKSPACE_KEYS.editorExpand,
        disabled: false,
        onClick: onExpand,
      })}
      <Box sx={{ flex: 1 }} />
      <HintTooltip title={submitLabel} shortcut={commitShortcut}>
        <span>
          <Button
            data-row-edit-action="commit"
            aria-label={submitLabel.toLowerCase()}
            variant="contained"
            size="small"
            disableElevation
            disabled={!sendable}
            onPointerDown={(event) => {
              if (event.button === 0) event.preventDefault();
            }}
            onClick={onSubmit}
            sx={{
              ...desktopToolbarActionSx,
              px: "0.5rem",
              gap: "0.375rem",
              fontWeight: 650,
              "& kbd": {
                color: "inherit",
                borderColor: "currentColor",
                bgcolor: "transparent",
                opacity: 0.65,
              },
            }}
          >
            {submitIcon ?? <Check />}
            {submitShortcut === "save" ? "Save" : "Send"}
            <DesktopShortcut
              shortcut={commitShortcut}
              quiet
              compact
              availability={shortcutAvailability(scoped && sendable)}
            />
          </Button>
        </span>
      </HintTooltip>
    </Box>
  );
}
