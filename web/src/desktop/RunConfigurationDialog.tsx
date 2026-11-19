import { ScrollEdges } from "@cowboy/app-shell";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Box, Button, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import { Tune } from "@mui/icons-material";
import { ConfigRevertBack } from "../ConfigRevertBack";
import { Kbd, useConfirmEnter } from "../Kbd";
import { ENTER_LABEL, MOD_LABEL } from "../platform";
import type { ConfigOption, SessionMeta, Status } from "../protocol";
import {
  providerConfigOptionDisabled,
  type ProviderConfigOptionPresentation,
} from "../providerConfigOptions";
import {
  type RunConfigDraft,
  runConfigDraftChanges,
  runConfigPresets,
  withRunConfigDraft,
} from "../runConfigPresets";
import { setSessionConfigOptions } from "../store";
import { SessionToolsSection } from "../AgentToolsPanel";
import { SessionContextSettings } from "../SessionContextSettings";
import { DesktopModal } from "./DesktopModal";
import type { DesktopShortcutGroup } from "./DesktopShortcutBar";
import {
  ConfigOptionControl,
  RecommendedPresetControls,
} from "./RunConfigControls";
import { useRunConfigKeyboard } from "./runConfigNavigation";
import { currentOptionName, optionShortcut } from "./runConfigOptions";

// Run configuration (FOCUS.md "Run configuration"): the full-window
// workbench for how this agent runs, its tools and its context.

/** The short panes (Agent, then Tools) stack in the left column so their
 *  content, not empty pane bottoms, fills its height; Context takes the
 *  right column, where the session tree grows to the window. One column on
 *  a narrow window. Each pane scrolls on its own and the one holding the
 *  cursor is outlined. DOM order (Agent, Tools, Context) is pane order. */
const runConfigPanelSx = {
  flex: 1,
  minWidth: 0,
  minHeight: 0,
  display: "grid",
  gap: 1.25,
  p: 1.25,
  overflow: "auto",
  gridTemplateColumns: "minmax(0, 1fr)",
  gridTemplateAreas: '"agent" "tools" "context"',
  "@media (min-width: 980px)": {
    overflow: "hidden",
    gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)",
    gridTemplateRows: "max-content minmax(0, 1fr)",
    gridTemplateAreas: '"agent context" "tools context"',
  },
} as const;

function RunConfigPane({
  pane,
  title,
  caption,
  footer,
  children,
}: {
  pane: "agent" | "context" | "tools";
  title: string;
  caption: string;
  /** Pinned under the pane's scrolling body. */
  footer?: React.ReactNode;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-config-pane={pane}
      tabIndex={-1}
      sx={{
        gridArea: pane,
        minWidth: 0,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        border: 1,
        borderColor: "divider",
        borderRadius: 2,
        bgcolor: (theme) => alpha(theme.palette.background.default, 0.32),
        outline: "none",
        transition: "border-color 120ms, box-shadow 120ms",
        "&:focus-within": {
          borderColor: (theme) => alpha(theme.palette.primary.main, 0.7),
          boxShadow: (theme) =>
            `0 0 0 1px ${alpha(theme.palette.primary.main, 0.28)}`,
        },
        "&:focus-within [data-config-pane-title]": { color: "primary.main" },
        // The field under the cursor: its whole row, not only the control.
        "& [data-config-row], & [data-settings-row]": {
          mx: -1,
          px: 1,
          borderRadius: 1.5,
          transition: "background-color 120ms",
        },
        "& [data-config-row]": { py: 0.5 },
        "& [data-config-row][data-config-changed]": {
          bgcolor: (theme) => alpha(theme.palette.warning.main, 0.07),
        },
        "& [data-settings-row]": { py: 0.25, minHeight: 36 },
        "& [data-config-row]:has(:focus-visible), & [data-settings-row]:has(:focus-visible)":
          {
            bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08),
            boxShadow: (theme) => `inset 3px 0 0 ${theme.palette.primary.main}`,
          },
      }}
    >
      <Stack
        direction="row"
        alignItems="baseline"
        spacing={1}
        sx={{
          px: 2,
          pt: 1.25,
          pb: 1,
          borderBottom: 1,
          borderColor: "divider",
          minWidth: 0,
        }}
      >
        <Typography
          data-config-pane-title
          variant="overline"
          fontWeight={800}
          color="text.secondary"
          sx={{ letterSpacing: 0.9, lineHeight: 1.4 }}
        >
          {title}
        </Typography>
        <Typography
          variant="caption"
          color="text.secondary"
          noWrap
          sx={{ flex: 1, minWidth: 0 }}
        >
          {caption}
        </Typography>
      </Stack>
      <Stack
        spacing={1}
        sx={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          px: 2,
          py: 1.25,
        }}
      >
        {children}
      <ScrollEdges /></Stack>
      {footer}
    </Box>
  );
}

