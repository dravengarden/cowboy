import { desktopSize } from "../surface/desktopSize";
import { alpha, Box, ButtonBase, Divider, Stack, Tooltip } from "@mui/material";
import { useExploreSessionState } from "../explore/exploreStore";
import type { Status } from "../protocol";
import { memo, useEffect, useState, useSyncExternalStore } from "react";
import {
  productSessionAlertVisible,
  subscribeProductSessionAlertVisible,
} from "../auth/productSessionAlertHost";
import { retrySyncNow, useHeldDeliveries, useStoreSelector, useSyncStatus } from "../store";
import {
  attentionCount,
  syncStatusDetail,
  syncStatusLabel,
  syncStatusTone,
  withHeld,
} from "../syncStatus";
import { useVimMode, VIM_MODE_COLOR } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useComposerSourceMode } from "../composerSourceMode";
  // The single status-line / toolbar prompt-buffer status updates live.
import { type DesktopPromptStatus, useDesktopPromptStatus } from "./promptBuffer";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopKeycap, DesktopShortcut } from "./commands/DesktopKeycap";
import {
  type DesktopCommand,
  useDesktopCommandActions,
  useDesktopCommandSelector,
} from "./commands/DesktopCommandProvider";
import { topBarRegionHints } from "./topBarRegionHints";
import type { ShortcutAvailability } from "./commands/shortcutAvailability";
import { useImeStatus } from "./vim/imeStatusStore";
import { useVimMacroRecording } from "./vim/macroStatusStore";
import { useDesktopReviewShown } from "./review/desktopWorkView";
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
  DESKTOP_DOCUMENT_KEYS,
  DESKTOP_RESIZE_HINT,
  DESKTOP_APPROVAL_SHORTCUT,
  DESKTOP_SHORTCUTS,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";

function Segment({
  label,
  tooltip,
  color = "text.secondary",
  mono = false,
  onClick,
}: {
  label: React.ReactNode;
  tooltip?: string;
  color?: string;
  mono?: boolean;
  onClick?: () => void;
}): React.JSX.Element {
  const sx = {
    height: desktopSize(28),
    px: 1,
    color,
    fontSize: "0.6875rem",
    fontWeight: 650,
    letterSpacing: "0.035em",
    whiteSpace: "nowrap",
    fontFamily: mono && typeof label === "string" ? "monospace" : "inherit",
  } as const;
  // A read-out is not a control: only a segment with an action is a button,
  // and each of those mirrors a keyboard command (FOCUS.md "Shortcut slots").
  const body = onClick
    ? (
      <ButtonBase
        disableRipple
        tabIndex={-1}
        onClick={onClick}
        sx={{
          ...sx,
          "&:hover": { bgcolor: "action.hover", color: "text.primary" },
        }}
      >
        {label}
      </ButtonBase>
    )
    : (
      <Box component="span" sx={{ ...sx, display: "inline-flex", alignItems: "center" }}>
        {label}
      </Box>
    );
  return tooltip ? <Tooltip title={tooltip}>{body}</Tooltip> : body;
}

function StatusLineHints({
  hints,
  allHighPriority,
}: {
  hints: readonly RegionHint[];
  allHighPriority: boolean;
}): React.JSX.Element | null {
  if (hints.length === 0) return null;
  return (
    <Stack
      direction="row"
      spacing={1.25}
      alignItems="center"
      aria-label="Focused region shortcuts"
      sx={{
        ml: 0.75,
        mr: 1,
        minWidth: 0,
        // Hints yield space; the connection segment on the right must not.
        overflow: "hidden",
        color: "text.disabled",
        "@media (max-width: 640px)": { display: "none" },
      }}
    >
      {hints.map((hint) => {
        const highPriority = allHighPriority || hint.priority === "high";
        return (
          <Stack
            key={`${hint.keys}-${hint.label}`}
            direction="row"
            spacing={0.5}
            alignItems="center"
            data-status-hint-priority={highPriority ? "high" : "normal"}
            sx={highPriority
              ? { flexShrink: 0 }
              : {
                flexShrink: 0,
                "@media (max-width: 1180px)": { display: "none" },
              }}
          >
            <DesktopShortcut
              shortcut={hint.keys}
              quiet
              availability={hint.availability ?? "available"}
            />
            <Box component="span" sx={{ fontSize: "0.625rem", whiteSpace: "nowrap" }}>
              {hint.label}
            </Box>
          </Stack>
        );
      })}
    </Stack>
  );
}

