import { desktopSize } from "./surface/desktopSize";
import { SegmentedTabs } from "./SegmentedTabs";
import { MachineProjects } from "./MachineProjects";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Alert, alpha, Box, Button, ButtonBase, Chip, CircularProgress, GlobalStyles, IconButton, MenuItem, Paper, Select, Stack, Switch, Tooltip, Typography, useMediaQuery } from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import { ArrowBackIosNew, ChatBubbleOutline, Check as CheckIcon, ChevronRight, Close as CloseIcon, Code as CodeIcon, DeleteOutline, ExpandLess, ExpandMore, Refresh as RefreshIcon, SystemUpdateAlt } from "@mui/icons-material";
import { type SettingsProductFocus } from "./appSettings";
import { ReviewSettingsContent } from "./mobile/review/ReviewSettings";
import { AppIconSettings } from "./AppIconSettings";
import { ClientUpdateSettings } from "./ClientUpdateSettings";
import { NotificationSettingsContent } from "./NotificationSettings";
import { AgentToolsSettings } from "./AgentToolsPanel";
import { KEYBOARD_INSET_CHANGED_EVENT } from "./keyboardInset";
import { machineConvergencePresentation, machineSupersessionPresentation, machineVersionPresentation } from "./machineVersions";
import { DelayedNetworkProgress, NetworkIconButton } from "./NetworkActionFeedback";
import { type MachineSummary } from "./protocol";
import { StatusDot } from "./SessionStatusDot";
import { useStoreSelector } from "./store";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { setVimConfigSource, setVimSetting, useVimConfigSource, useVimSetting } from "./vimSetting";
import { parseVimConfig, selectedEscapePreset, VIM_ESCAPE_PRESETS, withEscapePreset } from "./desktop/vim/vimConfig";
import { setComposerSourceMode, useComposerSourceMode } from "./composerSourceMode";
import { setComposerDebugSetting, useComposerDebugSetting } from "./composerDebugSetting";
import { reportComposerDebugModeChanged } from "./composer/composerInputDebug";
import { FONT_SCALE_PRESETS, LINE_HEIGHT_PRESETS, nearestPreset, PADDING_PRESETS, setFontScale, setFontVariant, setLineHeight, setPadding, useReadingSettings } from "./readingSettings";
import { FONT_PRESETS, getFontPreset } from "./fonts";
import { MachineProviderManagement, ProviderAuthenticationManagement } from "./ProviderManagement";
import { machinePresencePresentation } from "./machinePresence";
import { machineCommandResultPresentation } from "./machineCommandResult";
import { MobileSheetActionGroup } from "@cowboy/app-shell";
import { ConfirmSheet, Sheet } from "./Sheet";
import { Kbd, useConfirmEnter } from "./Kbd";
import { HintTooltip } from "./HintTooltip";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { InfoContent } from "./InfoSheet";
import { UsageLogs } from "./UsageLogs";
import { desktopFocusBoundary, type Mode as ThemeMode } from "./theme";
import { desktopImeOwnsKey } from "./desktop/commands/imeShortcut";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
import { sequentialShortcutAvailability } from "./desktop/commands/shortcutAvailability";
import { DesktopPanel } from "./desktop/DesktopPanel";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { adjacentControlCenterTab, controlCenterTabForShortcut, CONTROL_CENTER_TABS, type ControlCenterTab } from "./desktop/controlCenterTabs";
import { ProductAccountMenu } from "./auth/ProductAccountMenu";
import { ProductAccountSecurity } from "./auth/ProductAccountSecurity";
import { ProductSessionCapacityPanel } from "./auth/ProductSessionCapacityPanel";
import { ProductDevicesPanel } from "./auth/ProductDevicesPanel";
import { closestScrollableSettingsSurface, destinationScrollTop, nextSavedSettingsScroll, settingsFocusRevealDelta } from "./settingsDrillInScroll";
import { CONTROL_CENTER_PANEL_EXIT_MS, controlCenterViewTransitionStyles, controlCenterPanelMotionSx } from "./desktop/controlCenterMotion";

export type MachineChoice = MachineSummary;

export type SettingsTab = ControlCenterTab;

export type ControlCenterViewTransition = {
    finished: Promise<void>;
    ready: Promise<void>;
    skipTransition: () => void;
    updateCallbackDone: Promise<void>;
};

export type ControlCenterViewTransitionDocument = Document & {
    startViewTransition?: (
        update: () => void,
    ) => ControlCenterViewTransition;
};

export type ProviderSettingsTarget = {
    providerId: string;
    autoBeginAuthentication: boolean;
};

export type ProviderSettingsIntent = ProviderSettingsTarget & {
    requestId: number;
};

// Settings surface. Uses the shared BottomSheet (DetentSheet momentum sheet on
// mobile, centred Dialog on desktop) so it looks/behaves identically to every
// other app's settings + the portal's launcher. Touch is a preference list;
// the root page has no in-flow title because the sheet handle already marks
// the surface and the floating footer dismisses it.
export function DesktopSettingsChoice({
    active,
    children,
    onClick,
    ariaLabel,
}: {
    active: boolean;
    children: React.ReactNode;
    onClick: () => void;
    ariaLabel: string;
}): React.JSX.Element {
    return (
        <ButtonBase
            data-settings-choice
            aria-label={ariaLabel}
            aria-pressed={active}
            onClick={onClick}
            sx={{
                minHeight: desktopSize(38),
                minWidth: 0,
                width: "100%",
                px: 0.75,
                borderRadius: 1.25,
                border: 1,
                borderColor: active ? "primary.main" : "divider",
                bgcolor: active ? "action.selected" : "transparent",
                color: active ? "primary.main" : "text.primary",
                fontSize: "0.8125rem",
                fontWeight: active ? 700 : 500,
                justifyContent: "center",
                overflow: "hidden",
                transition: "background-color 120ms ease, border-color 120ms ease",
                "&:hover": { bgcolor: "action.hover", borderColor: "primary.light" },
                "&:focus-visible": {
                    outline: (theme) => `2px solid ${desktopFocusBoundary(theme)}`,
                    outlineOffset: 2,
                },
            }}
        >
            {children}
        </ButtonBase>
    );
}

export function DesktopSettingsRow({
    shortcut,
    shortcutAvailable = true,
    label,
    description,
    children,
}: {
    shortcut: string;
    shortcutAvailable?: boolean;
    label: string;
    description: string;
    children: React.ReactNode;
}): React.JSX.Element {
    return (
        <Box
            data-settings-row
            data-settings-shortcut={shortcut.toLowerCase()}
            sx={{
                display: "grid",
                gridTemplateColumns: "minmax(116px, 0.55fr) minmax(0, 1.45fr)",
                gap: 1,
                alignItems: "center",
                minHeight: 64,
                px: 1.5,
                py: 1,
                borderRadius: 1.5,
                border: 1,
                borderColor: "transparent",
                "&:has([data-settings-choice]:focus-visible)": {
                    borderColor: "primary.main",
                    bgcolor: "action.hover",
                },
            }}
        >
            <Stack direction="row" spacing={1} alignItems="flex-start" sx={{ minWidth: 0 }}>
                <Kbd
                    keys={shortcut.toUpperCase()}
                    availability={shortcutAvailable ? "available" : "inactive"}
                />
                <Box sx={{ minWidth: 0 }}>
                    <Typography variant="body2" fontWeight={700}>{label}</Typography>
                    <Typography variant="caption" color="text.secondary">{description}</Typography>
                </Box>
            </Stack>
            {children}
        </Box>
    );
}

export type MobileSettingsSection =
    | "appearance"
    | "reading"
    | "agent"
    | "code"
    | "notifications"
    | "providers"
    | "machines"
    | "info"
    | "logs"
    | "account";

export const MOBILE_SETTINGS_HEAVY_CONTENT_DELAY_MS = 180;

export function initialMobileSettingsSection(
    tab: ControlCenterTab,
    focus: SettingsProductFocus,
): MobileSettingsSection | null {
    if (tab !== "settings") return tab;
    return focus === "code" ? "code" : null;
}

