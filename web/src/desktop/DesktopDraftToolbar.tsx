import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  AttachFile,
  HistoryOutlined,
  ManageSearch,
  MoreHoriz,
  OpenInNew,
  SaveAlt,
  SaveOutlined,
} from "@mui/icons-material";
import {
  COMPOSER_COMMANDS,
  COMPOSER_COMMANDS_BY_ID,
} from "../composerCommands";
import { useComposerSourceMode } from "../composerSourceMode";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import {
  type DesktopCommand,
  useDesktopCommands,
  useOptionalDesktopCommands,
} from "./commands/DesktopCommandProvider";
import { DesktopShortcut, LeaderKeycap } from "./commands/DesktopKeycap";
import { formatChord } from "./commands/formatChord";
import { desktopOverlayOwnsShortcuts } from "./commands/desktopShortcutScope";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  DESKTOP_COMPOSER_FORMAT_CHORDS,
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_DOCUMENT_KEYS,
  desktopLeaderSequence,
  desktopWorkspaceSequence,
} from "./commands/workspaceShortcuts";

const DOCUMENT = DESKTOP_DOCUMENT_KEYS;
import { isImeComposing, useImeStatus } from "./vim/imeStatusStore";
import { EditorPluginToolbar } from "../editorPlugins/EditorPluginToolbar";

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