interface RegionHint {
  keys: string;
  label: string;
  availability?: ShortcutAvailability;
  priority?: "high";
}

// Desktop keeps the segment informational; only Mobile offers "Hide reminder".
const NO_ACKNOWLEDGED: ReadonlySet<string> = new Set();

const HISTORY_HINTS: RegionHint[] = [
  { keys: "J/K", label: "Scroll" },
  { keys: "Ctrl+D/U", label: "Half page" },
  { keys: "Ctrl+F/B", label: "Page" },
  { keys: "GG/G", label: "Oldest/latest" },
  { keys: "V", label: "Explore" },
  { keys: "F", label: "Follow" },
  { keys: "Tab", label: "Widget" },
  { keys: "H/L", label: "Close/open" },
];

const EXPLORE_HINTS: RegionHint[] = [
  { keys: "J/K", label: "Page" },
  { keys: "Ctrl+D/U", label: "Half page" },
  { keys: "Ctrl+F/B", label: "Scroll page" },
  { keys: "GG/G", label: "Oldest/latest" },
  { keys: "P", label: "Pages" },
  { keys: "N", label: "New page" },
  { keys: "V", label: "History" },
  { keys: "F", label: "Follow" },
  { keys: "Tab", label: "Widget" },
  { keys: "H/L", label: "Close/open" },
];

const REVIEW_NAVIGATOR_HINTS: RegionHint[] = [
  { keys: "J/K", label: "Row" },
  { keys: "L/Enter", label: "Open" },
  { keys: "H", label: "Collapse/up" },
  { keys: "[/]", label: "Tab" },
  { keys: "/", label: "Filter" },
  { keys: "'", label: "Labels" },
  { keys: DESKTOP_SHORTCUTS.reviewMark, label: "Mark" },
  { keys: DESKTOP_SHORTCUTS.reviewContext, label: "Context" },
  { keys: DESKTOP_SHORTCUTS.focusConversation, label: "Conversation" },
];

const REVIEW_DOCUMENT_HINTS: RegionHint[] = [
  { keys: "]c/[c", label: "Hunk" },
  { keys: "]f/[f", label: "File" },
  { keys: "]u", label: "Unreviewed" },
  { keys: "]d", label: "Problem" },
  { keys: "gO", label: "Outline" },
  { keys: "K", label: "Hover" },
  { keys: "gd/gr", label: "Definition/refs" },
  { keys: "gt/gT", label: "Tab" },
  { keys: "V", label: "Select" },
  { keys: "Enter", label: "Send" },
  { keys: DESKTOP_SHORTCUTS.reviewMark, label: "Mark" },
  { keys: DESKTOP_SHORTCUTS.reviewChanges, label: "Changes" },
  { keys: DESKTOP_SHORTCUTS.focusConversation, label: "Conversation" },
];