export function MobileSettingsRoute({
    id,
    title,
    description,
    activeSection,
    onChange,
    children,
}: {
    id: MobileSettingsSection;
    title: string;
    description: string;
    activeSection: MobileSettingsSection | null;
    onChange: (section: MobileSettingsSection | null) => void;
    children: React.ReactNode;
}): React.JSX.Element {
    const heavyContent = id === "machines" || id === "providers";
    const [contentReady, setContentReady] = useState(!heavyContent);
    useEffect(() => {
        if (!heavyContent) return undefined;
        if (activeSection !== id) {
            setContentReady(false);
            return undefined;
        }
        const timer = globalThis.setTimeout(
            () => setContentReady(true),
            MOBILE_SETTINGS_HEAVY_CONTENT_DELAY_MS,
        );
        return () => globalThis.clearTimeout(timer);
    }, [activeSection, heavyContent, id]);
    if (activeSection !== null && activeSection !== id) return <></>;
    if (activeSection === null) {
        return (
            <ButtonBase
                data-mobile-settings-route={id}
                onClick={(): void => onChange(id)}
                sx={{
                    width: "100%",
                    minHeight: 64,
                    px: 0.75,
                    py: 1,
                    gap: 1,
                    justifyContent: "flex-start",
                    textAlign: "left",
                    borderBottom: 1,
                    borderColor: "divider",
                    touchAction: "manipulation",
                    "&:first-of-type": { borderTop: 1, borderColor: "divider" },
                }}
            >
                <Stack spacing={0.125} sx={{ minWidth: 0, flex: 1 }}>
                    <Typography variant="body2" fontWeight={700}>{title}</Typography>
                    <Typography variant="caption" color="text.secondary" noWrap>
                        {description}
                    </Typography>
                </Stack>
                <ChevronRight fontSize="small" sx={{ color: "text.secondary", flexShrink: 0 }} />
            </ButtonBase>
        );
    }
    return (
        <Box
            data-mobile-settings-group={id}
            sx={{
                minHeight: "100%",
            }}
        >
            <Box
                id={`mobile-settings-${id}-header`}
                data-mobile-settings-detail-header
                sx={{
                    position: "sticky",
                    top: -1,
                    zIndex: 3,
                    bgcolor: "background.default",
                    px: 0.75,
                    pt: 1.75,
                    pb: 1.5,
                    borderBottom: 1,
                    borderColor: "divider",
                }}
            >
                <Stack spacing={0.5} sx={{ minWidth: 0 }}>
                    <Typography
                        variant="overline"
                        color="primary.main"
                        sx={{
                            fontSize: "0.68rem",
                            fontWeight: 760,
                            letterSpacing: "0.12em",
                            lineHeight: 1.2,
                        }}
                    >
                        Settings
                    </Typography>
                    <Typography variant="h6" fontWeight={780} sx={{ lineHeight: 1.2 }}>
                        {title}
                    </Typography>
                    <Typography variant="body2" color="text.secondary" sx={{ lineHeight: 1.45 }}>
                        {description}
                    </Typography>
                </Stack>
            </Box>
            <Box
                id={`mobile-settings-${id}-content`}
                sx={{
                    px: 0.75,
                    pt: 1.5,
                    pb: 12,
                }}
            >
                {contentReady
                    ? (
                        <Box
                            sx={{
                                animation: "mobile-settings-content-enter 100ms ease-out both",
                                "@keyframes mobile-settings-content-enter": {
                                    from: { opacity: 0 },
                                    to: { opacity: 1 },
                                },
                                "@media (prefers-reduced-motion: reduce)": {
                                    animation: "none",
                                },
                            }}
                        >
                            {children}
                        </Box>
                    )
                    : (
                        <Stack
                            direction="row"
                            spacing={1}
                            alignItems="center"
                            justifyContent="center"
                            sx={{ minHeight: 160, color: "text.secondary" }}
                        >
                            <CircularProgress size={desktopSize(18)} color="inherit" />
                            <Typography variant="caption">Loading…</Typography>
                        </Stack>
                    )}
            </Box>
        </Box>
    );
}

// Vim configuration: a quick Insert-escape choice (`jj`, …) plus the full
// vimrc subset it edits. Mappings apply to every CodeMirror editor; native
// text fields honour the Insert escape sequences.
export function DesktopVimConfigRows({
    shortcutsAvailable,
}: {
    shortcutsAvailable: boolean;
}): React.JSX.Element {
    const source = useVimConfigSource();
    const config = useMemo(() => parseVimConfig(source), [source]);
    const preset = selectedEscapePreset(source);
    return (
        <>
            <DesktopSettingsRow shortcut="I" shortcutAvailable={shortcutsAvailable} label="Leave Insert" description="Typed keys that act as Esc">
                <Box sx={{ display: "grid", gridTemplateColumns: `repeat(${String(VIM_ESCAPE_PRESETS.length + 1)}, minmax(0, 1fr))`, gap: 0.75 }}>
                    <DesktopSettingsChoice active={preset === null} onClick={() => setVimConfigSource(withEscapePreset(source, null))} ariaLabel="Esc only">Esc</DesktopSettingsChoice>
                    {VIM_ESCAPE_PRESETS.map((sequence) => (
                        <DesktopSettingsChoice key={sequence} active={preset === sequence} onClick={() => setVimConfigSource(withEscapePreset(source, sequence))} ariaLabel={`${sequence} leaves Insert`}>{sequence}</DesktopSettingsChoice>
                    ))}
                </Box>
            </DesktopSettingsRow>
            <DesktopSettingsRow shortcut="V" shortcutAvailable={shortcutsAvailable} label="Vim config" description="map / noremap lines and set timeoutlen=ms">
                <Stack spacing={0.5} sx={{ minWidth: 0 }}>
                    <Box
                        component="textarea"
                        data-settings-choice
                        aria-label="Vim config"
                        spellCheck={false}
                        rows={Math.min(8, Math.max(3, source.split("\n").length + 1))}
                        value={source}
                        placeholder={'inoremap jj <Esc>\nnnoremap H ^\nset timeoutlen=1000'}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) => setVimConfigSource(event.target.value)}
                        sx={{
                            width: "100%",
                            boxSizing: "border-box",
                            resize: "vertical",
                            px: 1,
                            py: 0.75,
                            borderRadius: 1.25,
                            border: 1,
                            borderColor: config.errors.length > 0 ? "warning.main" : "divider",
                            bgcolor: "transparent",
                            color: "text.primary",
                            font: "inherit",
                            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                            fontSize: "0.8125rem",
                            lineHeight: 1.5,
                            outline: "none",
                            "&:focus": { borderColor: "primary.main" },
                        }}
                    />
                    {config.errors.map((error) => (
                        <Typography key={error.line} variant="caption" color="warning.main" sx={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}>
                            Line {error.line}: {error.message}
                        </Typography>
                    ))}
                    {config.errors.length === 0 && config.mappings.length > 0 && (
                        <Typography variant="caption" color="text.secondary">
                            {config.mappings.length} mapping{config.mappings.length === 1 ? "" : "s"} · timeout {config.timeoutMs} ms
                        </Typography>
                    )}
                </Stack>
            </DesktopSettingsRow>
        </>
    );
}