function ConnectedToolbar(props: Props): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const composing = useImeStatus().phase === "composing";
  const sourceMode = useComposerSourceMode();
  const moreRef = useRef<HTMLButtonElement>(null);
  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null);
  const [overlay, setOverlay] = useState(false);
  useEffect(() => {
    const update = (): void =>
      setOverlay(desktopOverlayOwnsShortcuts(document));
    const observer = new MutationObserver(update);
    observer.observe(document.body, { childList: true });
    document.addEventListener("focusin", update);
    update();
    return () => {
      observer.disconnect();
      document.removeEventListener("focusin", update);
    };
  }, []);
  const scoped = workspace.focusedRegion === "prompt.composer" && !composing &&
    !overlay;
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
    // The open document's own actions take root keys (`␣Y` `␣H` `␣E`) and
    // run from any focus (Sessions, the title field) while this Draft is the
    // workspace item; which-key lists them first, under "Here". Rename is the
    // title is `␣T`, the cursor to the inline title.
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
  }, []);
  const { register, execute } = useDesktopCommands();
  useEffect(() => {
    const remove = commands.map(register);
    return () => remove.forEach((fn) => fn());
  }, [commands, register]);
  const action = (
    id: string,
    label: string,
    icon: ReactNode,
    run: () => void,
    key?: string,
    disabled = false,
    text = false,
  ): ReactNode => (
    <Tooltip
      key={id}
      title={key
        ? `${label} · ${key.includes("+") ? key : desktopWorkspaceSequence(key)}`
        : label}
    >
      <span
        data-draft-document-secondary={["copy", "history", "export"].includes(
            id,
          )
          ? "true"
          : undefined}
        data-draft-format-tier={id !== "sourceMode" &&
            COMPOSER_COMMANDS_BY_ID[id]
          ? ["bold", "italic", "code"].includes(id)
            ? "primary"
            : ["undo", "redo", "link", "bulletList"].includes(id)
            ? "secondary"
            : "extra"
          : undefined}
      >
        <Button
          data-draft-tool
          data-draft-action={id}
          ref={id === "more" ? moreRef : undefined}
          aria-pressed={id === "sourceMode" ? sourceMode : undefined}
          aria-haspopup={id === "more" ? "menu" : undefined}
          aria-expanded={id === "more" ? moreAnchor !== null : undefined}
          aria-label={label}
          size="small"
          color="inherit"
          disabled={disabled || composing}
          onPointerDown={(e) => {
            if (e.button === 0) e.preventDefault();
          }}
          onClick={run}
          sx={{
            minWidth: 0,
            minHeight: "2.25rem",
            px: "0.5rem",
            gap: "0.375rem",
            textTransform: "none",
            fontSize: "0.8125rem",
            "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
          }}
        >
          {icon}
          {text && <Box component="span" data-draft-action-label>{label}</Box>}
          {key && key.includes("+") && (
            // Rich text: a direct chord, live only while the editor has focus.
            <DesktopShortcut
              shortcut={key}
              compact
              quiet
              availability={shortcutAvailability(scoped && !disabled)}
            />
          )}
          {key && !key.includes("+") && (
            <LeaderKeycap
              leaderKey={key}
              // Document keys run from any focus; the rest belong to the
              // editor's scope.
              scopeAvailable={(Object.values(DOCUMENT).includes(
                  key as typeof DOCUMENT[keyof typeof DOCUMENT],
                )
                ? !composing
                : scoped) && !disabled}
              {...(id === "more" && moreAnchor
                ? { availability: "active" as const }
                : {})}
            />
          )}
        </Button>
      </span>
    </Tooltip>
  );
  return (
    <Box
      data-desktop-draft-toolbar
      sx={{
        borderTop: 1,
        borderColor: "divider",
        px: "0.75rem",
        py: "0.375rem",
        flexShrink: 0,
        color: "text.secondary",
        containerType: "inline-size",
        containerName: "draft-tools",
        "& [data-draft-format-tier], & [data-draft-action-label], & [data-draft-document-secondary], & [data-draft-readable-width]":
          {
            display: "none",
          },
        "@container draft-tools (min-width: 32rem)": {
          "& [data-draft-format-tier='primary']": { display: "inline-flex" },
          "& [data-draft-action-label]": { display: "inline" },
          "& [data-draft-document-secondary], & [data-draft-readable-width]": {
            display: "inline-flex",
          },
        },
        "@container draft-tools (min-width: 42rem)": {
          "& [data-draft-format-tier='secondary']": { display: "inline-flex" },
        },
        "@container draft-tools (min-width: 56rem)": {
          "& [data-draft-format-tier='extra']": { display: "inline-flex" },
        },
      }}
    >
      <Box
        data-draft-format-toolbar
        role="group"
        aria-label="Draft formatting"
        sx={{
          display: "flex",
          alignItems: "center",
          flexWrap: "wrap",
          gap: "0.125rem",
        }}
      >
        {props.toolbar.filter((id) =>
          !["slash", "mention", "attach", "sourceMode"].includes(id)
        ).map((id) => {
          const c = COMPOSER_COMMANDS_BY_ID[id];
          return c && action(
            id,
            c.label,
            c.icon,
            () => props.onFormat(id),
            id === "sourceMode"
              ? DESKTOP_WORKSPACE_KEYS.toggleSourceMode
              : DESKTOP_COMPOSER_FORMAT_CHORDS[id],
            !props.writable && id !== "sourceMode",
          );
        })}
        {action("more", "More formatting", <MoreHoriz />, () =>
          setMoreAnchor(moreRef.current), DESKTOP_WORKSPACE_KEYS.composerMore)}
        {action(
          "sourceMode",
          sourceMode ? "Switch to live preview" : "Switch to Source mode",
          COMPOSER_COMMANDS_BY_ID.sourceMode!.icon,
          () =>
            props.onFormat("sourceMode"),
          DESKTOP_WORKSPACE_KEYS.toggleSourceMode,
        )}
        {action(
          "attach",
          "Attach",
          <AttachFile />,
          props.onAttach,
          DESKTOP_WORKSPACE_KEYS.composerAttach,
          !props.writable,
        )}
        <EditorPluginToolbar kind="document" disabled={!props.writable || composing} />
        <Box sx={{ flex: 1 }} />
        <Button
          size="small"
          color="inherit"
          data-draft-readable-width
          aria-pressed={props.readableWidth}
          onClick={props.onReadableWidth}
          sx={{
            textTransform: "none",
            fontSize: "0.8125rem",
            minHeight: "2.25rem",
            bgcolor: props.readableWidth ? "action.selected" : undefined,
          }}
        >
          Readable width
          <Box component="span" sx={{ display: "inline-flex", ml: "0.375rem" }}>
            <LeaderKeycap leaderKey={DOCUMENT.readableWidth} scopeAvailable={!composing} />
          </Box>
        </Button>
      </Box>
      <Box
        role="group"
        aria-label="Draft document actions"
        sx={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: "0.25rem",
        }}
      >
        <Tooltip title="Save locally now. Drafts also save automatically; server synchronization continues in the background.">
          <span>
            <Button
              data-draft-action="save"
              aria-label="Save"
              size="small"
              color="inherit"
              disabled={!props.writable || composing}
              onPointerDown={(e) => {
                if (e.button === 0) {
                  e.preventDefault();
                }
              }}
              onClick={props.onSave}
              sx={{
                textTransform: "none",
                minHeight: "2.25rem",
                gap: "0.5rem",
                "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
              }}
            >
              {/* The label joins the density tiers so Save + its keycap fit a
                  narrow pane at large reading sizes. */}
              <SaveOutlined />
              <Box component="span" data-draft-action-label>Save</Box>
              <DesktopShortcut
                shortcut="Mod+S"
                compact
                quiet
                availability={shortcutAvailability(scoped && props.writable)}
              />
            </Button>
          </span>
        </Tooltip>
        {action(
          "copy",
          "Copy to Session",
          <OpenInNew />,
          props.onCopy,
          DOCUMENT.copy,
          false,
          true,
        )}
        {action(
          "history",
          "History",
          <HistoryOutlined />,
          props.onHistory,
          DOCUMENT.history,
          props.historyLoading,
          true,
        )}
        {action(
          "export",
          "Export Markdown",
          <SaveAlt />,
          props.onExport,
          DOCUMENT.export,
        )}
        <Tooltip title="Search all editing and document actions">
          <Button
            size="small"
            color="inherit"
            aria-label="Command Palette"
            data-draft-document-secondary
            onClick={() =>
              execute("commandPalette.open")}
            sx={{ textTransform: "none", minHeight: "2.25rem", gap: "0.5rem" }}
          >
            <ManageSearch sx={{ fontSize: "1.25rem" }} />
            <Box component="span" data-draft-action-label>Commands</Box>
            <DesktopShortcut
              shortcut={DESKTOP_SHORTCUTS.commands}
              compact
              quiet
              availability={shortcutAvailability(!overlay && !composing)}
            />
          </Button>
        </Tooltip>
        <Box sx={{ flex: 1 }} />
        <Typography
          variant="caption"
          role="status"
          sx={{
            px: "0.25rem",
            whiteSpace: "normal",
            overflowWrap: "anywhere",
            maxWidth: "100%",
          }}
        >
          {props.status}
        </Typography>
      </Box>
      <Menu
        anchorEl={moreAnchor}
        slotProps={{ transition: { timeout: 0 } }}
        open={moreAnchor !== null}
        onClose={() =>
          setMoreAnchor(null)}
      >
        <MenuItem
          onClick={() => {
            setMoreAnchor(null);
            props.onCopy();
          }}
        >
          <ListItemIcon>
            <OpenInNew />
          </ListItemIcon>
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
          onClick={() => {
            setMoreAnchor(null);
            props.onHistory();
          }}
        >
          <ListItemIcon>
            <HistoryOutlined />
          </ListItemIcon>
          <ListItemText>Recovery history</ListItemText>
          <DesktopShortcut
            shortcut={desktopWorkspaceSequence(DOCUMENT.history)}
            compact
            quiet
            availability="inactive"
          />
        </MenuItem>
        <MenuItem
          onClick={() => {
            setMoreAnchor(null);
            props.onExport();
          }}
        >
          <ListItemIcon>
            <SaveAlt />
          </ListItemIcon>
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
          onClick={() => {
            setMoreAnchor(null);
            props.onReadableWidth();
          }}
        >
          <ListItemText>Readable width</ListItemText>
        </MenuItem>
        <MenuItem
          onClick={() => {
            setMoreAnchor(null);
            execute("commandPalette.open");
          }}
        >
          <ListItemIcon>
            <ManageSearch />
          </ListItemIcon>
          <ListItemText>Command Palette</ListItemText>
          <DesktopShortcut
            shortcut={DESKTOP_SHORTCUTS.commands}
            compact
            quiet
            availability="inactive"
          />
        </MenuItem>
        {COMPOSER_COMMANDS.filter((c) =>
          !["slash", "mention", "attach"].includes(c.id)
        ).map((c) => (
          <MenuItem
            key={c.id}
            disabled={composing || (!props.writable && c.id !== "sourceMode")}
            onClick={() => {
              setMoreAnchor(null);
              props.onFormat(c.id);
            }}
          >
            <ListItemIcon>{c.icon}</ListItemIcon>
            <ListItemText>{c.label}</ListItemText>
            {DESKTOP_COMPOSER_FORMAT_CHORDS[c.id] && (
              <DesktopShortcut
                shortcut={DESKTOP_COMPOSER_FORMAT_CHORDS[c.id]!}
                compact
                quiet
                availability="inactive"
              />
            )}
          </MenuItem>
        ))}
      </Menu>
    </Box>
  );
}