function regionHints(
  region: string | null,
  status: Status | undefined,
  projection: "history" | "explore",
  sourceMode: boolean,
  live: boolean,
  vim: boolean,
  topBar: readonly RegionHint[],
  promptStatus: DesktopPromptStatus,
  documentMode: boolean,
): RegionHint[] {
  switch (region) {
    case "topbar.controls":
      // The action keys come from the registry (topBarRegionHints), so a
      // conditional action (Verify, Update) is listed exactly while it runs.
      return [
        { keys: "H/L", label: "Select" },
        { keys: "Enter", label: "Open" },
        ...topBar,
      ];
    case "sessions.rail":
      return [
        { keys: "J/K", label: "Folder" },
        { keys: "'", label: "Labels" },
        { keys: "L/Enter", label: "Open" },
        { keys: DESKTOP_SHORTCUTS.toggleSessions, label: "Expand list" },
      ];
    case "sessions.list":
      return [
        { keys: "J/K", label: "Row" },
        { keys: "GG/G", label: "First/last" },
        { keys: "'", label: "Labels" },
        { keys: "L/Enter", label: "Open" },
        { keys: "H", label: "Collapse/up" },
        { keys: "Ctrl+D/U", label: "Page" },
        { keys: "zz/zt/zb", label: "View" },
        { keys: "za/zM/zR", label: "Fold" },
        { keys: "A", label: "New session here" },
        { keys: "S", label: "Actions" },
        { keys: ",", label: "Settings" },
        { keys: "R", label: "Rename" },
        { keys: "M", label: "Move to" },
        { keys: "X", label: "Trash" },
        { keys: "N", label: "New folder" },
        { keys: "O", label: "Order mode" },
      ];
    case "prompt.calls":
      return [
        { keys: "J/K", label: "Call" },
        { keys: "Enter", label: "Details" },
      ];
    case "prompt.plan":
      return [
        { keys: "J/K", label: "Step" },
        { keys: "GG/G", label: "First/last" },
        { keys: "Enter", label: "Toggle" },
      ];
    case "prompt.queued":
      return [
        { keys: "J/K", label: "Message" },
        { keys: "I", label: "Edit" },
        { keys: "L/Enter", label: "Normal" },
        { keys: "Mod+Enter", label: "Deliver" },
        { keys: "R", label: "To drafts" },
        { keys: "X", label: "Remove" },
        { keys: "O", label: "Order" },
        { keys: "Esc/H", label: "Canvas" },
      ];
    case "prompt.draft":
      return [
        { keys: "J/K", label: "Message" },
        { keys: "I", label: "Edit" },
        { keys: "L/Enter", label: "Normal" },
        { keys: "N", label: "New draft" },
        { keys: "Mod+Enter", label: "Send" },
        { keys: "T", label: "Schedule" },
        { keys: "X", label: "Remove" },
        { keys: "O", label: "Order" },
        { keys: "Esc/H", label: "Canvas" },
      ];
    case "conversation.transcript":
      return projection === "explore" ? EXPLORE_HINTS : HISTORY_HINTS;
    case "review.navigator":
      return REVIEW_NAVIGATOR_HINTS;
    case "review.document":
      return REVIEW_DOCUMENT_HINTS;
    case "prompt.composer": {
      if (documentMode) {
        return [
          ...(vim ? [{ keys: "Esc", label: "Normal" }] : []),
          { keys: "Mod+Enter", label: "Use in Session…" },
          { keys: "Mod+S", label: "Save" },
          { keys: desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.title), label: "Title" },
          { keys: desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.copy), label: "Copy" },
          {
            keys: DESKTOP_SHORTCUTS.toggleSourceMode,
            label: sourceMode ? "Live preview" : "Source",
          },
        ];
      }
      if (promptStatus.kind === "queued") {
        return [
          { keys: "Esc", label: promptStatus.dirty && vim ? "Normal" : "Close" },
          { keys: "Mod+Enter", label: "Finish" },
          { keys: "Mod+S", label: "Save" },
          { keys: "Alt+Enter", label: "Deliver…" },
          ...(vim ? [{ keys: "[q/]q", label: "Queue" }, { keys: "ZQ", label: "Discard" }] : []),
        ];
      }
      if (promptStatus.kind === "scheduled") {
        return [
          { keys: "Esc", label: promptStatus.dirty && vim ? "Normal" : "Close" },
          { keys: "Mod+Enter", label: "Deliver…" },
          { keys: "Mod+S", label: "Save" },
          ...(vim ? [{ keys: "ZQ", label: "Discard" }] : []),
        ];
      }
      if (promptStatus.kind === "draft" || promptStatus.kind === "new-draft") {
        return [
          { keys: "Esc", label: "Save & back" },
          { keys: "Mod+Enter", label: promptStatus.sendLabel },
          { keys: "Mod+S", label: "Save" },
          ...(promptStatus.canForce ? [{ keys: "Alt+Enter", label: "Delivery" }] : []),
        ];
      }
      return [
        // Only Vim turns Esc into a mode change; without it the editor keeps
        // Esc for its own layers (completion, selection).
        ...(vim ? [{ keys: "Esc", label: "Normal" }] : []),
        {
          keys: "Mod+Enter",
          label: promptStatus.sendLabel || (status === "busy" || !live ? "Send…" : "Send"),
        },
        { keys: "Mod+S", label: "Draft" },
        // The label names the destination, not the current state: a hint
        // describes what the key does next.
        {
          keys: DESKTOP_SHORTCUTS.toggleSourceMode,
          label: sourceMode ? "Live preview" : "Source",
        },
      ];
    }
    default:
      return [];
  }
}

function statusPaneLabel(
  resizing: boolean,
  region: string | null,
  pane: string,
  reviewShown: boolean,
  documentMode: boolean,
): string {
  if (resizing) return "layout";
  if (region === "topbar.controls") return "topbar";
  if (documentMode && pane === "prompt") return "draft";
  // The work pane names the view it shows.
  return pane === "conversation" && reviewShown ? "review" : pane;
}