export function DesktopSettingsContent({
    themeMode,
    onSetThemeMode,
    shortcutsAvailable,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (mode: ThemeMode) => void;
    shortcutsAvailable: boolean;
}): React.JSX.Element {
    const vim = useVimSetting();
    const sourceMode = useComposerSourceMode();
    const composerDebug = useComposerDebugSetting();
    const reading = useReadingSettings();
    const selectedFont = getFontPreset(reading.fontVariant);
    return (
        <Box
            sx={{
                display: "grid",
                gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
                gap: 2,
                alignItems: "start",
                "@media (max-width: 1279px)": {
                    gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
                },
            }}
        >
            <DesktopPanel label="Appearance">
                <Box sx={{ p: 1.5 }}><AppIconSettings /></Box>
                <ClientUpdateSettings />
                <DesktopSettingsRow shortcut="T" shortcutAvailable={shortcutsAvailable} label="Theme" description="Follow the system or pin a palette">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 0.75 }}>
                        {(["system", "light", "dark"] as const).map((mode) => (
                            <DesktopSettingsChoice key={mode} active={themeMode === mode} onClick={() => onSetThemeMode(mode)} ariaLabel={`${mode} theme`}>
                                {mode.charAt(0).toUpperCase() + mode.slice(1)}
                            </DesktopSettingsChoice>
                        ))}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="F" shortcutAvailable={shortcutsAvailable} label="Reading font" description="Typeface used for transcript prose">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 0.75 }}>
                        {FONT_PRESETS.map((preset) => (
                            <DesktopSettingsChoice key={preset.id} active={reading.fontVariant === preset.id} onClick={() => setFontVariant(preset.id)} ariaLabel={`${preset.label} reading font`}>
                                <Typography component="span" variant="body2" sx={{ fontFamily: preset.stack }} noWrap>{preset.label} · Aa 阅读</Typography>
                            </DesktopSettingsChoice>
                        ))}
                    </Box>
                </DesktopSettingsRow>
                <Box sx={{ mx: 1.5, mt: 1, p: 1.5, borderRadius: 1.5, bgcolor: "action.hover", border: 1, borderColor: "divider" }}>
                    <Typography variant="overline" color="text.secondary">Live preview</Typography>
                    <Typography
                        sx={{
                            mt: 0.5,
                            px: `${reading.padding}px`,
                            fontFamily: selectedFont.stack,
                            // Global font scale is already applied at <html>.
                            // Keep the preview at one inherited rem so changing
                            // Font size is represented once, not squared here.
                            fontSize: "1rem",
                            lineHeight: reading.lineHeight,
                        }}
                    >
                        Make the common path fast. 阅读输出时，密度、节奏和字形会立即反映在这里。
                    </Typography>
                </Box>
            </DesktopPanel>
            <DesktopPanel label="Density">
                <DesktopSettingsRow shortcut="Z" shortcutAvailable={shortcutsAvailable} label="Font size" description="Scale application text">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 0.75 }}>
                        {FONT_SCALE_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.fontScale, FONT_SCALE_PRESETS) === value} onClick={() => setFontScale(value)} ariaLabel={`${Math.round(value * 100)} percent font size`}>{Math.round(value * 100)}%</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="P" shortcutAvailable={shortcutsAvailable} label="Padding" description="Transcript and composer side gutter">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 0.75 }}>
                        {PADDING_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.padding, PADDING_PRESETS) === value} onClick={() => setPadding(value)} ariaLabel={`${value} pixel padding`}>{value}px</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="R" shortcutAvailable={shortcutsAvailable} label="Line height" description="Vertical rhythm for long output">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 0.75 }}>
                        {LINE_HEIGHT_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.lineHeight, LINE_HEIGHT_PRESETS) === value} onClick={() => setLineHeight(value)} ariaLabel={`${value.toFixed(1)} line height`}>{value.toFixed(1)}</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
            </DesktopPanel>
            <DesktopPanel
                label="Workflow"
                sx={{ "@media (max-width: 1279px)": { gridColumn: "1 / -1" } }}
            >
                <Box
                    sx={{
                        display: "grid",
                        gridTemplateColumns: "1fr",
                        columnGap: 2,
                        "@media (max-width: 1279px)": {
                            gridTemplateColumns: "1fr 1fr",
                        },
                    }}
                >
                    <DesktopSettingsRow shortcut="M" shortcutAvailable={shortcutsAvailable} label="Vim keybindings" description="Modal editing in the composer">
                        <DesktopSettingsChoice active={vim} onClick={() => setVimSetting(!vim)} ariaLabel="Toggle Vim keybindings">{vim ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                    {vim && <DesktopVimConfigRows shortcutsAvailable={shortcutsAvailable} />}
                    <DesktopSettingsRow shortcut="E" shortcutAvailable={shortcutsAvailable} label="Source mode" description="Edit the literal markdown instead of the live preview">
                        <DesktopSettingsChoice active={sourceMode} onClick={() => setComposerSourceMode(!sourceMode)} ariaLabel="Toggle composer source mode">{sourceMode ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                    <DesktopSettingsRow shortcut="D" shortcutAvailable={shortcutsAvailable} label="Debug mode" description="Verbose composer input logs for agents">
                        <DesktopSettingsChoice active={composerDebug} onClick={() => { const next = !composerDebug; setComposerDebugSetting(next); reportComposerDebugModeChanged(next); }} ariaLabel="Toggle composer debug mode">{composerDebug ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                </Box>
            </DesktopPanel>
            <DesktopPanel label="Agent tools" sx={{ gridColumn: "1 / -1" }}>
                <AgentToolsSettings />
            </DesktopPanel>
        </Box>
    );
}

export function DesktopSettingsSectionTabs({
    value,
    onChange,
}: {
    value: SettingsProductFocus;
    onChange: (value: SettingsProductFocus) => void;
}): React.JSX.Element {
    return (
        <SegmentedTabs
            value={value}
            onChange={onChange}
            fullWidth={false}
            aria-label="Settings workspace"
            rootProps={{ "data-desktop-settings-section-tabs": "" }}
            sx={{ gridAutoColumns: "minmax(112px, 1fr)" }}
            options={[
                { value: "agent", label: "Agent", icon: <ChatBubbleOutline /> },
                { value: "code", label: "Code & diff", icon: <CodeIcon /> },
            ]}
        />
    );
}

/** Desktop mirror of the mobile `account` route. Keep these four panels in
 *  the same order as that route so Passkey management, client credentials and
 *  sign out stay reachable from every surface. */
export function DesktopAccountTabContent(): React.JSX.Element {
    return (
        <Stack spacing={2}>
            <ProductSessionCapacityPanel />
            <ProductAccountSecurity />
            <ProductDevicesPanel hideWhenEmpty />
            <ProductAccountMenu />
        </Stack>
    );
}

export function machineComponentName(component: MachineChoice["components"][number]): string {
    if (component.id.kind === "zed_server") return "Language engine";
    if (component.id.kind === "zed_adapter") return "Language engine adapter";
    if (component.id.kind === "code_adapter") return "Code adapter";
    if (component.id.kind === "machine_host") return "Machine host";
    if (component.id.kind === "managed_node") return "Managed Node";
    return component.id.slot || component.id.kind.replaceAll("_", " ");
}

export function MachineNpmUpdateButton({
    updating,
    onUpdate,
    fullWidthOnTouch = false,
}: {
    updating: boolean;
    onUpdate: () => void;
    fullWidthOnTouch?: boolean;
}): React.JSX.Element {
    // A stationary touch can end scroll momentum without producing Safari's
    // synthetic click. Commit on pointerup while retaining ordinary click and
    // keyboard activation so Machine updates always reach the controller.
    const updateTap = useReliableTouchTap<HTMLButtonElement>(onUpdate);
    return (
        <Button
            size="small"
            variant="outlined"
            color="warning"
            disabled={updating}
            startIcon={updating ? <DelayedNetworkProgress size={desktopSize(14)} /> : <SystemUpdateAlt />}
            {...updateTap}
            sx={fullWidthOnTouch
                ? {
                    "@media (pointer: coarse), (max-width: 600px)": {
                        width: "100%",
                    },
                }
                : undefined}
        >
            {updating ? "Updating" : "Update"}
        </Button>
    );
}

export function ProvidersContent({
    embedded = false,
    providerIntent,
}: {
    embedded?: boolean;
    providerIntent?: ProviderSettingsIntent | undefined;
} = {}): React.JSX.Element {
    return (
        <Stack spacing={2}>
            {!embedded && <Box>
                <Typography fontWeight={760}>Providers</Typography>
                <Typography variant="caption" color="text.secondary">
                    Sign in and manage provider accounts
                </Typography>
            </Box>}
            <ProviderAuthenticationManagement
                focusProviderId={providerIntent?.providerId}
                autoBeginAuthentication={providerIntent?.autoBeginAuthentication}
                authenticationRequestId={providerIntent?.requestId}
            />
        </Stack>
    );
}

export function MachinesContent({ embedded = false }: { embedded?: boolean } = {}): React.JSX.Element {
    const machines = useStoreSelector((snapshot) => snapshot.machines);
    const [commandFeedback, setCommandFeedback] = useState<Record<string, ReturnType<typeof machineCommandResultPresentation>>>({});
    const commandFeedbackTimers = useRef<Record<string, number>>({});
    const [expanded, setExpanded] = useState<Record<string, boolean>>({});
    const [busy, setBusy] = useState<Record<string, boolean>>({});
    const [componentErrors, setComponentErrors] = useState<Record<string, string>>({});
    const [deleteTarget, setDeleteTarget] = useState<MachineChoice | null>(null);
    const [deleteError, setDeleteError] = useState("");
    const [deleting, setDeleting] = useState(false);
    const deletingLastMachine = deleteTarget !== null &&
        machines.length === 1 && machines[0]?.id === deleteTarget.id;
    const [updateConfirmation, setUpdateConfirmation] = useState<{
        machineId: string;
        components: readonly MachineChoice["components"][number][];
        action: "npm" | "reconcile-one";
    } | null>(null);
    const showCommandFeedback = useCallback((machineId: string, accepted: boolean): void => {
        setCommandFeedback((current) => ({
            ...current,
            [machineId]: machineCommandResultPresentation(accepted),
        }));
        const previous = commandFeedbackTimers.current[machineId];
        if (previous !== undefined) globalThis.clearTimeout(previous);
        commandFeedbackTimers.current[machineId] = globalThis.setTimeout(() => {
            setCommandFeedback((current) => {
                const next = { ...current };
                delete next[machineId];
                return next;
            });
            delete commandFeedbackTimers.current[machineId];
        }, 4_500);
    }, []);
    useEffect(() => () => {
        Object.values(commandFeedbackTimers.current).forEach((timer) => globalThis.clearTimeout(timer));
    }, []);
    const refreshMachine = useCallback(async (machineId: string): Promise<void> => {
        const response = await fetch(`/api/machines/${encodeURIComponent(machineId)}/refresh`, { method: "POST" });
        if (!response.ok) throw new Error(await response.text() || "Could not refresh Machine inventory");
        showCommandFeedback(machineId, true);
    }, [showCommandFeedback]);
    const updateOne = (machineId: string, component: MachineChoice["components"][number]): void => {
        const key = `${machineId}:component:${component.id.kind}:${component.id.slot ?? ""}`;
        setBusy((current) => ({ ...current, [key]: true }));
        setComponentErrors((current) => ({ ...current, [key]: "" }));
        void (async () => {
            try {
                const response = await fetch(`/api/machines/${encodeURIComponent(machineId)}/components/reconcile-one`, {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ kind: component.id.kind, slot: component.id.slot ?? "" }),
                });
                if (!response.ok) throw new Error(await response.text() || "Could not update component");
                showCommandFeedback(machineId, true);
            } catch (error) {
                showCommandFeedback(machineId, false);
                setComponentErrors((current) => ({
                    ...current,
                    [key]: error instanceof Error ? error.message : "Update failed",
                }));
            } finally {
                setBusy((current) => ({ ...current, [key]: false }));
            }
        })();
    };
    const updateNpm = (machineId: string, component: MachineChoice["components"][number]): void => {
        const key = `${machineId}:npm:${component.id.kind}:${component.id.slot ?? ""}`;
        setBusy((current) => ({ ...current, [key]: true }));
        setComponentErrors((current) => ({ ...current, [key]: "" }));
        void (async () => {
            try {
                const response = await fetch(`/api/machines/${encodeURIComponent(machineId)}/components/update-npm`, {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ kind: component.id.kind, slot: component.id.slot ?? "" }),
                });
                if (!response.ok) throw new Error(await response.text() || "Could not start update");
                showCommandFeedback(machineId, true);
            } catch (error) {
                showCommandFeedback(machineId, false);
                setComponentErrors((current) => ({
                    ...current,
                    [key]: error instanceof Error ? error.message : "Update failed",
                }));
            } finally {
                setBusy((current) => ({ ...current, [key]: false }));
            }
        })();
    };
    const requestNpmUpdate = (machineId: string, component: MachineChoice["components"][number]): void => {
        if (component.active_leases > 0) {
            setUpdateConfirmation({ machineId, components: [component], action: "npm" });
            return;
        }
        updateNpm(machineId, component);
    };
    const requestReconcileOne = (machineId: string, component: MachineChoice["components"][number]): void => {
        if (component.active_leases > 0) {
            setUpdateConfirmation({ machineId, components: [component], action: "reconcile-one" });
            return;
        }
        updateOne(machineId, component);
    };
    const confirmMachineUpdate = (): void => {
        const pending = updateConfirmation;
        if (!pending) return;
        setUpdateConfirmation(null);
        const component = pending.components[0];
        if (pending.action === "npm" && component) updateNpm(pending.machineId, component);
        else if (pending.action === "reconcile-one" && component) updateOne(pending.machineId, component);
    };
    const deleteMachine = (): void => {
        if (!deleteTarget || deleting) return;
        const machineId = deleteTarget.id;
        setDeleting(true);
        setDeleteError("");
        void fetch(`/api/machines/${encodeURIComponent(machineId)}/revoke`, {
            method: "POST",
            credentials: "same-origin",
        }).then(async (response) => {
            if (!response.ok) throw new Error(await response.text() || "Could not delete Machine");
            setDeleteTarget(null);
        }).catch((error: unknown) => {
            setDeleteError(error instanceof Error ? error.message : "Could not delete Machine");
        }).finally(() => setDeleting(false));
    };
    useConfirmEnter(updateConfirmation !== null, confirmMachineUpdate);
    return (
        <Stack spacing={2}>
            {!embedded && <Box>
                <Typography fontWeight={760}>Machines</Typography>
                <Typography variant="caption" color="text.secondary">Where Cowboy sessions run</Typography>
            </Box>}
            {machines.map((machine) => {
                const open = Boolean(expanded[machine.id]);
                const pending = machine.pending_updates ?? [];
                const projectWorkspaces = machine.workspaces.filter((workspace) =>
                    workspace.id !== "home" && workspace.id !== "columbus"
                );
                const visibleComponents = machine.components.filter((component) =>
                    component.state !== "missing" ||
                    component.id.kind === "zed_server" ||
                    component.id.kind === "zed_adapter" ||
                    pending.some((id) =>
                        id.kind === component.id.kind && (id.slot ?? "") === (component.id.slot ?? "")
                    )
                );
                const zedComponent = machine.components.find((component) =>
                    component.id.kind === "zed_server"
                );
                const componentSections = [
                    {
                        label: "Integrations",
                        components: visibleComponents.filter((component) =>
                            component.id.kind === "zed_server" || component.id.kind === "zed_adapter" || component.id.kind === "code_adapter"
                        ),
                    },
                    {
                        label: "Runtime",
                        components: visibleComponents.filter((component) =>
                            component.id.kind !== "provider_cli" &&
                            component.id.kind !== "provider_adapter" &&
                            component.id.kind !== "acp_runtime" &&
                            component.id.kind !== "zed_server" &&
                            component.id.kind !== "zed_adapter" &&
                            component.id.kind !== "code_adapter"
                        ),
                    },
                ].filter((section) => section.components.length > 0);
                const presence = machinePresencePresentation(machine.status);
                const commandResult = commandFeedback[machine.id] ?? null;
                return (
                    <Paper
                        key={machine.id}
                        variant="outlined"
                        sx={{
                            borderRadius: 1.5,
                            overflow: "hidden",
                            borderColor: open ? "primary.main" : "divider",
                            transition: "border-color .2s",
                            "& [data-provider-management-card]": {
                                contentVisibility: "auto",
                                containIntrinsicSize: "auto 220px",
                            },
                        }}
                    >
                        <Stack spacing={1.25} sx={{ p: 1.5 }}>
                            <Stack direction="row" alignItems="center" spacing={1.25}>
                                <StatusDot status={presence.indicator} />
                                <Box sx={{ minWidth: 0, flex: 1 }}>
                                    <Stack direction="row" spacing={0.75} alignItems="baseline" flexWrap="wrap" useFlexGap>
                                        <Typography fontWeight={740}>{machine.display_name}</Typography>
                                        <Typography variant="caption" color="text.secondary">
                                            {machine.platform} · {machine.architecture}{machine.local ? " · Local server" : ""}
                                        </Typography>
                                    </Stack>
                                    <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.1 }}>
                                        {machine.workspaces.length} workspace{machine.workspaces.length === 1 ? "" : "s"} · {machine.active_sessions} active session{machine.active_sessions === 1 ? "" : "s"}
                                        {` · ${machine.capacity.max_sessions} max`}
                                    </Typography>
                                </Box>
                                <Tooltip title="Delete from this Cowboy Service">
                                    <IconButton
                                        size="small"
                                        aria-label={`Delete ${machine.display_name}`}
                                        onClick={() => {
                                            setDeleteError("");
                                            setDeleteTarget(machine);
                                        }}
                                    >
                                        <DeleteOutline fontSize="small" />
                                    </IconButton>
                                </Tooltip>
                                <Chip
                                    size="small"
                                    label={machine.schedulable
                                        ? "Ready"
                                        : machine.capacity.draining
                                        ? "Draining"
                                        : presence.label}
                                    color={machine.schedulable ? "success" : "default"}
                                    sx={{ fontWeight: 650 }}
                                />
                            </Stack>
                            <Stack spacing={0.75}>
                                    <Stack direction="row" spacing={1} alignItems="center">
                                        <Typography variant="overline" color="text.secondary" sx={{ width: 72, flexShrink: 0 }}>Projects</Typography>
                                        <HintTooltip title={projectWorkspaces.map((workspace) => workspace.display_name).join(", ")}>
                                            <Chip
                                                size="small"
                                                variant="outlined"
                                                label={`${projectWorkspaces.length} project${projectWorkspaces.length === 1 ? "" : "s"}`}
                                            />
                                        </HintTooltip>
                                    </Stack>
                                    <Stack direction="row" spacing={1} alignItems="center">
                                        <Typography variant="overline" color="text.secondary" sx={{ width: 72, flexShrink: 0 }}>Integrations</Typography>
                                        <Chip
                                            size="small"
                                            variant="outlined"
                                            color={zedComponent?.state === "active" ? "success" : zedComponent?.state === "failed" ? "error" : "default"}
                                            label={`Code intelligence · ${zedComponent?.state === "active" ? "ready" : zedComponent?.state === "failed" ? "error" : "unavailable"}`}
                                        />
                                    </Stack>
                            </Stack>
                            <MachineProviderManagement machine={machine} />
                            <Stack direction="row" alignItems="center" spacing={0.5}>
                                <ButtonBase
                                    onClick={() => setExpanded((current) => ({ ...current, [machine.id]: !open }))}
                                    sx={{
                                        flex: 1,
                                        justifyContent: "space-between",
                                        minHeight: 40,
                                        px: 0.5,
                                        borderRadius: 1.5,
                                        color: "text.secondary",
                                    }}
                                >
                                    <Typography variant="body2" fontWeight={650}>
                                        {open ? "Hide details" : "Details & versions"}
                                    </Typography>
                                    {open ? <ExpandLess fontSize="small" /> : <ExpandMore fontSize="small" />}
                                </ButtonBase>
                                <Tooltip title="Recheck projects and versions">
                                    <NetworkIconButton
                                        size="small"
                                        aria-label={`Refresh ${machine.display_name} inventory`}
                                        networkAction={() => refreshMachine(machine.id)}
                                    >
                                        <RefreshIcon fontSize="small" />
                                    </NetworkIconButton>
                                </Tooltip>
                            </Stack>
                            {open && (
                                <Stack spacing={1.25} sx={{ pt: 0.25 }}>
                                    <MachineProjects machine={machine} machines={machines} />
                                    {componentSections.map((section) => (
                                        <Stack key={section.label} spacing={0.75}>
                                            <Typography variant="overline" color="text.secondary">{section.label}</Typography>
                                            {section.components.map((component) => {
                                                const componentPending = pending.some((id) =>
                                                    id.kind === component.id.kind && (id.slot ?? "") === (component.id.slot ?? "")
                                                );
                                                const componentKey = `${machine.id}:component:${component.id.kind}:${component.id.slot ?? ""}`;
                                                // The Controller converges signed automatic components on its
                                                // own. Report what it is doing instead of offering an action
                                                // that would race it; draining and blocked keep theirs, because
                                                // both are exactly where a person still decides.
                                                const convergence = (machine.convergence ?? []).find((entry) =>
                                                    entry.id.kind === component.id.kind &&
                                                    (entry.id.slot ?? "") === (component.id.slot ?? "")
                                                );
                                                const converging = convergence !== undefined &&
                                                    convergence.state !== "draining" && convergence.state !== "blocked";
                                                const convergenceState = convergence &&
                                                    machineConvergencePresentation(convergence, Date.now());
                                                const update = component.update;
                                                const release = machineVersionPresentation(
                                                    component.version,
                                                    component.state,
                                                    update,
                                                    componentPending,
                                                );
                                                const updateTitle = update
                                                    ? `Checked ${new Date(update.checked_at_ms).toLocaleString()} via ${update.source}`
                                                    : "No authoritative release comparison is available";
                                                const npmUpdateKey = `${machine.id}:npm:${component.id.kind}:${component.id.slot ?? ""}`;
                                                const npmUpdating = busy[npmUpdateKey];
                                                // An installed Plugin ships its own pinned runtime for this
                                                // slot, so the legacy host binary is not what sessions run.
                                                // Offering its unpinned `@latest` here would ask for
                                                // maintenance of a runtime Cowboy no longer executes.
                                                const supersession = component.superseded_by === undefined
                                                    ? undefined
                                                    : machineSupersessionPresentation(component.superseded_by);
                                                const npmInstallable = update?.available === true &&
                                                    update.installable && supersession === undefined;
                                                return (
                                                    <Stack
                                                        key={`${component.id.kind}:${component.id.slot ?? ""}`}
                                                        direction="row"
                                                        spacing={1}
                                                        alignItems="center"
                                                        flexWrap="wrap"
                                                        useFlexGap
                                                        sx={{
                                                            minWidth: 0,
                                                            py: 0.75,
                                                            px: 1,
                                                            borderRadius: 1.5,
                                                            bgcolor: "action.hover",
                                                        }}
                                                    >
                                                        <Box
                                                            sx={{
                                                                minWidth: 0,
                                                                flex: 1,
                                                            }}
                                                        >
                                                            <Typography variant="body2" fontWeight={650}>
                                                                {machineComponentName(component)}
                                                            </Typography>
                                                            <HintTooltip title={updateTitle}>
                                                                <Typography
                                                                    variant="caption"
                                                                    color="text.secondary"
                                                                    sx={{ display: "block", overflowWrap: "anywhere" }}
                                                                >
                                                                    {release.version}
                                                                    {component.generation ? ` · generation ${component.generation}` : ""}
                                                                </Typography>
                                                            </HintTooltip>
                                                        </Box>
                                                        {convergenceState && (
                                                            <HintTooltip title={convergenceState.detail}>
                                                                <Chip
                                                                    size="small"
                                                                    variant="outlined"
                                                                    color={convergenceState.tone}
                                                                    label={convergenceState.status}
                                                                />
                                                            </HintTooltip>
                                                        )}
                                                        {componentPending && !converging && (
                                                            <Button
                                                                size="small"
                                                                variant="outlined"
                                                                color="warning"
                                                                disabled={busy[componentKey]}
                                                                onClick={() => requestReconcileOne(machine.id, component)}
                                                            >{busy[componentKey] ? <DelayedNetworkProgress size={desktopSize(14)} /> : "Update"}</Button>
                                                        )}
                                                        {!componentPending && npmInstallable && (
                                                            <MachineNpmUpdateButton
                                                                updating={Boolean(npmUpdating)}
                                                                onUpdate={() => requestNpmUpdate(machine.id, component)}
                                                            />
                                                        )}
                                                        {!componentPending && !npmInstallable && !convergenceState &&
                                                            supersession && (
                                                            <HintTooltip title={supersession.detail}>
                                                                <Chip
                                                                    size="small"
                                                                    variant="outlined"
                                                                    color={supersession.tone}
                                                                    label={supersession.status}
                                                                />
                                                            </HintTooltip>
                                                        )}
                                                        {!componentPending && !npmInstallable && !convergenceState &&
                                                            !supersession && (
                                                            <HintTooltip title={updateTitle}>
                                                                <Chip
                                                                    size="small"
                                                                    variant="outlined"
                                                                    color={release.tone}
                                                                    label={release.status}
                                                                />
                                                            </HintTooltip>
                                                        )}
                                                        {componentErrors[npmUpdateKey] && (
                                                            <Alert severity="error" sx={{ width: "100%" }}>
                                                                {componentErrors[npmUpdateKey]}
                                                            </Alert>
                                                        )}
                                                    </Stack>
                                                );
                                            })}
                                        </Stack>
                                    ))}
                                </Stack>
                            )}
                            {commandResult && (
                                <Alert severity={commandResult.severity} sx={{ py: 0.25 }}>
                                    {commandResult.message}
                                </Alert>
                            )}
                        </Stack>
                    </Paper>
                );
            })}
            <ConfirmSheet
                open={deleteTarget !== null}
                onClose={() => !deleting && setDeleteTarget(null)}
                title={deletingLastMachine ? "Delete the last Machine?" : "Delete this Machine?"}
                actions={
                    <>
                        <Button color="inherit" disabled={deleting} onClick={() => setDeleteTarget(null)}>
                            Cancel
                        </Button>
                        <Button variant="contained" color="error" disabled={deleting} onClick={deleteMachine}>
                            {deleting ? "Deleting" : "Delete Machine"}
                        </Button>
                    </>
                }
            >
                <Stack spacing={1.5} sx={{ pt: 0.5 }}>
                    <Typography variant="body2">
                        {deleteTarget?.display_name} will be disconnected and removed from this Cowboy Service.
                    </Typography>
                    {deletingLastMachine && (
                        <Alert severity="warning">
                            This is the last registered computer. Running sessions will disconnect, New session will be unavailable, and Cowboy will return to Connect a computer.
                        </Alert>
                    )}
                    <Typography variant="caption" color="text.secondary">
                        Other Cowboy Services and this computer&apos;s local files are not affected. Reconnecting here requires a new one-time code.
                    </Typography>
                    {deleteError && <Alert severity="error">{deleteError}</Alert>}
                </Stack>
            </ConfirmSheet>
            <ConfirmSheet
                open={updateConfirmation !== null}
                onClose={() => setUpdateConfirmation(null)}
                title="Roll out this update?"
                actions={
                    <>
                        <Button color="inherit" onClick={() => setUpdateConfirmation(null)}>
                            Cancel
                            <Kbd keys="Esc" />
                        </Button>
                        <Button
                            variant="contained"
                            color="warning"
                            onClick={confirmMachineUpdate}
                        >
                            Update and roll out
                            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
                        </Button>
                    </>
                }
            >
                {updateConfirmation && (
                    <Stack spacing={2} sx={{ pt: 0.5 }}>
                        <Typography variant="body2">
                            {updateConfirmation.components.map(machineComponentName).join(", ")} {updateConfirmation.components.length === 1 ? "is" : "are"} used by active sessions on this Machine.
                        </Typography>
                        <Stack spacing={0.5}>
                            {updateConfirmation.components.map((component) => (
                                <Typography key={`${component.id.kind}:${component.id.slot ?? ""}`} variant="caption" color="text.secondary">
                                    {machineComponentName(component)} · {component.active_leases} active {component.active_leases === 1 ? "session" : "sessions"}
                                </Typography>
                            ))}
                        </Stack>
                        <Alert severity="warning">
                            Current turns will finish first. Cowboy will then replace affected workers gradually. Each session may be briefly unavailable while it reconnects; its transcript and agent session are preserved. Machine host or runtime updates may also reconnect the Machine service.
                        </Alert>
                    </Stack>
                )}
            </ConfirmSheet>
        </Stack>
    );
}