/** The Agent pane's pending edits: what Apply sends, and the way out. */
function RunConfigApplyBar({
  count,
  applying,
  discardArmed,
  error,
  onApply,
  onDiscard,
}: {
  count: number;
  applying: boolean;
  discardArmed: boolean;
  error: string;
  onApply: () => void;
  onDiscard: () => void;
}): React.JSX.Element {
  return (
    <Stack
      data-config-apply-bar
      direction="row"
      alignItems="center"
      spacing={1}
      sx={{
        px: 2,
        py: 1,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.warning.main, 0.08),
      }}
    >
      <Typography
        variant="caption"
        role="status"
        color={error || discardArmed ? "error.main" : "text.secondary"}
        sx={{ flex: 1, minWidth: 0, fontWeight: 650 }}
      >
        {error ||
          (discardArmed
            ? "Unsaved changes · Esc again discards them"
            : `${count} unsaved ${
              count === 1 ? "change" : "changes"
            } · the agent keeps its current settings until you apply`)}
      </Typography>
      {count > 0 && (
        <>
          <Button
            size="small"
            disabled={applying}
            onClick={onDiscard}
            sx={{ textTransform: "none" }}
          >
            Discard
          </Button>
          <Button
            size="small"
            variant="contained"
            disableElevation
            disabled={applying}
            onClick={onApply}
            data-leader-key="s"
            sx={{ textTransform: "none", gap: 0.75 }}
          >
            {applying ? "Applying…" : "Apply"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} availability="available" />
          </Button>
        </>
      )}
    </Stack>
  );
}

/** The Agent pane's draft: what it shows, what Apply sends, and the
 *  close that warns before it discards. */
function useRunConfigDraft(
  sessionId: string,
  options: readonly ConfigOption[],
  configOpen: boolean,
  onClose: () => void,
) {
  // Agent options are edited as a draft and sent together on Apply: stepping
  // through choices must not switch the live agent (and its context or
  // cache) once per key. Tools and linked sessions keep their own saves.
  const [configDraft, setConfigDraft] = useState<RunConfigDraft>({});
  const [discardArmed, setDiscardArmed] = useState(false);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState("");
  const draftOptions = useMemo(
    () => withRunConfigDraft(options, configDraft),
    [options, configDraft],
  );
  const draftChanges = useMemo(
    () => runConfigDraftChanges(options, configDraft),
    [options, configDraft],
  );
  const unsaved = draftChanges.length > 0;
  useEffect(() => {
    if (configOpen) return;
    setConfigDraft({});
    setDiscardArmed(false);
    setApplyError("");
  }, [configOpen, sessionId]);
  const editConfig = useCallback(
    (id: string, value: string | boolean): void => {
      setConfigDraft((draft) => ({ ...draft, [id]: value }));
      setDiscardArmed(false);
      setApplyError("");
    },
    [],
  );
  const discardConfig = useCallback((): void => {
    setConfigDraft({});
    setDiscardArmed(false);
    setApplyError("");
  }, []);
  const applyConfig = async (): Promise<void> => {
    if (applying || !unsaved) return;
    setApplying(true);
    setApplyError("");
    try {
      await setSessionConfigOptions(sessionId, draftChanges);
      setConfigDraft({});
      setDiscardArmed(false);
    } catch (cause) {
      setApplyError(
        cause instanceof Error ? cause.message : "Could not apply changes",
      );
    } finally {
      setApplying(false);
    }
  };
  // Closing never drops edits silently: the first Esc warns, the second
  // discards them.
  const closeConfig = (): void => {
    if (unsaved && !discardArmed) {
      setDiscardArmed(true);
      return;
    }
    onClose();
  };
  useConfirmEnter(configOpen && unsaved, () => void applyConfig(), {
    suppressBareEnter: false,
  });
  const savedNames = new Map(
    draftChanges.map(({ configId }) => {
      const saved = options.find((option) => option.id === configId);
      return [configId, saved ? currentOptionName(saved) : ""] as const;
    }),
  );
  return {
    draftOptions,
    draftChanges,
    unsaved,
    discardArmed,
    applying,
    applyError,
    editConfig,
    discardConfig,
    applyConfig,
    closeConfig,
    savedNames,
  };
}

