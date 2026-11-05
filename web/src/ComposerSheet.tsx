import { desktopSize, glyphSize } from "./surface/desktopSize";
import { QueueDeliverySettings } from "./QueueDeliverySettings";
import { type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { alpha, Box, Button, ButtonBase, Chip, CircularProgress, Collapse, DialogContentText, Divider, IconButton, LinearProgress, List, MenuItem, Stack, Switch, TextField, Tooltip, Typography, useMediaQuery } from "@mui/material";
import { Check, CleaningServices, Close, Compress, ExpandMore, Refresh } from "@mui/icons-material";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { type SessionAction } from "./agentCommands";
import { ConfirmConsequence } from "./ConfirmConsequence";
import { currentProviderEntry, useProviderCatalog } from "./providerCatalog";
import { providerName } from "./providerPresentation";
import { SessionProviderAccess } from "./ProviderManagement";
import { SessionAccountDetails } from "./SessionAccountDetails";
import { resolveProviderAuthenticationPresentation } from "@cowboy/provider-ui";
import { sessionProviderFacts, sessionProviderManageLabel, sessionProviderNeedsAttention, sessionProviderShowsUsage, sessionProviderSummary, sessionProviderUsageEmptyMessage, sessionProviderUsageRows, workspaceOptionsSummary } from "./sessionSettingsPresentation";
import { providerUsage, providerUsageAccount, usageRefreshing } from "./usageLimits";
import { expectHttpOk } from "./httpResponse";
import { SessionReloadDialog } from "./SessionReloadDialog";
import { ProviderUpdateCard } from "./ProviderUpdateCard";
import { ProviderUpdateDialog } from "./ProviderUpdateDialog";
import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { SessionToolsSection } from "./AgentToolsPanel";
import { SessionContextSettings } from "./SessionContextSettings";
import { isManagedChild } from "./managedCalls";
import { type TranscriptProjection } from "./explore/exploreStore";
import { renameSession, setPaused, loadUsageSnapshot, requestUsageRefresh, useStoreSelector } from "./store";
import { haptic } from "./haptic";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { NetworkButton, NetworkIconButton, useNetworkActionState } from "./NetworkActionFeedback";
import type { ConfigOptionChange } from "./configOptionMutation";
import { originLabel } from "./protocol";
import { providerConfigOptionDisabled, providerConfigOptionPresentations } from "./providerConfigOptions";
import { activeRunConfigPreset, type RunConfigPreset, runConfigCurrentTitle, runConfigPresetChanges, runConfigPresets, runConfigSummary } from "./runConfigPresets";
import { usageCacheIntervalLabel, usageCacheMinHitTokens, usageCacheOptionName } from "./usageHostMap";
import type { ConfigOption, SessionMeta, Status } from "./protocol";
import { sessionProjectLabel } from "./sessionProject";
import { sessionExecution } from "./sessionExecution";
import { ConfirmSheet, Sheet } from "./Sheet";
import { MobileSheetDismiss } from "@cowboy/app-shell";
import { type SessionSettingsFocus } from "./sessionSettingsOpen";
import { ConfigRevertBack } from "./ConfigRevertBack";

export function SessionActionConfirmDialog({
  action,
  provider,
  activeTurn,
  disableRestoreFocus = false,
  onClose,
  onConfirm,
}: {
  action: SessionAction | null;
  provider: string;
  activeTurn: boolean;
  disableRestoreFocus?: boolean;
  onClose: () => void;
  onConfirm: () => Promise<void>;
}): React.JSX.Element {
  void disableRestoreFocus;
  return (
    <ConfirmSheet
      open={action !== null}
      onClose={onClose}
      title={action !== null ? `${action.label}?` : undefined}
      actions={action === null ? undefined : (
        <>
          <Button
            color="inherit"
            onClick={onClose}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <NetworkButton
            variant="contained"
            color={action.destructive ? "error" : "primary"}
            networkAction={onConfirm}
            sx={{ textTransform: "none" }}
          >
            {action.destructive ? "Clear" : "Compact"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </NetworkButton>
        </>
      )}
    >
      {action !== null && (
        <>
          <DialogContentText>{action.detail}</DialogContentText>
          <ConfirmConsequence
            tone={action.destructive ? "error" : "primary"}
            irreversible={action.destructive}
          >
            {action.kind === "slash" && action.command !== undefined
              ? (
                <>
                  Sends{" "}
                  <Box
                    component="code"
                    sx={{
                      fontFamily: "ui-monospace, monospace",
                      px: 0.5,
                      py: 0.125,
                      borderRadius: 0.75,
                      bgcolor: "action.hover",
                    }}
                  >
                    {action.command}
                  </Box>{" "}
                  to {provider || "the agent"}
                  {activeTurn ? " (queued — the agent is mid-turn)" : ""}.
                </>
              )
              : `Resets ${provider || "the agent"} to a fresh context now${
                activeTurn ? " (ends the current turn)" : ""
              }.`}
          </ConfirmConsequence>
        </>
      )}
    </ConfirmSheet>
  );
}

export function RecommendedRunConfigPresetButton({
  preset,
  selected,
  disabled,
  pending,
  progress,
  onActivate,
}: {
  preset: RunConfigPreset;
  selected: boolean;
  disabled: boolean;
  pending: boolean;
  progress: boolean;
  onActivate: () => void;
}): React.JSX.Element {
  const activateTap = useReliableTouchTap<HTMLButtonElement>(onActivate);

  return (
    <ButtonBase
      {...activateTap}
      disabled={disabled}
      aria-pressed={selected}
      aria-busy={pending || undefined}
      data-run-config-preset={preset.id}
      sx={{
        minHeight: 58,
        px: 1.5,
        py: 1,
        // Match the theme's 10px control radius used by the other actions in
        // this settings sheet.
        borderRadius: 1,
        border: 1,
        borderColor: selected ? "primary.main" : "divider",
        bgcolor: (theme) =>
          selected
            ? alpha(theme.palette.primary.main, 0.13)
            : alpha(theme.palette.background.default, 0.34),
        textAlign: "left",
        justifyContent: "flex-start",
        touchAction: "manipulation",
        "&:active": { transform: "scale(0.99)" },
        "&.Mui-disabled": { opacity: 0.46 },
      }}
    >
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={0.75} alignItems="center">
          <Typography variant="body2" sx={{ fontWeight: 750 }}>
            {preset.name}
          </Typography>
          {preset.isDefault && (
            <Chip
              label="Default preset"
              size="small"
              color="primary"
              variant="outlined"
              sx={{ height: 20, fontSize: "0.625rem" }}
            />
          )}
        </Stack>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: "block", mt: 0.2 }}
        >
          {preset.detail}
        </Typography>
      </Box>
      <Box
        aria-hidden
        sx={{
          width: 18,
          height: 18,
          display: "grid",
          placeItems: "center",
          flexShrink: 0,
        }}
      >
        {progress ? (
          <CircularProgress size={desktopSize(14)} thickness={4.5} color="inherit" />
        ) : selected ? <Check sx={{ fontSize: glyphSize(18) }} /> : null}
      </Box>
    </ButtonBase>
  );
}

