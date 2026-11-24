import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  Divider,
  ListItemIcon,
  ListItemText,
  MenuItem,
  Typography,
} from "@mui/material";
import {
  HistoryOutlined,
  ManageSearch,
  OpenInNew,
  SaveAlt,
  SaveOutlined,
  Send,
} from "@mui/icons-material";
import { COMPOSER_COMMANDS } from "../composerCommands";
import { useComposerSourceMode } from "../composerSourceMode";
import { HintTooltip } from "../HintTooltip";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import {
  type DesktopCommand,
  useDesktopCommandActions,
  useOptionalDesktopCommands,
} from "./commands/DesktopCommandProvider";
import { DesktopShortcut, LeaderKeycap } from "./commands/DesktopKeycap";
import { desktopOverlayOwnsShortcuts } from "./commands/desktopShortcutScope";
import { formatChord } from "./commands/formatChord";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_DOCUMENT_KEYS,
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderSequence,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";
import {
  DesktopEditorFormatRow,
  DesktopToolbarActionButton,
} from "./DesktopEditorFormatRow";
import { desktopToolbarActionSx } from "./desktopToolbarAction";
import { isImeComposing, useImeStatus } from "./vim/imeStatusStore";

const DOCUMENT = DESKTOP_DOCUMENT_KEYS;

interface Props {
  fallback: ReactNode;
  toolbar: readonly string[];
  status: string;
  readableWidth: boolean;
  writable: boolean;
  historyLoading: boolean;
  onFormat: (id: string) => void;
  onSave: () => void;
  onAttach: () => void;
  onCopy: () => void;
  onUseInSession?: (() => void) | undefined;
  onHistory: () => void;
  onExport: () => void;
  onReadableWidth: () => void;
  /** Move the cursor to the title (Vim Normal when Vim is on). */
  onTitle: () => void;
}

export default function DesktopDraftToolbar(props: Props): ReactNode {
  return useOptionalDesktopCommands()
    ? <ConnectedToolbar {...props} />
    : props.fallback;
}

function useDraftToolbarCommands(
  props: Props,
  moreRef: React.RefObject<HTMLButtonElement | null>,
  setMoreAnchor: (el: HTMLElement | null) => void,
): { execute: (id: string) => void } {
  const state = useRef({ props });
  state.current = { props };
  const commands = useMemo<DesktopCommand[]>(() => {
    const action = (
      id: string,
      title: string,
      run: (p: Props) => void,
      key?: string,
      writable = false,
    ): DesktopCommand => ({
      id,
      title,
      group: "Draft document",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      ...(key ? { sequence: desktopLeaderSequence(key) } : {}),
      when: () =>
        !isImeComposing() &&
        (!writable || state.current.props.writable) &&
        (id !== "document.history" || !state.current.props.historyLoading),
      disabledReason:
        "Finish composition or resolve this draft’s save error first",
      run: () => run(state.current.props),
    });
    const documentAction = (
      id: string,
      title: string,
      run: (p: Props) => void,
      key?: string,
    ): DesktopCommand => {
      const { contexts: _contexts, regions: _regions, ...command } = action(
        id,
        title,
        run,
      );
      return {
        ...command,
        ...(key ? { sequence: desktopLeaderSequence(key) } : {}),
        surface: "document",
        contextual: true,
        leaderAnywhere: true,
      };
    };
    return [
      {
        ...action(
          "document.save",
          "Save draft on this device",
          (p) => p.onSave(),
          undefined,
          true,
        ),
        shortcut: DESKTOP_SHORTCUTS.saveDraft,
        consumeWhenDisabled: true,
      },
      {
        ...action(
          "document.useInSession",
          "Use draft in Session…",
          (p) => p.onUseInSession?.(),
        ),
        shortcut: "Mod+Enter",
        when: () =>
          !isImeComposing() && Boolean(state.current.props.onUseInSession),
        disabledReason: "Choose a destination session first",
      },
      action(
        "composer.more",
        "More draft formatting",
        () => setMoreAnchor(moreRef.current),
        DESKTOP_WORKSPACE_KEYS.composerMore,
      ),
      action(
        "composer.attach",
        "Attach file to draft",
        (p) => p.onAttach(),
        DESKTOP_WORKSPACE_KEYS.composerAttach,
        true,
      ),
      documentAction(
        "document.title",
        "Go to title",
        (p) => p.onTitle(),
        DOCUMENT.title,
      ),
      documentAction(
        "document.copyToSession",
        "Copy draft to Session drafts",
        (p) => p.onCopy(),
        DOCUMENT.copy,
      ),
      documentAction(
        "document.history",
        "Draft recovery history",
        (p) => p.onHistory(),
        DOCUMENT.history,
      ),
      documentAction(
        "document.export",
        "Export draft as Markdown",
        (p) => p.onExport(),
        DOCUMENT.export,
      ),
      documentAction(
        "document.readableWidth",
        "Toggle draft readable width",
        (p) => p.onReadableWidth(),
        DOCUMENT.readableWidth,
      ),
      action(
        "composer.toggleSourceMode",
        "Toggle Source mode",
        (p) => p.onFormat("sourceMode"),
        DESKTOP_WORKSPACE_KEYS.toggleSourceMode,
      ),
      ...COMPOSER_COMMANDS.filter((c) =>
        !["slash", "mention", "attach", "sourceMode"].includes(c.id)
      ).map((c) => ({
        ...action(
          `composer.format.${c.id}`,
          c.label,
          (p) => p.onFormat(c.id),
          undefined,
          true,
        ),
        ...formatChord(c.id),
      })),
    ];
  }, [moreRef, setMoreAnchor]);
  const { register, execute } = useDesktopCommandActions();
  useEffect(() => {
    const remove = commands.map(register);
    return () => remove.forEach((fn) => fn());
  }, [commands, register]);
  return { execute };
}