/** The footer: the panel's keymap, then Apply and the close it guards. */
function runConfigShortcutGroups({ presets, direct, unsaved, discardArmed }: {
  presets: number;
  direct: readonly string[];
  unsaved: boolean;
  discardArmed: boolean;
}): DesktopShortcutGroup[] {
  return [{
    label: "Navigate",
    slots: [
      { shortcut: "J/K", label: "Field" },
      { shortcut: "H/L", label: "Change" },
      { shortcut: "[/]", label: "Pane" },
      { shortcut: "gg/G", label: "First · Last" },
      ...(presets > 0
        ? [{
          shortcut: Array.from({ length: presets }, (_, i) => String(i + 1))
            .join("/"),
          label: "Preset",
        }]
        : []),
      ...(direct.length > 0
        ? [{ shortcut: direct.join("/"), label: "Direct" }]
        : []),
      { shortcut: ENTER_LABEL, label: "Open · Toggle" },
      { shortcut: "␣", label: "Labels" },
    ],
  }, {
    slots: [
      {
        shortcut: `${MOD_LABEL}${ENTER_LABEL}`,
        label: "Apply",
        availability: unsaved ? "available" : "inactive",
      },
      {
        shortcut: "Esc",
        label: unsaved ? (discardArmed ? "Discard" : "Close…") : "Close",
      },
    ],
  }];
}

export function RunConfigurationDialog({
  open,
  onClose,
  sessionId,
  session,
  status,
  dead,
  options,
  optionPresentations,
}: {
  open: boolean;
  onClose: () => void;
  sessionId: string;
  session: SessionMeta | undefined;
  status: Status;
  dead: boolean;
  options: readonly ConfigOption[];
  optionPresentations: ReadonlyMap<string, ProviderConfigOptionPresentation>;
}): React.JSX.Element {
  const panelRef = useRef<HTMLDivElement>(null);
  useRunConfigKeyboard(open, panelRef);
  const directConfigShortcuts = options.map(optionShortcut).filter(
    (shortcut): shortcut is string => shortcut !== undefined,
  );
  const recommendedPresets = runConfigPresets(
    session?.provider,
    options,
    session?.provider_version,
    session?.provider_generation_digest,
  );
  const {
    draftOptions,
    draftChanges,
    unsaved,
    discardArmed,
    applying,
    applyError,
    editConfig,
    discardConfig,
    applyConfig,
    closeConfig,
    savedNames,
  } = useRunConfigDraft(sessionId, options, open, onClose);
  return (
    <DesktopModal
      open={open}
      onClose={closeConfig}
      title="Run configuration"
      description={dead
        ? "Changes are saved for your next message once you apply them; the session stays asleep. Tools and linked sessions save as you change them."
        : "Agent changes wait for Apply; Back restores them until your next message. Tools and linked sessions save as you change them."}
      icon={<Tune sx={{ color: "primary.main" }} />}
      fill
      shortcutGroups={runConfigShortcutGroups({
        presets: recommendedPresets.length,
        direct: directConfigShortcuts,
        unsaved,
        discardArmed,
      })}
    >
      <Box
        ref={panelRef}
        data-desktop-shortcut-scope="exclusive"
        data-run-config-panel
        sx={runConfigPanelSx}
      >
        <RunConfigPane
          pane="agent"
          title="Agent"
          caption="How this agent runs"
          footer={unsaved || applyError
            ? (
              <RunConfigApplyBar
                count={draftChanges.length}
                applying={applying}
                discardArmed={discardArmed}
                error={applyError}
                onApply={(): void => void applyConfig()}
                onDiscard={discardConfig}
              />
            )
            : undefined}
        >
          {!unsaved && (
            <ConfigRevertBack sessionId={sessionId} options={options} />
          )}
          {recommendedPresets.length > 0 && (
            <RecommendedPresetControls
              presets={recommendedPresets}
              options={draftOptions}
              onChange={editConfig}
            />
          )}
          {draftOptions.map((option) => (
            <ConfigOptionControl
              key={option.id}
              option={option}
              presentation={optionPresentations.get(option.id)}
              savedName={savedNames.get(option.id)}
              onChange={editConfig}
              disabled={applying || providerConfigOptionDisabled(
                status,
                optionPresentations.get(option.id),
              )}
            />
          ))}
        </RunConfigPane>
        <RunConfigPane
          pane="tools"
          title="Tools"
          caption="What this agent may call and remember"
        >
          <SessionToolsSection sessionId={sessionId} heading={false} />
        </RunConfigPane>
        <RunConfigPane
          pane="context"
          title="Context"
          caption="Where the work goes, and what the next message reads"
        >
          {session && <SessionContextSettings session={session} inlineLinks />}
        </RunConfigPane>
      <ScrollEdges /></Box>
    </DesktopModal>
  );
}