// Unified bottom sheet for touch viewports: every action lives here, none
// in a visible inline row. ChatGPT / DeepSeek / Gemini all collapse their
// composer controls behind a single `+` because chip rows wrap awkwardly
// on iPad portrait (820px) and break entirely on a 390px iPhone, while
// the bottom-sheet pattern is iOS-native muscle memory.
export function ComposerSheet({
  open,
  focus = "session",
  onClose,
  session,
  options,
  loading,
  status,
  compacting,
  compactAction,
  clearAction,
  onSessionAction,
  projection,
  onProjectionChange,
  onApplyPreset,
  onSelectOption,
}: {
  open: boolean;
  focus?: SessionSettingsFocus;
  onClose: () => void;
  session: SessionMeta | undefined;
  options: ConfigOption[];
  loading: boolean;
  status: Status;
  compacting: boolean;
  compactAction: SessionAction | null;
  clearAction: SessionAction | null;
  onSessionAction: (action: SessionAction) => Promise<void>;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
  onApplyPreset: (changes: readonly ConfigOptionChange[]) => Promise<void>;
  onSelectOption: (configId: string, value: string | boolean) => boolean;
}): React.JSX.Element {
  // Phones and portrait touch tablets keep the bottom sheet. Pointer-driven
  // devices from 768px and every viewport >=1024px get a centered dialog.
  const useSheetSurface = useMediaQuery(
    "(max-width: 767.95px), (min-width: 768px) and (max-width: 1023.95px) and (pointer: coarse)",
  );
  const recommendedPresets = runConfigPresets(
    session?.provider,
    options,
    session?.provider_version,
    session?.provider_generation_digest,
  );
  const optionPresentations = useMemo(
    () =>
      providerConfigOptionPresentations(
        session?.provider,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_generation_digest,
      session?.provider_version,
    ],
  );
  const authoritativeActivePreset = activeRunConfigPreset(
    recommendedPresets,
    options,
  );
  const [pendingPresetId, setPendingPresetId] = useState<string | null>(null);
  const presetAction = useNetworkActionState();
  const pendingPreset = pendingPresetId === null
    ? undefined
    : recommendedPresets.find((preset) => preset.id === pendingPresetId);
  const currentConfigSummary = runConfigSummary(options);
  const currentConfigTitle = runConfigCurrentTitle(
    authoritativeActivePreset?.name,
  );
  const [customizeAgent, setCustomizeAgent] = useState(false);
  const agentDetailsId = useId();
  const [sessionActionsExpanded, setSessionActionsExpanded] = useState(false);
  const [workspaceOptionsExpanded, setWorkspaceOptionsExpanded] = useState(
    false,
  );
  const [cmdConfirm, setCmdConfirm] = useState<SessionAction | null>(null);
  const [reloadConfirm, setReloadConfirm] = useState(false);
  const [updateConfirm, setUpdateConfirm] = useState(false);
  const closeUpdateConfirm = useCallback(() => setUpdateConfirm(false), []);
  useEffect(() => {
    if (open) {
      setUpdateConfirm(false);
      setCustomizeAgent(false);
      setPendingPresetId(null);
      setSessionActionsExpanded(false);
      setWorkspaceOptionsExpanded(false);
      setCmdConfirm(null);
      setReloadConfirm(false);
    }
  }, [open, session?.id]);
  useEffect(() => {
    if (!open || focus !== "agent") return undefined;
    const frame = globalThis.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-session-settings-agent]")
        ?.scrollIntoView({ block: "start", behavior: "smooth" });
    });
    return () => globalThis.cancelAnimationFrame(frame);
  }, [focus, open, session?.id]);
  const showAgentDetails = recommendedPresets.length === 0 || customizeAgent;
  const displayTitle = session?.title.startsWith(`${session.provider} · `)
    ? session.title.slice(session.provider.length + 3)
    : session?.title ?? "";
  const [title, setTitle] = useState(displayTitle);
  const [titleFocused, setTitleFocused] = useState(false);
  const titleInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    setTitle(displayTitle);
    setTitleFocused(false);
  }, [displayTitle, open]);
  const trimmedTitle = title.trim();
  const titleDirty = trimmedTitle !== displayTitle;
  const saveTitle = (): void => {
    if (session && trimmedTitle && titleDirty) {
      renameSession(session.id, trimmedTitle);
    } else if (!trimmedTitle) {
      setTitle(displayTitle);
    }
    titleInputRef.current?.blur();
    setTitleFocused(false);
  };
  const saveTitleAndClose = (): void => {
    if (!trimmedTitle) return;
    saveTitle();
    setSessionActionsExpanded(false);
    setWorkspaceOptionsExpanded(false);
    setCmdConfirm(null);
    setReloadConfirm(false);
    onClose();
  };
  const cancelTitle = (): void => {
    setTitle(displayTitle);
    titleInputRef.current?.blur();
    setTitleFocused(false);
  };
  const close = (): void => {
    setTitle(displayTitle);
    setTitleFocused(false);
    setSessionActionsExpanded(false);
    setWorkspaceOptionsExpanded(false);
    setCmdConfirm(null);
    onClose();
  };
  const confirmSessionAction = async (): Promise<void> => {
    if (cmdConfirm === null) return;
    const action = cmdConfirm;
    setCmdConfirm(null);
    await onSessionAction(action);
  };
  useConfirmEnter(cmdConfirm !== null, () => {
    void confirmSessionAction();
  });
  const editingTitle = titleFocused || titleDirty;
  return (
    <>
      <Sheet
      open={open}
      onClose={close}
      forceSheet={useSheetSurface}
      cover={useSheetSurface}
      wide
      mobileDismiss={useSheetSurface ? "none" : "header"}
      floatingActions={useSheetSurface ? !editingTitle : true}
      actions={useSheetSurface
        ? (
          editingTitle
            ? (
              <MobileDecisionActions
                shelf
                onCancel={cancelTitle}
                confirmLabel="Save"
                onConfirm={saveTitleAndClose}
                confirmDisabled={!trimmedTitle}
                preserveFocus
              />
            )
            : <MobileSheetDismiss onClose={close} />
        )
        : undefined}
      title={useSheetSurface ? undefined : (
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
        >
          <Typography variant="h6" component="span" sx={{ fontWeight: 700 }}>
            Session settings
          </Typography>
          <Box sx={{ position: "relative", display: "inline-flex" }}>
            <IconButton
              aria-label="close session settings"
              onClick={close}
              size="small"
            >
              <Close fontSize="small" />
            </IconButton>
            <Kbd keys="Esc" floating />
          </Box>
        </Stack>
      )}
    >
      {(loading || options.length > 0) && (
        <>
          <Box data-session-settings-agent sx={{ py: 1.5 }}>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
            >
              Agent
            </Typography>
            {status === "exited" && (
              <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                Changes are saved for your next message. The session stays asleep.
              </Typography>
            )}
            {session && <ConfigRevertBack sessionId={session.id} options={options} />}
            {loading
              ? (
                <Stack
                  direction="row"
                  spacing={1.5}
                  alignItems="center"
                  sx={{ py: 1, color: "text.secondary" }}
                >
                  <CircularProgress size={desktopSize(16)} />
                  <Typography variant="body2">
                    Loading agent options…
                  </Typography>
                </Stack>
              )
              : (
                // Selecting a value does NOT close the sheet — mode, model, and
                // effort are commonly changed together, and each <Select> already
                // closes its own menu on pick. The user dismisses the sheet by
                // tapping outside once they're done.
                <Stack spacing={1.25} sx={{ mt: 1.25 }}>
                  {recommendedPresets.length > 0
                    ? (
                      <ButtonBase
                        aria-label={showAgentDetails
                          ? "Collapse current agent configuration"
                          : "Expand current agent configuration"}
                        aria-expanded={showAgentDetails}
                        aria-controls={agentDetailsId}
                        onClick={(): void => {
                          haptic();
                          setCustomizeAgent((value) => !value);
                        }}
                        sx={{
                          width: "100%",
                          minHeight: 44,
                          px: 0.5,
                          borderRadius: 1.5,
                          justifyContent: "space-between",
                          color: "text.secondary",
                          textAlign: "left",
                          "&:active": { bgcolor: "action.hover" },
                        }}
                      >
                        <Box sx={{ minWidth: 0, pr: 1 }}>
                          <Typography
                            variant="body2"
                            sx={{ fontWeight: 650 }}
                          >
                            {currentConfigTitle}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                            sx={{ display: "block", lineHeight: 1.35 }}
                          >
                            {pendingPreset
                              ? `Applying ${pendingPreset.name}…`
                              : currentConfigSummary}
                          </Typography>
                        </Box>
                        <ExpandMore
                          fontSize="small"
                          sx={{
                            flexShrink: 0,
                            transform: showAgentDetails
                              ? "rotate(180deg)"
                              : "none",
                            transition: (theme) =>
                              theme.transitions.create("transform"),
                          }}
                        />
                      </ButtonBase>
                    )
                    : (
                      <Box>
                        <Typography
                          variant="body2"
                          sx={{ fontWeight: 650 }}
                        >
                          {currentConfigTitle}
                        </Typography>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ display: "block", lineHeight: 1.35 }}
                        >
                          {pendingPreset
                            ? `Applying ${pendingPreset.name}…`
                            : currentConfigSummary}
                        </Typography>
                      </Box>
                    )}
                  <Collapse
                    in={showAgentDetails}
                    id={agentDetailsId}
                    unmountOnExit={recommendedPresets.length > 0}
                  >
                    <Stack
                      spacing={2}
                      sx={{ pt: recommendedPresets.length > 0 ? 0.5 : 0 }}
                    >
                      {options.map((opt) => (
                        <ConfigSheetDropdown
                          key={opt.id}
                          option={opt}
                          disabled={providerConfigOptionDisabled(
                            status,
                            optionPresentations.get(opt.id),
                          )}
                          onSelect={(value): void => {
                            onSelectOption(opt.id, value);
                          }}
                        />
                      ))}
                    </Stack>
                  </Collapse>
                  {recommendedPresets.length > 0 && (
                    <>
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{ fontWeight: 700 }}
                      >
                        Recommended
                      </Typography>
                      <Box
                        sx={{
                          display: "grid",
                          gridTemplateColumns: {
                            xs: "1fr",
                            sm: "repeat(2, minmax(0, 1fr))",
                          },
                          gap: 1,
                        }}
                      >
                        {recommendedPresets.map((preset) => (
                          <RecommendedRunConfigPresetButton
                            key={preset.id}
                            preset={preset}
                            selected={authoritativeActivePreset?.id ===
                              preset.id}
                            disabled={presetAction.pending}
                            pending={pendingPresetId === preset.id}
                            progress={pendingPresetId === preset.id &&
                              presetAction.progress}
                            onActivate={(): void => {
                              const changes = runConfigPresetChanges(
                                preset,
                                options,
                              );
                              if (changes.length === 0) {
                                setPendingPresetId(null);
                                setCustomizeAgent(false);
                                return;
                              }
                              setPendingPresetId(preset.id);
                              haptic();
                              void presetAction.run(() =>
                                onApplyPreset(changes)
                              ).then((succeeded) => {
                                setPendingPresetId(null);
                                if (succeeded) setCustomizeAgent(false);
                              });
                            }}
                          />
                        ))}
                      </Box>
                    </>
                  )}
                </Stack>
              )}
          </Box>
        </>
      )}
      {session && !isManagedChild(session) && (
        <>
          {/* Right under the Agent block: model and reasoning apply in place;
              another subscription or Provider is a Switch with Back. */}
          <SessionContextSettings session={session} />
          <Divider />
          <SessionToolsSection sessionId={session.id} />
        </>
      )}
      {session && (
        <>
          <Divider />
          <SessionInfoSection
            session={session}
            title={title}
            titleInputRef={titleInputRef}
            onTitleChange={setTitle}
            onTitleFocusChange={setTitleFocused}
            onTitleSave={saveTitle}
            saveOnBlur={!useSheetSurface}
            compactAction={compactAction}
            clearAction={clearAction}
            compacting={compacting}
            actionsExpanded={sessionActionsExpanded}
            onActionsExpandedChange={setSessionActionsExpanded}
            onSessionAction={setCmdConfirm}
            onReload={(): void => setReloadConfirm(true)}
            onProviderUpdate={(): void => setUpdateConfirm(true)}
          />
          <WorkspaceOptionsSection
            session={session}
            expanded={workspaceOptionsExpanded}
            onExpandedChange={setWorkspaceOptionsExpanded}
            projection={projection}
            onProjectionChange={onProjectionChange}
          />
          <SessionProviderSection session={session} />
        </>
      )}
      <SessionActionConfirmDialog
        action={cmdConfirm}
        provider={providerName(
          session?.provider ?? "",
          session?.provider_version,
          session?.provider_generation_digest,
        )}
        activeTurn={session?.status === "busy" ||
          session?.status === "starting"}
        onClose={(): void => setCmdConfirm(null)}
        onConfirm={confirmSessionAction}
      />
      <SessionReloadDialog
        session={open && reloadConfirm ? session : null}
        onClose={(): void => setReloadConfirm(false)}
      />
      <ProviderUpdateDialog
        session={open && updateConfirm ? session : null}
        onClose={closeUpdateConfirm}
      />
      </Sheet>
    </>
  );
}