const EMPTY_TOP_BAR_HINTS: readonly RegionHint[] = [];
const EMPTY_TOP_BAR_COMMANDS: readonly DesktopCommand[] = [];

function sameCommandRefs(
  previous: readonly DesktopCommand[],
  next: readonly DesktopCommand[],
): boolean {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;
  for (let i = 0; i < previous.length; i += 1) {
    if (previous[i] !== next[i]) return false;
  }
  return true;
}

function useStatusLineSubscriptions(focusedRegion: string | null) {
  useSyncExternalStore(
    subscribeProductSessionAlertVisible,
    () => (focusedRegion === "topbar.controls" ? productSessionAlertVisible() : false),
    () => false,
  );
  const topBarCommands = useDesktopCommandSelector(
    (registered) =>
      focusedRegion === "topbar.controls"
        ? registered.filter((command) => command.regions?.includes("topbar.controls"))
        : EMPTY_TOP_BAR_COMMANDS,
    sameCommandRefs,
  );
  const topBarHints = focusedRegion === "topbar.controls"
    ? topBarRegionHints(topBarCommands)
    : EMPTY_TOP_BAR_HINTS;
  const hasApprovalChoice = useDesktopCommandSelector((registered) =>
    registered.some((command) => command.id.startsWith("approval.choice."))
  );
  const hasSessionItems = useStoreSelector((snapshot) =>
    focusedRegion === "sessions.list" || focusedRegion === "sessions.rail"
      ? snapshot.sessions.length > 0 || snapshot.sessionFolders.folders.length > 0
      : false
  );
  return { topBarHints, hasApprovalChoice, hasSessionItems };
}

function promptBufferBadge(
  promptStatus: DesktopPromptStatus,
): { label: string; color: string; tooltip: string } | null {
  const pos = promptStatus.index !== undefined ? ` #${String(promptStatus.index + 1)}` : "";
  if (promptStatus.kind === "queued") {
    return promptStatus.holdStatus === "unconfirmed"
      ? {
        label: `QUEUE${pos} · HOLD UNCONFIRMED`,
        color: "warning.main",
        tooltip: "Editing queued message offline — server drain hold will be re-confirmed on reconnect",
      }
      : {
        label: `QUEUE${pos}`,
        color: "warning.main",
        tooltip: "Editing queued message in Prompt canvas (auto-drain held)",
      };
  }
  if (promptStatus.kind === "scheduled") {
    return {
      label: promptStatus.scheduleLabel ? `SCHEDULED · ${promptStatus.scheduleLabel}` : "SCHEDULED",
      color: "info.main",
      tooltip: "Editing scheduled draft in Prompt canvas",
    };
  }
  if (promptStatus.kind === "new-draft") {
    return {
      label: "NEW DRAFT",
      color: "primary.main",
      tooltip: "Composing a new session draft in Prompt canvas",
    };
  }
  if (promptStatus.kind === "draft") {
    return {
      label: `DRAFT${pos}`,
      color: "primary.main",
      tooltip: "Editing session draft in Prompt canvas",
    };
  }
  return null;
}