function DraftDocumentMenuItems({
  props,
  close,
  execute,
}: {
  props: Props;
  close: () => void;
  execute: (id: string) => void;
}): React.JSX.Element {
  return (
    <>
      {props.onUseInSession && (
        <MenuItem
          onClick={() => {
            close();
            props.onUseInSession?.();
          }}
        >
          <ListItemIcon><Send fontSize="small" /></ListItemIcon>
          <ListItemText>Use in Session…</ListItemText>
          <DesktopShortcut shortcut="Mod+Enter" compact quiet availability="inactive" />
        </MenuItem>
      )}
      <MenuItem onClick={() => { close(); props.onCopy(); }}>
        <ListItemIcon><OpenInNew fontSize="small" /></ListItemIcon>
        <ListItemText>Copy to Session</ListItemText>
        <DesktopShortcut
          shortcut={desktopWorkspaceSequence(DOCUMENT.copy)}
          compact
          quiet
          availability="inactive"
        />
      </MenuItem>
      <MenuItem
        disabled={props.historyLoading}
        onClick={() => { close(); props.onHistory(); }}
      >
        <ListItemIcon><HistoryOutlined fontSize="small" /></ListItemIcon>
        <ListItemText>Recovery history</ListItemText>
        <DesktopShortcut
          shortcut={desktopWorkspaceSequence(DOCUMENT.history)}
          compact
          quiet
          availability="inactive"
        />
      </MenuItem>
      <MenuItem onClick={() => { close(); props.onExport(); }}>
        <ListItemIcon><SaveAlt fontSize="small" /></ListItemIcon>
        <ListItemText>Export Markdown</ListItemText>
        <DesktopShortcut
          shortcut={desktopWorkspaceSequence(DOCUMENT.export)}
          compact
          quiet
          availability="inactive"
        />
      </MenuItem>
      <MenuItem
        aria-checked={props.readableWidth}
        role="menuitemcheckbox"
        onClick={() => { close(); props.onReadableWidth(); }}
      >
        <ListItemText>Readable width</ListItemText>
      </MenuItem>
      <MenuItem onClick={() => { close(); execute("commandPalette.open"); }}>
        <ListItemIcon><ManageSearch fontSize="small" /></ListItemIcon>
        <ListItemText>Command Palette</ListItemText>
        <DesktopShortcut
          shortcut={DESKTOP_SHORTCUTS.commands}
          compact
          quiet
          availability="inactive"
        />
      </MenuItem>
      <Divider />
    </>
  );
}