// Read-only session metadata at the top of the options sheet. This used to
// live behind a long-press on the mobile title bar (a gesture nobody found),
// so it now rides the one popup the user already opens to change mode / model
// / effort. Desktop shows the same facts in the persistent sidebar, so this
// section only renders inside the compact-tier sheet.
export interface DeepseekCacheProtectionStatus {
  state: "protected" | "inactive" | "disabled";
  protected_tokens?: number;
  last_verified_at_ms?: number;
  next_attempt_at_ms?: number;
  scheduled_interval_ms?: number;
  base_interval_ms?: number;
  adaptive_interval_ms?: number;
  minimum_interval_ms?: number;
  maximum_interval_ms?: number;
  jitter_ms?: number;
  expires_at_ms?: number;
  attempts?: number;
  minimumHitTokens?: number;
  contextUsed?: number;
}

export function compactCacheTokens(tokens: number | undefined): string {
  if (tokens === undefined || !Number.isFinite(tokens) || tokens < 0) return "";
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000).toLocaleString()}K`;
  return tokens.toLocaleString();
}

export function compactCacheDuration(milliseconds: number | undefined): string {
  if (
    milliseconds === undefined || !Number.isFinite(milliseconds) ||
    milliseconds <= 0
  ) {
    return "";
  }
  const totalMinutes = Math.max(1, Math.round(milliseconds / 60_000));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) return `${minutes}m`;
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

export function SessionInfoSection({
  session,
  title,
  titleInputRef,
  onTitleChange,
  onTitleFocusChange,
  onTitleSave,
  saveOnBlur,
  compactAction,
  clearAction,
  compacting,
  actionsExpanded,
  onActionsExpandedChange,
  onSessionAction,
  onReload,
  onProviderUpdate,
}: {
  session: SessionMeta;
  title: string;
  titleInputRef: React.RefObject<HTMLInputElement | null>;
  onTitleChange: (title: string) => void;
  onTitleFocusChange: (focused: boolean) => void;
  onTitleSave: () => void;
  saveOnBlur: boolean;
  compactAction: SessionAction | null;
  clearAction: SessionAction | null;
  compacting: boolean;
  actionsExpanded: boolean;
  onActionsExpandedChange: (expanded: boolean) => void;
  onSessionAction: (action: SessionAction) => void;
  onReload: () => void;
  onProviderUpdate: () => void;
}): React.JSX.Element {
  // Title is editable right here — this sheet already shows the session's identity,
  // so the rename (edit-title) belongs with it rather than off in app Settings.
  // Strip the auto "provider · " prefix like the navbar does, so you edit the
  // DISPLAY title, not the machine string. The parent owns the draft because
  // mobile replaces the footer Close action with an explicit Save action while
  // this field is being edited. Plain Enter is intentionally not a commit path.
  const project = sessionProjectLabel(session) ?? "Not recorded";
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const execution = sessionExecution(session);
  const machineName = (id: string | undefined): string =>
    machines.find((machine) => machine.id === id)?.display_name ?? id ?? "Local";
  const contextUsed = session.context_used ?? 0;
  const contextSize = session.context_size ?? 0;
  const hasContext = contextSize > 0;
  const contextPercent = hasContext
    ? Math.min(100, Math.max(0, contextUsed / contextSize * 100))
    : 0;
  const providerEntry = currentProviderEntry(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const usageAccount = providerEntry?.manifest.host.account_usage?.provider ??
    session.provider;
  const cacheOptionName = usageCacheOptionName(usageAccount);
  const cacheProtectionVisible =
    providerEntry?.manifest.host.features.includes("cache_protection_v1") ===
      true &&
    contextUsed >= usageCacheMinHitTokens(usageAccount);
  const sessionProviderName = providerName(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const [cacheProtection, setCacheProtection] = useState<
    DeepseekCacheProtectionStatus | null
  >(null);
  const [cacheProtectionUnavailable, setCacheProtectionUnavailable] = useState(
    false,
  );
  useEffect(() => {
    if (!cacheProtectionVisible) {
      setCacheProtection(null);
      setCacheProtectionUnavailable(false);
      return;
    }
    const controller = new AbortController();
    const refresh = (): void => {
      void fetch(
        `/api/sessions/${encodeURIComponent(session.id)}/cache-protection`,
        { signal: controller.signal },
      ).then(async (response) => {
        await expectHttpOk(response, "Could not load cache protection status");
        const value = await response.json() as DeepseekCacheProtectionStatus;
        if (!["protected", "inactive", "disabled"].includes(value.state)) {
          throw new Error("invalid cache status");
        }
        setCacheProtection(value);
        setCacheProtectionUnavailable(false);
      }).catch(() => {
        if (!controller.signal.aborted) {
          setCacheProtection(null);
          setCacheProtectionUnavailable(true);
        }
      });
    };
    refresh();
    const interval = globalThis.setInterval(refresh, 60_000);
    return (): void => {
      controller.abort();
      globalThis.clearInterval(interval);
    };
  }, [cacheProtectionVisible, session.id]);
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: "Project", value: project },
    { label: "Working dir", value: execution.cwd, mono: true },
    ...(execution.state === "local" ? [] : [
      { label: "Agent runtime", value: machineName(session.machine_id) },
      { label: "Environment", value: execution.machineId ? machineName(execution.machineId) : "Unavailable" },
    ]),
    { label: "Source", value: originLabel(session.origin) },
    { label: "Status", value: session.status },
    { label: "Session id", value: session.id, mono: true },
  ];
  const cacheBaseInterval =
    compactCacheDuration(cacheProtection?.base_interval_ms) ||
    usageCacheIntervalLabel(usageAccount);
  const cacheAdaptiveInterval = compactCacheDuration(
    cacheProtection?.adaptive_interval_ms,
  );
  const cacheScheduledInterval = compactCacheDuration(
    cacheProtection?.scheduled_interval_ms,
  );
  const actionsPanelId = useId();
  const cacheIntervalDetail = cacheProtection?.state === "protected"
    ? `Base ${cacheBaseInterval} · next ${
      cacheScheduledInterval || cacheAdaptiveInterval || "learning"
    }`
    : `Base ${cacheBaseInterval} · adaptive target ${
      cacheAdaptiveInterval || "learning"
    }`;
  return (
    <>
      <Box sx={{ pt: 0.5, pb: 0.25 }}>
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
        >
          Session
        </Typography>
      </Box>
      <ProviderUpdateCard session={session} onUpdate={onProviderUpdate} />
      <TextField
        inputRef={titleInputRef}
        name="session-title"
        size="small"
        label="Title"
        value={title}
        onChange={(e): void => onTitleChange(e.target.value)}
        onFocus={(e): void => {
          onTitleFocusChange(true);
          // Select the whole title on focus, so tapping it to rename lets you
          // replace it in one go (instead of fiddling a caret into a long string).
          // Deferred a frame — iOS collapses a synchronous select() back to a caret
          // as it finishes installing focus/the keyboard.
          const input = e.target as HTMLInputElement;
          requestAnimationFrame(() => input.select());
        }}
        onBlur={(): void => {
          onTitleFocusChange(false);
          if (saveOnBlur) onTitleSave();
        }}
        onKeyDown={(event): void => {
          // Desktop retains its compact field convention. Mobile Enter belongs
          // to the system keyboard and never submits the rename.
          if (
            saveOnBlur && event.key === "Enter" &&
            !isImeKeyEvent(event.nativeEvent)
          ) {
            (event.target as HTMLInputElement).blur();
          }
        }}
        fullWidth
        sx={{ mt: 1, mb: 0.5 }}
      />
      <List dense disablePadding>
        {rows.map((r) => (
          <SheetDetailRow
            key={r.label}
            label={r.label}
            value={r.value}
            mono={r.mono === true}
          />
        ))}
      </List>
      <Stack spacing={0.65} sx={{ pt: 0.75, pb: 1 }}>
        <Stack
          direction="row"
          alignItems="baseline"
          justifyContent="space-between"
          spacing={2}
        >
          <Typography variant="caption" color="text.secondary">
            Context
          </Typography>
          <Typography variant="body2" sx={{ fontWeight: 600 }}>
            {hasContext
              ? `${Math.round(contextPercent)}% used`
              : "Waiting for usage"}
          </Typography>
        </Stack>
        <LinearProgress
          variant="determinate"
          value={contextPercent}
          color={contextPercent >= 90
            ? "error"
            : contextPercent >= 75
            ? "warning"
            : "primary"}
          aria-label="session context usage"
          sx={{
            height: 7,
            borderRadius: 99,
            bgcolor: "action.selected",
            "& .MuiLinearProgress-bar": { borderRadius: 99 },
          }}
        />
        {hasContext && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ textAlign: "right" }}
          >
            {contextUsed.toLocaleString()} / {contextSize.toLocaleString()}{" "}
            tokens
          </Typography>
        )}
        {cacheProtectionVisible &&
          (cacheProtection || cacheProtectionUnavailable) && (
          <Stack spacing={0.35} alignItems="flex-start">
            <Tooltip
              title={cacheProtection?.state === "protected"
                ? `${cacheIntervalDetail}. Last verified ${
                  cacheProtection.last_verified_at_ms
                    ? new Date(cacheProtection.last_verified_at_ms)
                      .toLocaleString()
                    : "recently"
                }. Next attempt ${
                  cacheProtection.next_attempt_at_ms
                    ? new Date(cacheProtection.next_attempt_at_ms)
                      .toLocaleString()
                    : "is being scheduled"
                }. Real agent work always preempts it.`
                : cacheProtection?.state === "disabled"
                ? `Automatic ${sessionProviderName} cache protection is disabled for this session. Base interval is ${cacheBaseInterval}.`
                : cacheProtectionUnavailable
                ? "The owning Machine could not report cache status. Agent work is unaffected."
                : "This session is large enough for protection; Cowboy is waiting for a verified ≥90% cache hit before scheduling a keepalive."}
            >
              <Chip
                size="small"
                variant="outlined"
                color={cacheProtection?.state === "protected"
                  ? "success"
                  : cacheProtectionUnavailable
                  ? "warning"
                  : "default"}
                label={cacheProtection?.state === "protected"
                  ? `Cache protected · ${
                    compactCacheTokens(cacheProtection.protected_tokens)
                  }`
                  : cacheProtection?.state === "disabled"
                  ? `${cacheOptionName} off`
                  : cacheProtectionUnavailable
                  ? "Cache status unavailable"
                  : `${cacheOptionName} learning`}
                sx={{ alignSelf: "flex-start" }}
              />
            </Tooltip>
            {cacheProtection && !cacheProtectionUnavailable && (
              <Typography variant="caption" color="text.secondary">
                {cacheIntervalDetail} · adaptive learning
              </Typography>
            )}
          </Stack>
        )}
        <Box sx={{ pt: 0.5 }}>
          <ButtonBase
            aria-label={actionsExpanded
              ? "Collapse session actions"
              : "Expand session actions"}
            aria-expanded={actionsExpanded}
            aria-controls={actionsPanelId}
            onClick={(): void => {
              haptic();
              onActionsExpandedChange(!actionsExpanded);
            }}
            sx={{
              width: "100%",
              minHeight: 44,
              px: 0.5,
              borderRadius: 1.5,
              justifyContent: "space-between",
              color: "text.secondary",
              textAlign: "left",
              "&:active": { bgcolor: "action.hover" },
            }}
          >
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Session actions
            </Typography>
            <ExpandMore
              fontSize="small"
              sx={{
                transform: actionsExpanded ? "rotate(180deg)" : "none",
                transition: (theme) => theme.transitions.create("transform"),
              }}
            />
          </ButtonBase>
          <Collapse id={actionsPanelId} in={actionsExpanded} unmountOnExit>
            <Stack spacing={1} sx={{ pt: 0.75 }}>
              <Button
                fullWidth
                variant="outlined"
                startIcon={<Refresh />}
                aria-label="reload session from session settings"
                onClick={(event): void => {
                  event.currentTarget.blur();
                  onReload();
                }}
                sx={{ minHeight: 44, textTransform: "none" }}
              >
                Reload runtime
              </Button>
              {compactAction && (
                <Button
                  fullWidth
                  variant="outlined"
                  startIcon={<Compress />}
                  disabled={compacting}
                  aria-label="compact conversation from session settings"
                  onClick={(event): void => {
                    event.currentTarget.blur();
                    onSessionAction(compactAction);
                  }}
                  sx={{ minHeight: 44, textTransform: "none" }}
                >
                  {compacting ? "Compacting…" : "Compact"}
                </Button>
              )}
              {clearAction && (
                <Button
                  fullWidth
                  variant="outlined"
                  color="error"
                  startIcon={<CleaningServices />}
                  aria-label="clear conversation from session settings"
                  onClick={(event): void => {
                    event.currentTarget.blur();
                    onSessionAction(clearAction);
                  }}
                  sx={{ minHeight: 44, textTransform: "none" }}
                >
                  Clear
                </Button>
              )}
            </Stack>
          </Collapse>
        </Box>
      </Stack>
    </>
  );
}

export function SessionProviderSection({
  session,
}: {
  session: SessionMeta;
}): React.JSX.Element {
  return <>
    <SessionAccountDetails session={session} />
    {(!session.provider_account_id || session.provider_account_id === "default") &&
      <ServiceSessionProviderSection session={session} />}
  </>;
}

export function ServiceSessionProviderSection({ session }: { session: SessionMeta }): React.JSX.Element {
  const { catalog } = useProviderCatalog();
  const entry = currentProviderEntry(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const auth = catalog?.authentications.find((status) =>
    status.provider_id === session.provider ||
    status.authentication_scope === entry?.authentication_scope
  );
  const required = entry?.manifest.authentication.required === true;
  const ready = !required || auth?.authentication_state === "ready";
  const needsAttention = sessionProviderNeedsAttention({
    catalogReady: catalog !== null,
    required,
    ready,
  });
  const [expanded, setExpanded] = useState(true);
  const [actionsOpen, setActionsOpen] = useState(needsAttention);
  useEffect(() => {
    if (needsAttention) setExpanded(true);
    setActionsOpen(needsAttention);
  }, [needsAttention, session.id]);
  const presentation = entry
    ? resolveProviderAuthenticationPresentation(entry.manifest.authentication)
    : "none";
  const facts = entry
    ? sessionProviderFacts({
      vendor: entry.manifest.display.vendor,
      version: entry.provider_version,
      ...(ready && auth?.account_label
        ? { accountLabel: auth.account_label }
        : {}),
    })
    : [];
  const manageLabel = sessionProviderManageLabel(
    presentation === "none" ? "none" : presentation,
  );
  const summary = sessionProviderSummary({
    displayName: providerName(
      session.provider,
      session.provider_version,
      session.provider_generation_digest,
    ),
    required,
    ready,
    ...(auth?.account_label ? { accountLabel: auth.account_label } : {}),
    presentation: presentation === "none" ? "none" : presentation,
  });
  const panelId = useId();
  return (
    <>
      <Divider />
      <Box sx={{ py: 0.75 }}>
        <ButtonBase
          aria-label={expanded
            ? "Collapse provider"
            : "Expand provider"}
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={(): void => {
            haptic();
            setExpanded((current) => !current);
          }}
          sx={{
            width: "100%",
            minHeight: 44,
            px: 0.5,
            borderRadius: 1.5,
            justifyContent: "space-between",
            color: "text.secondary",
            textAlign: "left",
            "&:active": { bgcolor: "action.hover" },
          }}
        >
          <Box sx={{ minWidth: 0, pr: 1 }}>
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Provider
            </Typography>
            <Typography
              variant="caption"
              color={needsAttention ? "warning.main" : "text.secondary"}
              sx={{ display: "block", lineHeight: 1.35 }}
            >
              {summary}
            </Typography>
          </Box>
          <ExpandMore
            fontSize="small"
            sx={{
              flexShrink: 0,
              transform: expanded ? "rotate(180deg)" : "none",
              transition: (theme) => theme.transitions.create("transform"),
            }}
          />
        </ButtonBase>
        <Collapse id={panelId} in={expanded}>
          <Box sx={{ pt: 0.5 }}>
            {entry && (
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{
                  display: "block",
                  px: 0.5,
                  pb: 0.25,
                  lineHeight: 1.4,
                }}
              >
                {entry.manifest.display.summary}
              </Typography>
            )}
            {entry && (
              <List dense disablePadding>
                {facts.map((row) => (
                  <SheetDetailRow
                    key={row.label}
                    label={row.label}
                    value={row.value}
                    mono={row.mono === true}
                  />
                ))}
                {sessionProviderShowsUsage({
                  ready,
                  ...(entry.manifest.host.account_usage?.provider
                    ? {
                      accountUsageProvider:
                        entry.manifest.host.account_usage.provider,
                    }
                    : {}),
                }) && (
                  <SessionProviderUsage
                    provider={session.provider}
                    {...(session.provider_version === undefined
                      ? {}
                      : { providerVersion: session.provider_version })}
                    {...(session.provider_generation_digest === undefined
                      ? {}
                      : {
                        providerDigest: session.provider_generation_digest,
                      })}
                  />
                )}
              </List>
            )}
            {ready && (
              <ButtonBase
                aria-label={actionsOpen
                  ? `Collapse ${manageLabel.toLowerCase()}`
                  : `Expand ${manageLabel.toLowerCase()}`}
                aria-expanded={actionsOpen}
                onClick={(): void => {
                  haptic();
                  setActionsOpen((current) => !current);
                }}
                sx={{
                  width: "100%",
                  minHeight: 44,
                  px: 0.5,
                  mt: 0.25,
                  borderRadius: 1.5,
                  justifyContent: "space-between",
                  color: "text.secondary",
                  textAlign: "left",
                  "&:active": { bgcolor: "action.hover" },
                }}
              >
                <Typography variant="body2" sx={{ fontWeight: 650 }}>
                  {manageLabel}
                </Typography>
                <ExpandMore
                  fontSize="small"
                  sx={{
                    flexShrink: 0,
                    transform: actionsOpen ? "rotate(180deg)" : "none",
                    transition: (theme) =>
                      theme.transitions.create("transform"),
                  }}
                />
              </ButtonBase>
            )}
            <Collapse in={!ready || actionsOpen} unmountOnExit>
              <Box sx={{ pt: 0.5 }}>
                <SessionProviderAccess providerId={session.provider} />
              </Box>
            </Collapse>
          </Box>
        </Collapse>
      </Box>
    </>
  );
}

// Queue pause and Page view are infrequent. Keep them behind one disclosure
// so the sheet opens on identity, Provider, and Agent. Do not call this
// Workspace — that word already means a project checkout. A paused queue
// still names itself on the collapsed row.
export function WorkspaceOptionsSection({
  session,
  expanded,
  onExpandedChange,
  projection,
  onProjectionChange,
}: {
  session: SessionMeta;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
}): React.JSX.Element {
  const paused = session.paused ?? false;
  const pageView = projection === "explore";
  const panelId = useId();
  return (
    <>
      <Divider />
      <Box sx={{ py: 0.75 }}>
        <ButtonBase
          aria-label={expanded
            ? "Collapse queue and view"
            : "Expand queue and view"}
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={(): void => {
            haptic();
            onExpandedChange(!expanded);
          }}
          sx={{
            width: "100%",
            minHeight: 44,
            px: 0.5,
            borderRadius: 1.5,
            justifyContent: "space-between",
            color: paused ? "warning.main" : "text.secondary",
            textAlign: "left",
            "&:active": { bgcolor: "action.hover" },
          }}
        >
          <Box sx={{ minWidth: 0, pr: 1 }}>
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Queue & view
            </Typography>
            <Typography
              variant="caption"
              color={paused ? "warning.main" : "text.secondary"}
              sx={{ display: "block", lineHeight: 1.35 }}
            >
              {workspaceOptionsSummary({ queuePaused: paused, pageView })}
            </Typography>
          </Box>
          <ExpandMore
            fontSize="small"
            sx={{
              flexShrink: 0,
              transform: expanded ? "rotate(180deg)" : "none",
              transition: (theme) => theme.transitions.create("transform"),
            }}
          />
        </ButtonBase>
        <Collapse id={panelId} in={expanded} unmountOnExit>
          <Stack spacing={0.5} sx={{ pt: 0.5 }}>
            <QueueDeliverySettings sessionId={session.id} pauseControl={false} />
            <Stack
              direction="row"
              alignItems="center"
              spacing={2}
              sx={{ minHeight: 56 }}
            >
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Typography variant="body2">Pause queue</Typography>
                <Typography variant="caption" color="text.secondary">
                  Hold the queue: a running turn still finishes, but queued and
                  scheduled sends wait until you resume.
                </Typography>
              </Box>
              <Switch
                edge="end"
                checked={paused}
                color="warning"
                inputProps={{ "aria-label": "pause queue" }}
                onChange={(e): void => {
                  haptic();
                  setPaused(session.id, e.target.checked);
                }}
              />
            </Stack>
            {projection && onProjectionChange && (
              <Stack
                direction="row"
                alignItems="center"
                spacing={2}
                sx={{ minHeight: 56 }}
              >
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="body2" sx={{ fontWeight: 650 }}>
                    Page view
                  </Typography>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ display: "block", lineHeight: 1.35 }}
                  >
                    One prompt and response per page
                  </Typography>
                </Box>
                <Switch
                  inputProps={{ "aria-label": "Page view" }}
                  checked={pageView}
                  onChange={(_event, checked): void =>
                    onProjectionChange(checked ? "explore" : "history")}
                />
              </Stack>
            )}
          </Stack>
        </Collapse>
      </Box>
    </>
  );
}

export function SessionProviderUsage({
  provider,
  providerVersion,
  providerDigest,
}: {
  provider: string;
  providerVersion?: string;
  providerDigest?: string;
}): React.JSX.Element {
  // Controller-owned usage shared by every surface and device.
  const snapshot = useStoreSelector((state) => state.usage);
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    const timer = globalThis.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => globalThis.clearInterval(timer);
  }, []);
  useEffect(() => setClock(Date.now()), [snapshot]);
  const account = providerUsageAccount(provider, providerVersion, providerDigest);
  const refreshing = account !== undefined && usageRefreshing(snapshot, account);
  const load = useCallback(async (manual: boolean): Promise<void> => {
    if (manual) {
      if (!account) {
        throw new Error("Usage is unavailable for this session's Provider.");
      }
      await requestUsageRefresh(account);
    } else {
      await loadUsageSnapshot();
    }
    setError(null);
  }, [account]);
  useEffect(() => {
    const ctrl = new AbortController();
    void loadUsageSnapshot(ctrl.signal).catch((cause: unknown) => {
      if (ctrl.signal.aborted) return;
      setError(cause instanceof Error ? cause.message : "Could not load usage");
    });
    return (): void => ctrl.abort();
  }, [provider]);
  const usage = providerUsage(
    snapshot,
    provider,
    providerVersion,
    providerDigest,
  );
  const rows = sessionProviderUsageRows(usage, clock);
  const refresh = (
    <NetworkIconButton
      aria-label="Refresh provider usage"
      size="small"
      reliableTouch
      networkAction={() => load(true)}
      disabled={refreshing}
      sx={{
        width: "2.75em",
        height: "2.75em",
        p: 0,
        color: "text.secondary",
        "& .MuiSvgIcon-root": { fontSize: "1em" },
      }}
    >
      <Refresh />
    </NetworkIconButton>
  );
  if (error) {
    return <SheetDetailRow label="Usage" value={error} action={refresh} />;
  }
  if (!snapshot) {
    return <SheetDetailRow label="Usage" value="Loading…" />;
  }
  if (rows.length === 0) {
    return (
      <SheetDetailRow
        label="Usage"
        value={sessionProviderUsageEmptyMessage(usage)}
        action={refresh}
      />
    );
  }
  return (
    <>
      {rows.map((row) => (
        <Box key={row.id}>
          <SheetDetailRow
            label={row.label}
            value={row.value}
            action={row.id === "usage-updated" ? refresh : undefined}
          />
          {row.remaining !== undefined && (
            <LinearProgress
              variant="determinate"
              value={row.remaining}
              aria-label={`${row.label} remaining`}
              sx={{
                height: 5,
                mb: 0.75,
                borderRadius: 99,
                bgcolor: "action.selected",
                "& .MuiLinearProgress-bar": { borderRadius: 99 },
              }}
            />
          )}
        </Box>
      ))}
    </>
  );
}

export function SheetDetailRow({
  label,
  value,
  mono = false,
  action,
}: {
  label: string;
  value: string;
  mono?: boolean;
  action?: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        py: 0.75,
        display: "flex",
        gap: 2,
        alignItems: action ? "center" : "baseline",
      }}
    >
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ minWidth: 96, flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography
        variant="body2"
        sx={{
          flex: 1,
          minWidth: 0,
          wordBreak: "break-word",
          fontFamily: mono ? MONO : "inherit",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </Typography>
      {action}
    </Box>
  );
}

// One labelled dropdown per agent option (mode / model / effort). Collapses
// what used to be an always-expanded radio list — the sheet stays short even
// when an agent advertises a dozen models. ACP option values may be string OR
// boolean, so the <Select> is keyed on String(value) and mapped back to the
// original type on change.
export function ConfigSheetDropdown({
  option,
  disabled,
  onSelect,
}: {
  option: ConfigOption;
  disabled: boolean;
  onSelect: (value: string | boolean) => void;
}): React.JSX.Element {
  const currentKey = String(option.currentValue);
  return (
    <TextField
      select
      fullWidth
      size="small"
      disabled={disabled}
      label={option.name}
      value={currentKey}
      onChange={(e): void => {
        const picked = option.options.find(
          (o) => String(o.value) === e.target.value,
        );
        if (picked) onSelect(picked.value);
      }}
      {...(option.description ? { helperText: option.description } : {})}
    >
      {option.options.map((o) => (
        <MenuItem key={String(o.value)} value={String(o.value)}>
          {o.name}
        </MenuItem>
      ))}
    </TextField>
  );
}

export const MONO = "ui-monospace, SFMono-Regular, Menlo, monospace";