export const DesktopStatusLine = memo(function DesktopStatusLine({
  sessionId,
  status,
  documentMode = false,
}: {
  sessionId?: string | undefined;
  status?: Status | undefined;
  documentMode?: boolean | undefined;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const commands = useDesktopCommandActions();
  const { focusedPane, focusedRegion, mode } = workspace;
  const { topBarHints, hasApprovalChoice, hasSessionItems } = useStatusLineSubscriptions(focusedRegion);
  const promptStatus = useDesktopPromptStatus();
  const bufferBadge = documentMode ? null : promptBufferBadge(promptStatus);
  const vimEnabled = useVimSetting();
  const sourceMode = useComposerSourceMode();
  const vimMode = useVimMode();
  const ime = useImeStatus();
  const macro = useVimMacroRecording();
  const reviewShown = useDesktopReviewShown();
  const projection = useExploreSessionState(sessionId ?? "").projection;
  const rawSync = useSyncStatus();
  const held = useHeldDeliveries();
  // Held rows of this pane's session already show their failure in place;
  // the segment points only at the ones the user is not looking at.
  const sync = withHeld(rawSync, attentionCount(held.sessions, sessionId ?? "", NO_ACKNOWLEDGED));
  const syncLive = sync.phase === "live" && sync.outbox.held === 0;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (syncLive) return undefined;
    const timer = globalThis.setInterval(() => setNow(Date.now()), 1000);
    return () => globalThis.clearInterval(timer);
  }, [syncLive]);
  const syncLabel = syncStatusLabel(sync, now) ?? "LIVE";
  const syncTone = syncStatusTone(sync.phase);
  const activeElement = document.activeElement instanceof Element
    ? document.activeElement
    : null;
  const promptEditorContext = focusedRegion === "prompt.composer" ||
    (focusedRegion?.startsWith("prompt.") === true &&
      Boolean(activeElement?.closest(".cm-editor, [data-vim-command-sink]")));
  const resizingLayout = workspace.selectedSplitter !== null;
  const workspaceCommandMode = mode === "command";
  const effectiveMode = resizingLayout
    ? "resize"
    : workspaceCommandMode
    ? "command"
    : promptEditorContext && vimEnabled ? vimMode : mode;
  const modeColor = resizingLayout
    ? "primary.main"
    : workspaceCommandMode
    ? "secondary.main"
    : promptEditorContext && vimEnabled
    ? (VIM_MODE_COLOR[vimMode] ?? "primary.main")
    : "primary.main";
  const imeLabel = ime.phase === "composing"
    ? (ime.autoInserted ? "IME → INSERT" : "IME · COMPOSING")
    : ime.phase === "committed"
    ? "IME · COMMITTED"
    : null;
  const needsRegionElement = focusedRegion === "prompt.queued" ||
    focusedRegion === "prompt.draft";
  const regionElement = needsRegionElement
    ? document.querySelector<HTMLElement>(
      `[data-desktop-region="${CSS.escape(focusedRegion)}"]`,
    )
    : null;
  const promptRegions = focusedPane === "prompt" &&
      focusedRegion?.startsWith("prompt.") === true &&
      focusedRegion !== "prompt.composer"
    ? [
      { keys: DESKTOP_FOCUS_PLAN_SHORTCUT, label: "Plan" },
      { keys: DESKTOP_SHORTCUTS.focusCalls, label: "Tasks" },
      { keys: DESKTOP_SHORTCUTS.focusQueue, label: "Queue" },
      { keys: DESKTOP_SHORTCUTS.focusDrafts, label: "Drafts" },
      { keys: DESKTOP_FOCUS_PROMPT_SHORTCUT, label: "Editor" },
    ]
    : [];
  const ordinaryHints: RegionHint[] = [
    ...promptRegions,
    ...regionHints(
      focusedRegion,
      status,
      projection,
      sourceMode,
      rawSync.phase === "live",
      vimEnabled,
      topBarHints,
      promptStatus,
      documentMode,
    ),
    // Window motion works from every region (FOCUS.md "Window motion").
    ...(focusedRegion ? [{ keys: "Ctrl+H/J/K/L", label: "Window" }] : []),
    ...((focusedRegion === "sessions.list" || focusedRegion === "sessions.rail") && hasSessionItems
      ? [{ keys: DESKTOP_SHORTCUTS.switchSession, label: "Switch" }]
      : []),
    ...(regionElement?.dataset.desktopReorderable === "true"
      ? [{ keys: "Shift+J/K", label: "Reorder" }]
      : []),
    // A pending approval answers from every focus (FOCUS.md "Approvals").
    ...(hasApprovalChoice
      ? [{ keys: DESKTOP_APPROVAL_SHORTCUT, label: "Answer", priority: "high" as const }]
      : []),
    ...(status === "busy"
      ? [{ keys: DESKTOP_SHORTCUTS.stop, label: "Stop", priority: "high" as const }]
      : []),
    { keys: DESKTOP_RESIZE_HINT, label: "Resize" },
  ];
  const hints: RegionHint[] = resizingLayout
    ? [
      { keys: "H/L", label: "Resize" },
      { keys: "Shift+H/L", label: "Large step" },
      { keys: "Tab", label: "Next bar" },
      { keys: "Esc/Enter", label: "Done" },
    ]
    : workspaceCommandMode
    // The which-key panel lists every armed key; the status line only says
    // how to leave the layer.
    ? [
      { keys: "Backspace", label: "Back", availability: "available" },
      { keys: "Esc", label: "Cancel", availability: "available" },
    ]
    : ordinaryHints;
  const paneLabel = statusPaneLabel(resizingLayout, focusedRegion, focusedPane, reviewShown, documentMode);
  const regionLabel = workspace.selectedSplitter ?? focusedRegion;

  return (
    <Box
      component="footer"
      data-desktop-status-line
      aria-label="Desktop status line"
      sx={{
        order: 2,
        position: "relative",
        zIndex: 2,
        minHeight: desktopSize(29),
        display: "flex",
        alignItems: "center",
        borderTop: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.56),
        color: "text.secondary",
        userSelect: "none",
        overflow: "hidden",
      }}
    >
      <Stack direction="row" alignItems="center" divider={<Divider orientation="vertical" flexItem />}>
        <Segment
          label={effectiveMode.toUpperCase()}
          color={modeColor}
          tooltip={promptEditorContext && vimEnabled
            ? `Editor Vim mode: ${vimMode}`
            : `Workspace mode: ${mode}`}
          mono
        />
        <Segment label={paneLabel.toUpperCase()} tooltip="Focused workspace pane" mono />
        {regionLabel && regionLabel.split(".").at(-1) !== paneLabel && (
          <Segment
            label={(regionLabel.split(".").at(-1) ?? regionLabel).toUpperCase()}
            tooltip={resizingLayout ? "Selected resize bar" : "Focused region"}
            mono
          />
        )}
        {bufferBadge && (
          <Segment
            label={bufferBadge.label}
            color={bufferBadge.color}
            tooltip={bufferBadge.tooltip}
            mono
          />
        )}
        {imeLabel && (
          <Segment
            label={imeLabel}
            color={ime.phase === "committed" ? "success.main" : "info.main"}
            tooltip={ime.autoInserted
              ? "Cowboy detected native composition and safely entered Vim Insert mode"
              : ime.phase === "committed"
              ? "Native IME composition committed"
              : "Native IME composition is active"}
            mono
          />
        )}
        {macro && (
          <Segment
            label={
              <Stack direction="row" spacing={0.6} alignItems="center">
                <Box
                  component="span"
                  sx={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    bgcolor: "error.main",
                    boxShadow: (theme) =>
                      `0 0 0 3px ${alpha(theme.palette.error.main, 0.12)}`,
                  }}
                />
                <Box component="span">REC @{macro.register}</Box>
                <DesktopKeycap keyLabel="Q" quiet />
                <Box component="span" sx={{ color: "text.secondary" }}>Stop</Box>
              </Stack>
            }
            color="error.main"
            tooltip={`Recording Vim macro into register ${macro.register}. Press Q or click to stop.`}
            mono
            onClick={macro.stop}
          />
        )}
      </Stack>
      <StatusLineHints
        hints={hints}
        allHighPriority={resizingLayout || workspaceCommandMode}
      />
      <Box sx={{ flex: 1 }} />
      <Stack
        direction="row"
        alignItems="center"
        divider={<Divider orientation="vertical" flexItem />}
        sx={{ flexShrink: 0 }}
      >
      {status && <Segment label={status.toUpperCase()} tooltip="Session status" mono />}
        <Segment
          label={
            <Stack
              direction="row"
              spacing={0.6}
              alignItems="center"
              data-desktop-sync-retry={syncLive ? undefined : true}
            >
              <Box
                component="span"
                aria-hidden
                sx={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  bgcolor: "currentColor",
                  flexShrink: 0,
                }}
              />
              <Box component="span">{syncLabel.toUpperCase()}</Box>
              {!syncLive && <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.reconnect} quiet />}
            </Stack>
          }
          color={sync.phase === "live" && sync.outbox.held > 0 ? "warning.main" : `${syncTone}.main`}
          tooltip={syncStatusDetail(sync, now)}
          mono
          {...(syncLive ? {} : { onClick: (): void => retrySyncNow() })}
        />
        <Segment
          label={
            <Stack direction="row" spacing={0.55} alignItems="center">
              <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.commands} quiet />
              <Box component="span">Commands</Box>
            </Stack>
          }
          tooltip="Open the Desktop command palette"
          onClick={(): void => {
            commands.execute("commandPalette.open");
          }}
        />
        <Segment
          label={
            <Stack direction="row" spacing={0.55} alignItems="center">
              <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.shortcuts} quiet />
              <Box component="span">Shortcuts</Box>
            </Stack>
          }
          tooltip={`Open the Desktop keyboard shortcut guide (${DESKTOP_SHORTCUTS.shortcuts})`}
          onClick={(): void => {
            commands.execute("shortcuts.open");
          }}
        />
      </Stack>
    </Box>
  );
});