function DraftDocumentActionRow({
  props,
  scoped,
  composing,
  overlay,
  execute,
}: {
  props: Props;
  scoped: boolean;
  composing: boolean;
  overlay: boolean;
  execute: (id: string) => void;
}): React.JSX.Element {
  const prefixAvailable = !composing && !overlay;
  const action = (
    id: string,
    label: string,
    icon: ReactNode,
    onClick: () => void,
    key: string,
    disabled = false,
  ): ReactNode => (
    <Box key={id} component="span" data-draft-document-secondary="true" sx={{ display: "inline-flex" }}>
      <DesktopToolbarActionButton
        spec={{ id, label, icon, onClick, key, disabled, anywhere: true }}
        scoped={scoped}
        sourceScope={scoped}
        prefixAvailable={prefixAvailable}
        overlayOwnsKeys={overlay}
      />
    </Box>
  );

  return (
    <Box
      role="group"
      aria-label="Draft document actions"
      sx={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 0.25,
        px: 1,
        pb: 0.5,
      }}
    >
      <DesktopToolbarActionButton
        spec={{
          id: "save",
          label: "Save",
          title: "Save locally now",
          icon: <SaveOutlined />,
          shortcut: "Mod+S",
          disabled: !props.writable || composing,
          onClick: props.onSave,
        }}
        scoped={scoped}
        sourceScope={scoped}
        prefixAvailable={prefixAvailable}
        overlayOwnsKeys={overlay}
      />
      {action("copy", "Copy to Session", <OpenInNew />, props.onCopy, DOCUMENT.copy)}
      {action("history", "History", <HistoryOutlined />, props.onHistory, DOCUMENT.history, props.historyLoading)}
      {action("export", "Export Markdown", <SaveAlt />, props.onExport, DOCUMENT.export)}
      <Box component="span" data-draft-document-secondary="true" sx={{ display: "inline-flex" }}>
        <DesktopToolbarActionButton
          spec={{
            id: "commands",
            label: "Commands",
            title: "Search all editing and document actions",
            icon: <ManageSearch />,
            shortcut: DESKTOP_SHORTCUTS.commands,
            disabled: composing,
            anywhere: true,
            onClick: () => execute("commandPalette.open"),
          }}
          scoped={scoped}
          sourceScope={scoped}
          prefixAvailable={prefixAvailable}
          overlayOwnsKeys={overlay}
        />
      </Box>
      <Box sx={{ flex: 1 }} />
      <Typography
        variant="caption"
        role="status"
        color="text.secondary"
        sx={{
          px: "0.25rem",
          whiteSpace: "normal",
          overflowWrap: "anywhere",
          maxWidth: "100%",
        }}
      >
        {props.status}
      </Typography>
      {props.onUseInSession && (
        <HintTooltip
          title="Copy to a Session's drafts and open in its Prompt canvas"
          shortcut="Mod+Enter"
        >
          <span>
            <Button
              variant="contained"
              size="small"
              data-draft-action="use-in-session"
              disabled={composing}
              onPointerDown={(e) => { if (e.button === 0) e.preventDefault(); }}
              onClick={props.onUseInSession}
              sx={{
                ...desktopToolbarActionSx,
                px: "0.625rem",
                gap: "0.375rem",
              }}
            >
              <Send sx={{ fontSize: "1rem !important", width: "1rem !important", height: "1rem !important" }} />
              <Box component="span" data-draft-action-label>Use in Session…</Box>
              <DesktopShortcut
                shortcut="Mod+Enter"
                compact
                quiet
                availability={shortcutAvailability(scoped && !composing)}
              />
            </Button>
          </span>
        </HintTooltip>
      )}
    </Box>
  );
}