export function isSettingsEditableTarget(target: EventTarget | null): target is HTMLElement {
    return target instanceof HTMLElement && (
        target.matches("input, textarea, select, [contenteditable='true']") ||
        target.closest("[contenteditable='true']") !== null
    );
}

export function isSettingsTextEntryTarget(target: EventTarget | null): target is HTMLElement {
    if (target instanceof HTMLTextAreaElement) return true;
    if (target instanceof HTMLInputElement) {
        return ![
            "button",
            "checkbox",
            "color",
            "file",
            "hidden",
            "radio",
            "range",
            "reset",
            "submit",
        ].includes(target.type);
    }
    return target instanceof HTMLElement && (
        target.matches("[contenteditable='true']") ||
        target.closest("[contenteditable='true']") !== null
    );
}

export function SettingsShell({
    open,
    onClose,
    initialTab,
    initialSection,
    initialProviderIntent,
    themeMode,
    onSetThemeMode,
}: {
    open: boolean;
    onClose: () => void;
    initialTab: SettingsTab;
    initialSection: SettingsProductFocus;
    initialProviderIntent?: ProviderSettingsIntent | undefined;
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    // One tab model is shared by the touch segmented control and Desktop's
    // semantic tablist. Each entry remains independently addressable by key.
    const [tab, setTab] = useState<ControlCenterTab>(initialTab);
    const [renderedTab, setRenderedTab] = useState<ControlCenterTab>(initialTab);
    const [section, setSection] = useState<SettingsProductFocus>(initialSection);
    const [tabPanelVisible, setTabPanelVisible] = useState(true);
    const [mobileSettingsSection, setMobileSettingsSection] = useState<MobileSettingsSection | null>(
        initialMobileSettingsSection(initialTab, initialSection),
    );
    const [fontOpen, setFontOpen] = useState(false);
    // Vim and animated workbench tabs are desktop-only: both require a fine
    // pointer/physical-keyboard surface rather than the touch sheet.
    const desktop = useMediaQuery("(pointer: fine) and (hover: hover)");
    const settingsPanelRef = useRef<HTMLDivElement>(null);
    const settingsListRef = useRef<HTMLDivElement>(null);
    const settingsListScrollRef = useRef(0);
    const viewTransitionRef = useRef<ControlCenterViewTransition | null>(null);
    const settingsScrollSurface = useCallback((): HTMLElement | null => {
        const list = settingsPanelRef.current ?? settingsListRef.current;
        return list === null ? null : closestScrollableSettingsSurface(list);
    }, []);
    const captureSettingsListScroll = useCallback((): void => {
        const surface = settingsScrollSurface();
        if (surface === null) return;
        settingsListScrollRef.current = nextSavedSettingsScroll(
            tab,
            surface.scrollTop,
            settingsListScrollRef.current,
        );
    }, [settingsScrollSurface, tab]);
    const applyDestinationScroll = useCallback((destination: ControlCenterTab): void => {
        const surface = settingsScrollSurface();
        if (surface === null) return;
        surface.scrollTop = destinationScrollTop(
            destination,
            settingsListScrollRef.current,
        );
    }, [settingsScrollSurface]);
    const changeTab = useCallback((next: ControlCenterTab): void => {
        if (next === tab) return;
        captureSettingsListScroll();
        const reducedMotion = globalThis.matchMedia(
            "(prefers-reduced-motion: reduce)",
        ).matches;
        const transitionDocument = globalThis.document as
            ControlCenterViewTransitionDocument;
        const startViewTransition = transitionDocument.startViewTransition;
        if (desktop && !reducedMotion && startViewTransition !== undefined) {
            viewTransitionRef.current?.skipTransition();
            // The active indicator can move immediately. The browser captures
            // the old panel before React mounts the next heavy tree, keeping
            // useful content visible throughout that synchronous render.
            flushSync(() => {
                setTabPanelVisible(true);
                setTab(next);
            });
            try {
                const transition = startViewTransition.call(
                    transitionDocument,
                    () => {
                        flushSync(() => {
                            setRenderedTab(next);
                        });
                        applyDestinationScroll(next);
                    },
                );
                viewTransitionRef.current = transition;
                // `skipTransition()` intentionally rejects `ready` when a
                // second tab wins the race. Consume every native lifecycle
                // promise so rapid navigation remains console-clean.
                void transition.ready.catch(() => undefined);
                void transition.updateCallbackDone.catch(() => undefined);
                void transition.finished
                    .catch(() => undefined)
                    .finally(() => {
                        if (viewTransitionRef.current === transition) {
                            viewTransitionRef.current = null;
                        }
                    });
                return;
            } catch {
                // Fall through to the persistent-panel CSS transition when a
                // browser advertises the API but cannot start this transition.
            }
        }
        // Compatibility path: move the indicator immediately, then let the old
        // content exit before the selected panel mounts.
        setTabPanelVisible(false);
        setTab(next);
    }, [applyDestinationScroll, captureSettingsListScroll, desktop, tab]);
    useEffect(() => {
        if (!open) {
            viewTransitionRef.current?.skipTransition();
            viewTransitionRef.current = null;
            return;
        }
        const openingTab = desktop ? initialTab : "settings";
        setTab(openingTab);
        setRenderedTab(openingTab);
        setSection(initialSection);
        setTabPanelVisible(true);
        setMobileSettingsSection(initialMobileSettingsSection(initialTab, initialSection));
        setFontOpen(false);
        settingsListScrollRef.current = 0;
    }, [desktop, open, initialTab, initialSection]);
    const changeMobileSettingsSection = useCallback((next: MobileSettingsSection | null): void => {
        const surface = settingsScrollSurface();
        if (mobileSettingsSection === null && next !== null && surface !== null) {
            settingsListScrollRef.current = surface.scrollTop;
        }
        setMobileSettingsSection(next);
    }, [mobileSettingsSection, settingsScrollSurface]);
    useLayoutEffect(() => {
        if (!open || desktop || tab !== "settings") return;
        const surface = settingsScrollSurface();
        if (surface === null) return;
        surface.scrollTop = mobileSettingsSection === null
            ? settingsListScrollRef.current
            : 0;
    }, [desktop, mobileSettingsSection, open, settingsScrollSurface, tab]);
    useLayoutEffect(() => {
        if (!open || !tabPanelVisible || renderedTab !== tab) return;
        applyDestinationScroll(renderedTab);
    }, [applyDestinationScroll, open, renderedTab, tab, tabPanelVisible]);
    useEffect(() => {
        if (!open || desktop) return undefined;
        const doc = globalThis.document;
        const viewport = globalThis.visualViewport;
        let raf = 0;
        let timers: number[] = [];
        const clearTimers = (): void => {
            for (const timer of timers) globalThis.clearTimeout(timer);
            timers = [];
        };
        const revealFocusedSetting = (): void => {
            raf = 0;
            const panel = settingsListRef.current;
            const active = doc.activeElement;
            if (
                panel === null || !isSettingsTextEntryTarget(active) ||
                !panel.contains(active)
            ) return;
            const surface = closestScrollableSettingsSurface(panel);
            const sheet = surface.closest<HTMLElement>(
                "[data-detent-sheet='true']",
            );
            const navigation = sheet?.querySelector<HTMLElement>(
                "[data-mobile-settings-navigation]",
            );
            if (navigation === undefined || navigation === null) return;
            const field = active.closest<HTMLElement>(".MuiFormControl-root") ?? active;
            const header = panel.querySelector<HTMLElement>(
                "[data-mobile-settings-detail-header]",
            );
            const surfaceBounds = surface.getBoundingClientRect();
            const delta = settingsFocusRevealDelta(
                field.getBoundingClientRect(),
                {
                    top: Math.max(
                        surfaceBounds.top,
                        header?.getBoundingClientRect().bottom ?? surfaceBounds.top,
                    ),
                    bottom: Math.min(
                        surfaceBounds.bottom,
                        navigation.getBoundingClientRect().top,
                    ),
                },
            );
            if (Math.abs(delta) < 0.5) return;
            surface.scrollBy({ top: delta, behavior: "auto" });
        };
        const queueReveal = (): void => {
            if (raf !== 0) globalThis.cancelAnimationFrame(raf);
            raf = globalThis.requestAnimationFrame(revealFocusedSetting);
        };
        const scheduleReveal = (): void => {
            queueReveal();
            clearTimers();
            timers = [120, 300, 550].map((delay) =>
                globalThis.setTimeout(revealFocusedSetting, delay)
            );
        };
        const onFocusIn = (event: FocusEvent): void => {
            const panel = settingsListRef.current;
            if (
                panel !== null && panel.contains(event.target as Node) &&
                isSettingsTextEntryTarget(event.target)
            ) scheduleReveal();
        };
        doc.addEventListener("focusin", onFocusIn);
        viewport?.addEventListener("resize", scheduleReveal);
        globalThis.addEventListener(KEYBOARD_INSET_CHANGED_EVENT, scheduleReveal);
        if (isSettingsTextEntryTarget(doc.activeElement)) scheduleReveal();
        return () => {
            doc.removeEventListener("focusin", onFocusIn);
            viewport?.removeEventListener("resize", scheduleReveal);
            globalThis.removeEventListener(KEYBOARD_INSET_CHANGED_EVENT, scheduleReveal);
            clearTimers();
            if (raf !== 0) globalThis.cancelAnimationFrame(raf);
        };
    }, [desktop, mobileSettingsSection, open]);
    const composerDebug = useComposerDebugSetting();
    const reading = useReadingSettings();
    // Font picker is collapsed by default (the 7 preview cards otherwise fill the
    // screen); the collapsed summary still shows the current face. Resets to
    // collapsed each time Settings opens — the desired compact default.
    const selectedFont = getFontPreset(reading.fontVariant);
    // Shared card style for both the collapsed summary and the expanded list.
    const fontCardSx = (active: boolean): SxProps<Theme> => ({
        cursor: "pointer",
        borderRadius: 1,
        border: 2,
        borderColor: active ? "primary.main" : "divider",
        px: 1.5,
        py: 1,
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 1,
        transition: "border-color 0.15s ease",
        "&:hover": { borderColor: active ? "primary.main" : "text.secondary" },
    });
    // Surface policy for this content-heavy modal:
    // - phones (<768px) stay a bottom sheet even with a fine pointer;
    // - portrait/mid-size touch tablets (<1024px, coarse pointer) keep the
    //   touch-native sheet;
    // - landscape tablets and desktop windows (>=1024px), plus pointer-driven
    //   devices from 768px up, use the centered dialog.
    const useSheetSurface = useMediaQuery(
        "(max-width: 767.95px), (min-width: 768px) and (max-width: 1023.95px) and (pointer: coarse)",
    );
    useEffect(() => {
        if (!open || tabPanelVisible) return undefined;
        const reducedMotion = globalThis.matchMedia(
            "(prefers-reduced-motion: reduce)",
        ).matches;
        let firstFrame = 0;
        let secondFrame = 0;
        let swapTimer: number | null = null;
        const mountSelectedPanel = (): void => {
            setRenderedTab(tab);
            firstFrame = globalThis.requestAnimationFrame(() => {
                applyDestinationScroll(tab);
                secondFrame = globalThis.requestAnimationFrame(() => {
                    setTabPanelVisible(true);
                    applyDestinationScroll(tab);
                });
            });
        };
        if (desktop && !reducedMotion) {
            swapTimer = globalThis.setTimeout(
                mountSelectedPanel,
                CONTROL_CENTER_PANEL_EXIT_MS,
            );
        } else {
            // Touch and reduced-motion surfaces keep the existing two-frame
            // deferral, but do not wait for or render an animation.
            firstFrame = globalThis.requestAnimationFrame(() => {
                secondFrame = globalThis.requestAnimationFrame(() => {
                    setRenderedTab(tab);
                    setTabPanelVisible(true);
                    applyDestinationScroll(tab);
                });
            });
        }
        return () => {
            if (swapTimer !== null) globalThis.clearTimeout(swapTimer);
            if (firstFrame !== 0) globalThis.cancelAnimationFrame(firstFrame);
            if (secondFrame !== 0) globalThis.cancelAnimationFrame(secondFrame);
        };
    }, [applyDestinationScroll, desktop, open, tab, tabPanelVisible]);
    const [settingsGoPrefix, setSettingsGoPrefix] = useState(false);
    const [settingsEditableFocus, setSettingsEditableFocus] = useState(false);
    useEffect(() => {
        if (!open || !desktop) return undefined;
        let goPending = false;
        let goTimer: number | null = null;
        const setGoPending = (pending: boolean): void => {
            goPending = pending;
            setSettingsGoPrefix(pending);
            if (goTimer !== null) globalThis.clearTimeout(goTimer);
            goTimer = pending
                ? globalThis.setTimeout(() => {
                    goPending = false;
                    setSettingsGoPrefix(false);
                }, 1200)
                : null;
        };
        const focusFirst = requestAnimationFrame(() => {
            settingsPanelRef.current?.querySelector<HTMLElement>(
                "[data-settings-choice]",
            )?.focus();
        });
        const onKeyDown = (event: KeyboardEvent): void => {
            if (desktopImeOwnsKey(event)) return;
            const target = event.target;
            if (isSettingsEditableTarget(target)) {
                if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    target.blur();
                    const returnTarget = target.closest<HTMLElement>(
                        "[data-settings-row]",
                    );
                    requestAnimationFrame(() =>
                        (returnTarget ?? settingsPanelRef.current)?.focus({ preventScroll: true }));
                }
                return;
            }
            const key = workspaceCommandKey(event).toLowerCase();
            const panel = settingsPanelRef.current;
            const scrollSurface = panel === null
                ? null
                : closestScrollableSettingsSurface(panel);
            if (event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && ["d", "u", "f", "b"].includes(key)) {
                if (!scrollSurface) return;
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                const direction = key === "d" || key === "f" ? 1 : -1;
                const distance = scrollSurface.clientHeight * (key === "d" || key === "u" ? 0.5 : 0.92);
                scrollSurface.scrollBy({ top: direction * distance, behavior: "auto" });
                return;
            }
            if (event.metaKey || event.ctrlKey || event.altKey) {
                setGoPending(false);
                return;
            }
            if (event.repeat && key === "g") {
                event.preventDefault();
                event.stopPropagation();
                return;
            }
            if (goPending) {
                event.preventDefault();
                event.stopPropagation();
                const goTop = !event.shiftKey && key === "g";
                setGoPending(false);
                if (goTop) scrollSurface?.scrollTo({ top: 0, behavior: "auto" });
                return;
            }
            if (event.shiftKey && key === "g") {
                if (!scrollSurface) return;
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                scrollSurface.scrollTo({ top: scrollSurface.scrollHeight, behavior: "auto" });
                return;
            }
            if (!event.shiftKey && key === "g") {
                if (!scrollSurface) return;
                event.preventDefault();
                event.stopPropagation();
                setGoPending(true);
                return;
            }
            const directTab = !event.shiftKey
                ? controlCenterTabForShortcut(key)
                : null;
            if (directTab) {
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                changeTab(directTab);
                return;
            }
            if (!event.shiftKey && (key === "[" || key === "]")) {
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                changeTab(adjacentControlCenterTab(tab, key === "]" ? 1 : -1));
                return;
            }
            if (!panel) return;
            if (!desktop && tab !== "settings") return;
            const rows = [...panel.querySelectorAll<HTMLElement>("[data-settings-row]")];
            const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
            const activeRow = active?.closest<HTMLElement>("[data-settings-row]");
            const focusChoice = (row: HTMLElement, preferred = 0): void => {
                const choices = [...row.querySelectorAll<HTMLElement>("[data-settings-choice]")];
                const selected = choices.findIndex((choice) => choice.getAttribute("aria-pressed") === "true");
                choices[Math.max(0, Math.min(choices.length - 1, preferred < 0 ? selected : preferred))]?.focus();
            };
            const direct = panel.querySelector<HTMLElement>(`[data-settings-shortcut="${CSS.escape(key)}"]`);
            if (direct && !["h", "j", "k", "l"].includes(key)) {
                event.preventDefault();
                focusChoice(direct, -1);
                return;
            }
            if (!["h", "j", "k", "l"].includes(key)) return;
            event.preventDefault();
            if (desktop && !activeRow) {
                const controls = [...panel.querySelectorAll<HTMLElement>(
                    'button:not(:disabled), [href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]',
                )].filter((control) => control.offsetParent !== null);
                const controlIndex = active ? controls.indexOf(active) : -1;
                const delta = key === "j" || key === "l" ? 1 : -1;
                controls[Math.max(0, Math.min(controls.length - 1, controlIndex + delta))]?.focus();
                return;
            }
            const rowIndex = activeRow ? rows.indexOf(activeRow) : -1;
            if (key === "j" || key === "k") {
                const next = rowIndex < 0
                    ? (key === "j" ? rows[0] : rows.at(-1))
                    : rows[Math.max(0, Math.min(rows.length - 1, rowIndex + (key === "j" ? 1 : -1)))];
                if (next) focusChoice(next, -1);
                return;
            }
            const row = activeRow ?? rows[0];
            if (!row) return;
            const choices = [...row.querySelectorAll<HTMLElement>("[data-settings-choice]")];
            const choiceIndex = active ? choices.indexOf(active) : -1;
            const next = choices[Math.max(0, Math.min(choices.length - 1, choiceIndex + (key === "l" ? 1 : -1)))];
            next?.focus();
        };
        const onFocusIn = (event: FocusEvent): void => {
            const panel = settingsPanelRef.current;
            if (
                panel?.contains(event.target as Node) &&
                isSettingsEditableTarget(event.target)
            ) setGoPending(false);
        };
        globalThis.addEventListener("keydown", onKeyDown, true);
        globalThis.addEventListener("focusin", onFocusIn, true);
        return () => {
            cancelAnimationFrame(focusFirst);
            if (goTimer !== null) globalThis.clearTimeout(goTimer);
            globalThis.removeEventListener("keydown", onKeyDown, true);
            globalThis.removeEventListener("focusin", onFocusIn, true);
            setSettingsGoPrefix(false);
            setSettingsEditableFocus(false);
        };
    }, [changeTab, desktop, open, tab]);
    const settingsShortcutsAvailable = !settingsGoPrefix && !settingsEditableFocus;
    return (
        <Sheet
            open={open}
            onClose={onClose}
            forceSheet={useSheetSurface}
            wide
            cover
            portal
            desktopMaxWidth={1440}
            mobileDismiss={desktop ? "footer" : "none"}
            actions={!desktop
                ? (
                    <Box data-mobile-settings-navigation sx={{ width: "100%" }}>
                        <MobileSheetActionGroup
                            actions={[{
                                key: mobileSettingsSection === null ? "close" : "back",
                                label: mobileSettingsSection === null ? "Close" : "Back",
                                onPress: mobileSettingsSection === null
                                    ? onClose
                                    : (): void => changeMobileSettingsSection(null),
                                icon: mobileSettingsSection === null
                                    ? <CloseIcon aria-hidden fontSize="small" />
                                    : <ArrowBackIosNew aria-hidden fontSize="small" />,
                            }]}
                        />
                    </Box>
                )
                : undefined}
        >
            <GlobalStyles styles={controlCenterViewTransitionStyles} />
            {/* The control center runs the modal grammar itself: digits and
                [ ] for sections, J/K rows, H/L choices (FOCUS.md "Modals"). */}
            {desktop && <Box component="span" hidden data-desktop-modal-keys="own" />}
            {/* Touch is one mutually exclusive accordion list. Desktop keeps
                native tabs so Machines / Info / Logs remain first-class. */}
            {desktop && (
            <Box
                sx={{
                    position: "sticky",
                    top: -1,
                    zIndex: 4,
                    // On the sheet surface this band really does sit over the
                    // scrolling panel, so it keeps a fill. Frosted rather than
                    // opaque: an opaque `background.paper` printed a lighter
                    // seam across the translucent Desktop modal material. Same
                    // material as the shortcut bar closing the other end.
                    bgcolor: (theme) => alpha(theme.palette.background.paper, 0.94),
                    backdropFilter: "blur(16px)",
                    pt: 0,
                    pb: 0,
                    borderRadius: 0,
                    borderBottom: 0,
                    borderColor: "divider",
                }}
            >
            <Box
                sx={{
                    display: "flex",
                    alignItems: "center",
                    mt: 0.25,
                    mb: 1,
                    py: 0.5,
                    width: "100%",
                }}
            >
                <Box sx={{ flex: 1 }}>
                    <Typography variant="h6" fontWeight={780}>Cowboy control center</Typography>
                    <Typography variant="caption" color="text.secondary">Preferences, runtime information, and automation history</Typography>
                </Box>
                <Box
                    sx={{
                        flex: 1,
                        display: "flex",
                        justifyContent: "flex-end",
                        visibility: useSheetSurface ? "hidden" : "visible",
                        pointerEvents: useSheetSurface ? "none" : "auto",
                    }}
                >
                    <Box sx={{ position: "relative", display: "inline-flex" }}>
                        <IconButton aria-label="close settings" onClick={onClose} sx={{ width: 36, height: 36 }}>
                            <CloseIcon fontSize="small" />
                        </IconButton>
                        <Kbd
                            keys="Esc"
                            floating
                            availability={settingsShortcutsAvailable ? "available" : "inactive"}
                        />
                    </Box>
                </Box>
            </Box>
            {desktop && (
                <SegmentedTabs
                    value={tab}
                    onChange={changeTab}
                    aria-label="Control center sections"
                    sx={{ px: 1, py: 0.75, borderBottom: 1, borderColor: "divider", boxSizing: "border-box" }}
                    options={CONTROL_CENTER_TABS.map(({ value, label, shortcut }) => ({
                        value,
                        id: `control-center-tab-${value}`,
                        controls: `control-center-panel-${value}`,
                        keyShortcuts: shortcut,
                        ariaLabel: label,
                        label: (
                            <Stack component="span" direction="row" spacing={0.75} alignItems="center" justifyContent="center">
                                <Box component="span" sx={{ overflow: "hidden", textOverflow: "ellipsis" }}>{label}</Box>
                                <Kbd
                                    keys={shortcut}
                                    availability={settingsShortcutsAvailable ? "available" : "inactive"}
                                />
                            </Stack>
                        ),
                    }))}
                />
            )}
            </Box>
            )}
            {desktop ? (
                <Box
                    ref={settingsPanelRef}
                    data-desktop-settings-workbench
                    data-control-center-tab={tab}
                    role="tabpanel"
                    id={`control-center-panel-${tab}`}
                    aria-labelledby={`control-center-tab-${tab}`}
                    aria-busy={!tabPanelVisible}
                    tabIndex={-1}
                    onFocusCapture={(event): void => {
                        setSettingsEditableFocus(isSettingsEditableTarget(event.target));
                    }}
                    onBlurCapture={(): void => {
                        requestAnimationFrame(() => {
                            const panel = settingsPanelRef.current;
                            setSettingsEditableFocus(
                                panel !== null && panel.contains(document.activeElement) &&
                                isSettingsEditableTarget(document.activeElement),
                            );
                        });
                    }}
                    sx={{
                        flex: 1,
                        minHeight: 0,
                        overflowY: "auto",
                        pt: 1.5,
                        pr: 0.75,
                        pb: 2,
                    }}
                >
                    <Box
                        key={renderedTab}
                        data-control-center-panel-content
                        data-control-center-rendered-tab={renderedTab}
                        sx={controlCenterPanelMotionSx(tabPanelVisible)}
                    >
                        {renderedTab === "settings" ? (
                            <Stack spacing={2}>
                                <DesktopSettingsSectionTabs
                                    value={section}
                                    onChange={setSection}
                                />
                                {section === "code"
                                    ? <ReviewSettingsContent />
                                    : (
                                        <DesktopSettingsContent
                                            themeMode={themeMode}
                                            onSetThemeMode={onSetThemeMode}
                                            shortcutsAvailable={settingsShortcutsAvailable}
                                        />
                                    )}
                            </Stack>
                        ) : renderedTab === "notifications" ? <NotificationSettingsContent /> : renderedTab === "providers" ? <ProvidersContent providerIntent={initialProviderIntent} /> : renderedTab === "machines" ? <MachinesContent /> : renderedTab === "account" ? <DesktopAccountTabContent /> : renderedTab === "info" ? (
                            <InfoContent desktop />
                        ) : <UsageLogs />}
                    </Box>
                </Box>
            ) : !tabPanelVisible ? null : renderedTab === "notifications" ? <NotificationSettingsContent /> : renderedTab === "providers" ? <ProvidersContent providerIntent={initialProviderIntent} /> : renderedTab === "machines" ? <MachinesContent /> : renderedTab === "info" ? <InfoContent /> : renderedTab === "logs" ? <UsageLogs /> : (
            <Stack
                key={mobileSettingsSection ?? "index"}
                ref={settingsListRef}
                data-settings-list="true"
                data-mobile-settings-level={mobileSettingsSection === null ? "index" : "detail"}
                spacing={0}
                sx={{
                    animation: "mobile-settings-route-enter 140ms cubic-bezier(0.2, 0.8, 0.2, 1) both",
                    willChange: "transform, opacity",
                    "@keyframes mobile-settings-route-enter": {
                        from: {
                            opacity: 0,
                            transform: mobileSettingsSection === null
                                ? "translate3d(-8px, 0, 0)"
                                : "translate3d(8px, 0, 0)",
                        },
                        to: { opacity: 1, transform: "translate3d(0, 0, 0)" },
                    },
                    "@media (prefers-reduced-motion: reduce)": {
                        animation: "none",
                    },
                }}
            >
                <MobileSettingsRoute
                    id="appearance"
                    title="Appearance"
                    description="Theme, app icon, and reading font"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                <Stack spacing={2}>
                <AppIconSettings />
                <ClientUpdateSettings />
                <Stack
                    direction="row"
                    alignItems="center"
                    justifyContent="space-between"
                    spacing={2}
                >
                    <Stack>
                        <Typography variant="body2">Theme</Typography>
                        <Typography variant="caption" color="text.secondary">
                            Follow the system or pin a palette
                        </Typography>
                    </Stack>
                    <Select
                        size="small"
                        value={themeMode}
                        onChange={(event): void =>
                            onSetThemeMode(event.target.value as ThemeMode)}
                        sx={{ minWidth: 104 }}
                    >
                        {(["system", "light", "dark"] as const).map((mode) => (
                            <MenuItem key={mode} value={mode}>
                                {mode.charAt(0).toUpperCase() + mode.slice(1)}
                            </MenuItem>
                        ))}
                    </Select>
                </Stack>

                {/* Reading comfort — scales only the transcript message content
                    and its side gutter (chrome stays put). Dropdowns, not a
                    slider: they tap cleanly on touch (see readingSettings). */}
                    {/* Font family — collapsible. Collapsed shows the current
                        face previewed in itself (so the selection is always
                        visible without the 7-card list filling the screen);
                        expanding drops the full picker, and choosing a face
                        auto-collapses. Each card previews its own @fontsource
                        woff2 (loaded lazily once selected). */}
                    {fontOpen ? (
                        <Stack spacing={0.75}>
                            {/* Collapse header */}
                            <Box
                                onClick={(): void => setFontOpen(false)}
                                sx={{
                                    display: "flex",
                                    alignItems: "center",
                                    justifyContent: "space-between",
                                    cursor: "pointer",
                                    px: 0.5,
                                }}
                            >
                                <Typography variant="body2">
                                    Reading font
                                </Typography>
                                <ExpandLess sx={{ color: "text.secondary" }} />
                            </Box>
                            {FONT_PRESETS.map((preset) => {
                                const selected =
                                    reading.fontVariant === preset.id;
                                return (
                                    <Box
                                        key={preset.id}
                                        onClick={(): void => {
                                            setFontVariant(preset.id);
                                            setFontOpen(false);
                                        }}
                                        sx={fontCardSx(selected)}
                                    >
                                        <Box sx={{ minWidth: 0 }}>
                                            <Typography
                                                sx={{
                                                    fontFamily: preset.stack,
                                                    fontSize: "1.05rem",
                                                    lineHeight: 1.3,
                                                }}
                                                noWrap
                                            >
                                                {preset.label} · 阅读 Aa
                                            </Typography>
                                            <Typography
                                                variant="caption"
                                                color="text.secondary"
                                                noWrap
                                            >
                                                {preset.note}
                                            </Typography>
                                        </Box>
                                        {selected && (
                                            <CheckIcon
                                                fontSize="medium"
                                                color="primary"
                                            />
                                        )}
                                    </Box>
                                );
                            })}
                        </Stack>
                    ) : (
                        // Collapsed summary — the current face, tap to change.
                        <Box
                            onClick={(): void => setFontOpen(true)}
                            aria-label="Change reading font"
                            sx={fontCardSx(false)}
                        >
                            <Box sx={{ minWidth: 0 }}>
                                <Typography
                                    variant="caption"
                                    color="text.secondary"
                                >
                                    Reading font
                                </Typography>
                                <Typography
                                    sx={{
                                        fontFamily: selectedFont.stack,
                                        fontSize: "1.05rem",
                                        lineHeight: 1.3,
                                    }}
                                    noWrap
                                >
                                    {selectedFont.label} · 阅读 Aa
                                </Typography>
                            </Box>
                            <ExpandMore sx={{ color: "text.secondary" }} />
                        </Box>
                    )}
                </Stack>
                </MobileSettingsRoute>

                <MobileSettingsRoute
                    id="reading"
                    title="Reading layout"
                    description="Text size, padding, and line spacing"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <Stack spacing={2}>
                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Font size</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Scales all app text
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.fontScale,
                                FONT_SCALE_PRESETS,
                            )}
                            onChange={(e): void =>
                                setFontScale(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {FONT_SCALE_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {Math.round(v * 100)}%
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Padding</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Side gutter of the transcript + composer
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.padding,
                                PADDING_PRESETS,
                            )}
                            onChange={(e): void =>
                                setPadding(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {PADDING_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {`${v}px`}
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Line height</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Spacing between lines of text
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.lineHeight,
                                LINE_HEIGHT_PRESETS,
                            )}
                            onChange={(e): void =>
                                setLineHeight(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {LINE_HEIGHT_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {v.toFixed(1)}
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                    </Stack>
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="agent"
                    title="Agent behavior"
                    description="Agent tools and composer diagnostics"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <Stack spacing={2}>
                <AgentToolsSettings />
                <Stack
                    direction="row"
                    alignItems="center"
                    justifyContent="space-between"
                    spacing={2}
                >
                    <Stack>
                        <Typography variant="body2">
                            Debug mode
                        </Typography>
                        <Typography
                            variant="caption"
                            color="text.secondary"
                        >
                            Send verbose composer input logs so an agent can
                            diagnose caret, IME, and Return on this device
                        </Typography>
                    </Stack>
                    <Switch
                        checked={composerDebug}
                        onChange={(e): void => {
                            setComposerDebugSetting(e.target.checked);
                            reportComposerDebugModeChanged(e.target.checked);
                        }}
                        inputProps={{ "aria-label": "Composer debug mode" }}
                    />
                </Stack>
                    </Stack>
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="code"
                    title="Code & diff"
                    description="Code display and review preferences"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                <Box
                    data-settings-section="code"
                    sx={{ scrollMarginTop: 16 }}
                >
                    <ReviewSettingsContent />
                </Box>
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="notifications"
                    title="Notifications"
                    description="Apple alerts, privacy, sound, and vibration"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <NotificationSettingsContent embedded />
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="providers"
                    title="Accounts & sign-in"
                    description="Provider authentication and accounts"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <ProvidersContent
                        embedded
                        providerIntent={initialProviderIntent}
                    />
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="machines"
                    title="Machines"
                    description="Connected runtimes and component updates"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <MachinesContent embedded />
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="info"
                    title="About"
                    description="Cowboy information and diagnostics"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <InfoContent />
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="logs"
                    title="Logs"
                    description="Usage and diagnostic history"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <UsageLogs dense />
                </MobileSettingsRoute>
                <MobileSettingsRoute
                    id="account"
                    title="Account"
                    description="Product login, Passkeys, clients, and sign out"
                    activeSection={mobileSettingsSection}
                    onChange={changeMobileSettingsSection}
                >
                    <Stack data-mobile-account-sections spacing={2}>
                        <ProductSessionCapacityPanel />
                        <ProductAccountSecurity />
                        <ProductDevicesPanel hideWhenEmpty />
                        <ProductAccountMenu />
                    </Stack>
                </MobileSettingsRoute>
            </Stack>
            )}
            {desktop && (
                <Box sx={{ flex: "0 0 auto", mx: -3 }}>
                    <DesktopShortcutBar
                            groups={[
                                {
                                    label: "Navigate",
                                    slots: tab === "settings" ? [
                                        { shortcut: "J/K", label: "Field", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: "H/L", label: "Choice", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: ENTER_LABEL, label: "Apply", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                    ] : [
                                        { shortcut: "H/K", label: "Previous", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: "J/L", label: "Next", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: ENTER_LABEL, label: "Activate", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                    ],
                                },
                                {
                                    label: "Page",
                                    slots: [
                                        { shortcut: "Ctrl+D/U", label: "Half", availability: settingsEditableFocus ? "inactive" : "available" },
                                        { shortcut: "Ctrl+F/B", label: "Full", availability: settingsEditableFocus ? "inactive" : "available" },
                                        {
                                            shortcut: "G",
                                            label: "Go",
                                            availability: sequentialShortcutAvailability({
                                                scopeAvailable: !settingsEditableFocus,
                                                armed: settingsGoPrefix,
                                                prefix: true,
                                            }),
                                        },
                                        {
                                            shortcut: "G",
                                            label: "Top",
                                            availability: sequentialShortcutAvailability({
                                                scopeAvailable: !settingsEditableFocus,
                                                armed: settingsGoPrefix,
                                                prefix: false,
                                            }),
                                        },
                                        { shortcut: "Shift+G", label: "Bottom", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                    ],
                                },
                                {
                                    label: "Jump",
                                    slots: ([
                                            ["T", "Theme"], ["F", "Reading font"],
                                            ["Z", "Font size"], ["P", "Padding"],
                                            ["R", "Line height"], ["M", "Vim"],
                                        ] as const).map(([shortcut, title]) => ({
                                            shortcut,
                                            title,
                                            availability: settingsShortcutsAvailable && tab === "settings" ? "available" as const : "inactive" as const,
                                        })),
                                },
                                {
                                    label: "Tabs",
                                    slots: [
                                        ...CONTROL_CENTER_TABS.map(({ shortcut, label }) => ({
                                            shortcut,
                                            label,
                                            availability: settingsShortcutsAvailable ? "available" as const : "inactive" as const,
                                        })),
                                        { shortcut: "[/]", label: "Cycle", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        {
                                            shortcut: "Esc",
                                            label: settingsGoPrefix
                                                ? "Cancel prefix"
                                                : settingsEditableFocus
                                                ? "Leave field"
                                                : "Close",
                                        },
                                    ],
                                },
                            ]}
                    />
                </Box>
            )}
        </Sheet>
    );
}