function ConnectedToolbar(props: Props): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const composing = useImeStatus().phase === "composing";
  const sourceMode = useComposerSourceMode();
  const moreRef = useRef<HTMLButtonElement>(null);
  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null);
  const [overlay, setOverlay] = useState(false);

  useEffect(() => {
    const update = (): void => setOverlay(desktopOverlayOwnsShortcuts(document));
    const observer = new MutationObserver(update);
    observer.observe(document.body, { childList: true });
    document.addEventListener("focusin", update);
    update();
    return () => {
      observer.disconnect();
      document.removeEventListener("focusin", update);
    };
  }, []);

  const scoped = workspace.focusedRegion === "prompt.composer" && !composing && !overlay;
  const { execute } = useDraftToolbarCommands(props, moreRef, setMoreAnchor);

  return (
    <Box
      data-desktop-draft-toolbar
      sx={{
        containerType: "inline-size",
        containerName: "draft-tools",
        borderTop: 1,
        borderColor: "divider",
        display: "flex",
        flexDirection: "column",
        flexShrink: 0,
        minWidth: 0,
        "& [data-format-tier], & [data-composer-action-label], & [data-draft-action-label], & [data-draft-document-secondary], & [data-draft-readable-width]": {
          display: "none",
        },
        "@container draft-tools (min-width: 30rem)": {
          "& [data-format-tier='primary']": { display: "inline-flex" },
          "& [data-draft-action-label]": { display: "inline" },
        },
        "@container draft-tools (min-width: 38rem)": {
          "& [data-draft-document-secondary], & [data-draft-readable-width]": {
            display: "inline-flex",
          },
        },
        "@container draft-tools (min-width: 46rem)": {
          "& [data-format-tier='secondary']": { display: "inline-flex" },
          "& [data-composer-action-label]": { display: "inline" },
        },
        "@container draft-tools (min-width: 54rem)": {
          "& [data-format-tier='extra']": { display: "inline-flex" },
        },
      }}
    >
      <Box data-draft-format-toolbar>
        <DesktopEditorFormatRow
          kind="document"
          ariaLabel="Draft formatting"
          menuId="desktop-draft-more-menu"
          formatIds={props.toolbar.filter((id) =>
            !["slash", "mention", "attach", "sourceMode"].includes(id)
          )}
          canInsert={false}
          canAttach={props.writable}
          writable={props.writable}
          composing={composing}
          sourceMode={sourceMode}
          scoped={scoped}
          sourceScope={scoped}
          prefixAvailable={!composing && !overlay}
          overlayOwnsKeys={overlay}
          moreRef={moreRef}
          moreAnchor={moreAnchor}
          onMoreOpen={setMoreAnchor}
          onMoreClose={() => setMoreAnchor(null)}
          onAttach={props.onAttach}
          onFormat={props.onFormat}
          onToggleSourceMode={() => props.onFormat("sourceMode")}
          extraMenuItems={
            <DraftDocumentMenuItems
              props={props}
              close={() => setMoreAnchor(null)}
              execute={execute}
            />
          }
          trailing={
            <Button
              size="small"
              color="inherit"
              data-draft-readable-width
              aria-pressed={props.readableWidth}
              onPointerDown={(event) => event.preventDefault()}
              onMouseDown={(event) => event.preventDefault()}
              onClick={props.onReadableWidth}
              sx={{
                ...desktopToolbarActionSx,
                bgcolor: props.readableWidth ? "action.selected" : undefined,
              }}
            >
              Readable width
              <Box component="span" sx={{ display: "inline-flex", ml: "0.25rem" }}>
                <LeaderKeycap leaderKey={DOCUMENT.readableWidth} scopeAvailable={!composing} />
              </Box>
            </Button>
          }
        />
      </Box>
      <DraftDocumentActionRow
        props={props}
        scoped={scoped}
        composing={composing}
        overlay={overlay}
        execute={execute}
      />
    </Box>
  );
}
