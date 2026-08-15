import {
  Alert,
  AlertTitle,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  keyframes,
  Stack,
  SvgIcon,
  Typography,
  useMediaQuery,
  useTheme,
} from "@mui/material";
import type { Theme } from "@mui/material/styles";
import { useEffect, useId, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  assertNever,
  type EffectCapability,
  type EffectSchema,
  evaluateExpression,
  initialProviderState,
  type ProviderHostContext,
  type ProviderState,
  type ProviderUiManifest,
  resolveText,
  type SurfaceSlot,
  transitionProvider,
  type UiAsset,
  type UiNode,
} from "../../packages/provider-ui-sdk/src/index.ts";
import {
  providerPresentationEntry,
  useProviderCatalog,
} from "./providerCatalog";

export function ProviderRuntimeSurface({
  provider,
  providerVersion,
  providerDigest,
  slot,
  fallback = null,
}: {
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  slot: SurfaceSlot;
  fallback?: ReactNode;
}): React.JSX.Element {
  const { catalog } = useProviderCatalog();
  const entries = catalog?.providers ?? [];
  const entry = providerPresentationEntry(
    entries,
    provider,
    providerVersion,
    providerDigest,
  );
  if (!entry) return <>{fallback}</>;
  const authentication =
    catalog?.authentications.find((candidate) =>
      candidate.provider_id === provider
    ) ?? catalog?.authentications.find((candidate) =>
      candidate.authentication_scope === entry.authentication_scope
    );
  return (
    <ProviderSurface
      manifest={entry.manifest}
      slot={slot}
      host={{
        provider_version: entry.provider_version,
        installation_state: "active",
        authentication_state: authentication?.authentication_state ??
          "signed_out",
        distribution_state: authentication?.distribution_state ?? "none",
        machine_name: "",
        error_detail: "",
        installed: true,
        auth_ready: !entry.manifest.authentication.required ||
          authentication?.authentication_state === "ready",
        auth_required: entry.manifest.authentication.required,
        machine_online: true,
        upgrade_available: false,
      }}
    />
  );
}

export function ProviderMark({
  manifest,
  role = "icon",
  size = 24,
  className,
}: {
  manifest: ProviderUiManifest;
  role?: UiAsset["role"];
  size?: number;
  className?: string | undefined;
}): React.JSX.Element | null {
  const theme = useTheme();
  const assetId = role === "logo"
    ? manifest.display.logo_asset
    : role === "icon"
    ? manifest.display.icon_asset
    : undefined;
  const asset = manifest.ui.assets.find((candidate) =>
    (assetId !== undefined && candidate.id === assetId) ||
    candidate.role === role
  );
  if (!asset) return null;
  const markColor = asset.content.kind === "vector_path" &&
      asset.content.gradient === undefined && asset.content.fill === undefined
    ? readableProviderMarkColor(manifest.display.accent, theme)
    : undefined;
  return (
    <ProviderAssetGraphic
      asset={asset}
      size={size}
      className={className}
      {...(markColor ? { sx: { color: markColor } } : {})}
    />
  );
}

/** Keep thin monochrome marks legible on dark surfaces without overriding a
 * Provider-authored explicit fill or gradient. */
function readableProviderMarkColor(
  accent: string,
  theme: Theme,
): string {
  if (theme.palette.mode !== "dark") return accent;
  const match = /^#([0-9a-f]{6})$/i.exec(accent.trim());
  if (!match) return accent;
  const channels = [0, 2, 4].map((offset) =>
    Number.parseInt(match[1]!.slice(offset, offset + 2), 16) / 255
  );
  const luminance = channels
    .map((channel) =>
      channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
    )
    .reduce(
      (sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index]!,
      0,
    );
  return luminance < 0.25 ? theme.palette.common.white : accent;
}

/** Shared safe renderer for signed Provider artwork. */
export function ProviderAssetGraphic({
  asset,
  size,
  className,
  sx,
}: {
  asset: UiAsset;
  size: number;
  className?: string | undefined;
  sx?: Record<string, unknown>;
}): React.JSX.Element {
  const gradientId = `provider-gradient-${useId().replaceAll(":", "")}`;
  if (asset.content.kind === "vector_path") {
    const gradient = asset.content.gradient;
    return (
      <SvgIcon
        className={className}
        role="img"
        aria-label={asset.accessible_label}
        viewBox={asset.content.view_box}
        sx={{
          width: size,
          height: size,
          ...(asset.content.fill ? { color: asset.content.fill } : {}),
          flexShrink: 0,
          ...sx,
        }}
      >
        {gradient
          ? (
            <defs>
              <linearGradient
                id={gradientId}
                x1={`${gradient.x1_percent}%`}
                y1={`${gradient.y1_percent}%`}
                x2={`${gradient.x2_percent}%`}
                y2={`${gradient.y2_percent}%`}
              >
                {gradient.stops.map((stop) => (
                  <stop
                    key={`${stop.offset_percent}:${stop.color}`}
                    offset={`${stop.offset_percent}%`}
                    stopColor={stop.color}
                  />
                ))}
              </linearGradient>
            </defs>
          )
          : null}
        <path
          d={asset.content.path}
          fill={gradient
            ? `url(#${gradientId})`
            : asset.content.fill ?? "currentColor"}
        />
      </SvgIcon>
    );
  }
  return (
    <Box
      component="img"
      className={className}
      alt={asset.accessible_label}
      src={`data:${asset.media_type};base64,${asset.content.base64}`}
      sx={{ width: size, height: size, flexShrink: 0, ...sx }}
    />
  );
}

export function ProviderSurface({
  manifest,
  slot,
  host,
  onEffect,
  blockedCapabilities,
}: {
  manifest: ProviderUiManifest;
  slot: SurfaceSlot;
  host: ProviderHostContext;
  onEffect?: (effect: EffectSchema) => Promise<void>;
  blockedCapabilities?: ReadonlySet<EffectCapability> | undefined;
}): React.JSX.Element {
  const [state, setState] = useState<ProviderState>(() =>
    initialProviderState(manifest)
  );
  const [busyEffect, setBusyEffect] = useState<string | null>(null);
  useEffect(() => {
    setState(initialProviderState(manifest));
    setBusyEffect(null);
  }, [manifest]);
  const assets = useMemo(
    () => new Map(manifest.ui.assets.map((asset) => [asset.id, asset])),
    [manifest.ui.assets],
  );
  const emit = async (
    node: Extract<UiNode, { component: "button" }>,
  ): Promise<void> => {
    const transitioned = transitionProvider(manifest, state, node.emit);
    setState(transitioned.state);
    if (!transitioned.effect || !onEffect) return;
    setBusyEffect(transitioned.effect.id);
    try {
      await onEffect(transitioned.effect);
      setState((current) =>
        transitionProvider(manifest, current, {
          message: transitioned.effect?.success_message ??
            "operation_succeeded",
          payload: {},
        }).state
      );
    } catch (cause) {
      const detail = cause instanceof Error
        ? cause.message
        : "Provider operation failed";
      setState((current) =>
        transitionProvider(manifest, current, {
          message: transitioned.effect?.failure_message ?? "operation_failed",
          payload: { detail },
        }).state
      );
      throw cause;
    } finally {
      setBusyEffect(null);
    }
  };
  if (slot === "loading" && manifest.ui.schema_version === 1) {
    return <LegacyProviderActivity />;
  }
  return (
    <ProviderNode
      node={manifest.ui.surfaces[slot]}
      manifest={manifest}
      host={host}
      state={state}
      assets={assets}
      busyEffect={busyEffect}
      blockedCapabilities={blockedCapabilities}
      emit={emit}
      path={slot}
    />
  );
}

function ProviderNode({
  node,
  manifest,
  host,
  state,
  assets,
  busyEffect,
  blockedCapabilities,
  emit,
  path,
}: {
  node: UiNode;
  manifest: ProviderUiManifest;
  host: ProviderHostContext;
  state: ProviderState;
  assets: ReadonlyMap<string, UiAsset>;
  busyEffect: string | null;
  blockedCapabilities: ReadonlySet<EffectCapability> | undefined;
  emit: (node: Extract<UiNode, { component: "button" }>) => Promise<void>;
  path: string;
}): React.JSX.Element | null {
  switch (node.component) {
    case "stack": {
      if (!evaluateExpression(node.visible_when, state, host)) return null;
      const legacyResponsive = manifest.ui.schema_version === 1 &&
        node.direction === "responsive";
      const direction = node.direction === "responsive"
        ? legacyResponsive ? "row" : { xs: "column", sm: "row" } as const
        : node.direction;
      const wrap = node.wrap === true || legacyResponsive;
      const spacing = { xs: 0.5, sm: 1, md: 2, lg: 3 }[node.gap];
      return (
        <Stack
          direction={direction}
          spacing={spacing}
          alignItems={node.direction === "row" || legacyResponsive
            ? "center"
            : undefined}
          flexWrap={wrap ? "wrap" : "nowrap"}
          useFlexGap={wrap}
          sx={{ minWidth: 0 }}
        >
          {node.children.map((child, index) => (
            <ProviderNode
              key={`${path}.${index}`}
              node={child}
              manifest={manifest}
              host={host}
              state={state}
              assets={assets}
              busyEffect={busyEffect}
              blockedCapabilities={blockedCapabilities}
              emit={emit}
              path={`${path}.${index}`}
            />
          ))}
        </Stack>
      );
    }
    case "text":
      return (
        <Typography
          variant={node.variant === "title"
            ? "subtitle1"
            : node.variant === "caption"
            ? "caption"
            : "body2"}
          color={toneColor(node.tone)}
          component="span"
          sx={{
            overflowWrap: "anywhere",
            ...(node.variant === "title" ? { fontWeight: 720 } : {}),
            ...(node.variant === "code" ? { fontFamily: "monospace" } : {}),
          }}
        >
          {resolveText(node.value, state, host)}
        </Typography>
      );
    case "asset": {
      const asset = assets.get(node.asset);
      if (!asset) return null;
      const size = { sm: 18, md: 24, lg: 40, fill: 64 }[node.size];
      return (
        <ProviderAssetGraphic
          asset={asset}
          size={size}
        />
      );
    }
    case "badge":
      return (
        <Chip
          size="small"
          label={resolveText(node.label, state, host)}
          color={toneChip(node.tone)}
          variant="outlined"
        />
      );
    case "progress":
      return (
        <Stack direction="row" spacing={1} alignItems="center">
          <CircularProgress size={18} />
          <Typography variant="body2">
            {resolveText(node.label, state, host)}
          </Typography>
        </Stack>
      );
    case "activity":
      return (
        <ProviderActivity
          node={node}
          manifest={manifest}
          host={host}
          state={state}
          assets={assets}
        />
      );
    case "alert":
      return (
        <Alert severity={toneAlert(node.tone)}>
          <AlertTitle>{resolveText(node.title, state, host)}</AlertTitle>
          {resolveText(node.body, state, host)}
        </Alert>
      );
    case "divider":
      return <Divider flexItem />;
    case "button": {
      const effectId = manifest.logic.reducers.find((rule) =>
        rule.message === node.emit.message
      )?.effect;
      const effect = manifest.logic.effects.find((candidate) =>
        candidate.id === effectId
      );
      const busy = effectId !== undefined && effectId === busyEffect;
      const blocked = effect !== undefined &&
        blockedCapabilities?.has(effect.capability) === true;
      return (
        <Button
          size="small"
          variant={node.style === "primary" ? "contained" : "outlined"}
          color={node.style === "destructive" ? "error" : "primary"}
          disabled={busy || blocked ||
            !evaluateExpression(node.enabled_when, state, host)}
          startIcon={busy ? <CircularProgress size={14} /> : undefined}
          onClick={() => void emit(node).catch(() => undefined)}
        >
          {resolveText(node.label, state, host)}
        </Button>
      );
    }
    default:
      return assertNever(node);
  }
}

const activityShimmer = keyframes`to { background-position: -200% 0; }`;
const activityFade = keyframes`
  0%, 100% { opacity: 0.34; }
  14%, 86% { opacity: 1; }
`;
const assetSignalMotion = keyframes`
  from { -webkit-mask-position: 120% 0; mask-position: 120% 0; }
  to { -webkit-mask-position: -20% 0; mask-position: -20% 0; }
`;
const terminalPromptMotion = keyframes`
  0%, 100% { transform: translateX(0); opacity: 0.55; }
  50% { transform: translateX(1.5px); opacity: 1; }
`;
const terminalCaretMotion = keyframes`
  0%, 45% { opacity: 1; }
  55%, 100% { opacity: 0.24; }
`;
const assetPulseMotion = keyframes`
  0%, 100% { transform: scale(0.92); opacity: 0.58; }
  50% { transform: scale(1); opacity: 1; }
`;

function LegacyProviderActivity(): React.JSX.Element {
  return (
    <Stack
      role="status"
      aria-label="Provider is thinking"
      direction="row"
      spacing={0.8}
      alignItems="center"
      sx={{ py: 0.5, alignSelf: "flex-start", color: "text.secondary" }}
    >
      <CircularProgress size={16} thickness={5} color="inherit" aria-hidden />
      <Typography variant="caption" aria-hidden>
        Thinking…
      </Typography>
    </Stack>
  );
}

function ProviderActivity({
  node,
  manifest,
  host,
  state,
  assets,
}: {
  node: Extract<UiNode, { component: "activity" }>;
  manifest: ProviderUiManifest;
  host: ProviderHostContext;
  state: ProviderState;
  assets: ReadonlyMap<string, UiAsset>;
}): React.JSX.Element {
  const theme = useTheme();
  const reducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [frameIndex, setFrameIndex] = useState(0);
  const [phraseIndex, setPhraseIndex] = useState(0);
  const frames = node.indicator.kind === "glyph_cycle"
    ? node.indicator.frames
    : [];
  const frameInterval = node.indicator.kind === "glyph_cycle"
    ? node.indicator.interval_ms
    : 0;
  useEffect(() => {
    if (reducedMotion || frames.length < 2 || frameInterval === 0) {
      return undefined;
    }
    const timer = globalThis.setInterval(
      () => setFrameIndex((current) => current + 1),
      frameInterval,
    );
    return () => globalThis.clearInterval(timer);
  }, [frameInterval, frames.length, reducedMotion]);
  const phrases = node.label.kind === "phrase_cycle" ? node.label.phrases : [];
  const phraseInterval = node.label.kind === "phrase_cycle"
    ? node.label.interval_ms
    : 0;
  useEffect(() => {
    if (reducedMotion || phrases.length < 2 || phraseInterval === 0) {
      return undefined;
    }
    const timer = globalThis.setInterval(
      () => setPhraseIndex((current) => current + 1),
      phraseInterval,
    );
    return () => globalThis.clearInterval(timer);
  }, [phraseInterval, phrases.length, reducedMotion]);

  const indicator = (() => {
    switch (node.indicator.kind) {
      case "progress_ring":
        return (
          <CircularProgress
            size={16}
            thickness={5}
            color="inherit"
            aria-hidden
          />
        );
      case "glyph_cycle":
        return (
          <Box
            component="span"
            aria-hidden
            sx={{
              width: 16,
              flexShrink: 0,
              textAlign: "center",
              color: manifest.display.accent,
              fontSize: 15,
              fontWeight: 700,
              lineHeight: 1,
              fontFamily:
                "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
            }}
          >
            {node.indicator.frames[
              reducedMotion ? 0 : frameIndex % node.indicator.frames.length
            ]}
          </Box>
        );
      case "terminal_prompt":
        return (
          <Box
            component="svg"
            viewBox="0 0 18 18"
            aria-hidden
            sx={{
              width: 17,
              height: 17,
              display: "block",
              flexShrink: 0,
              overflow: "visible",
              color: manifest.display.accent,
              "& .provider-terminal-prompt": {
                animation: reducedMotion
                  ? "none"
                  : `${terminalPromptMotion} ${node.indicator.interval_ms}ms ease-in-out infinite`,
              },
              "& .provider-terminal-caret": {
                animation: reducedMotion
                  ? "none"
                  : `${terminalCaretMotion} ${node.indicator.interval_ms}ms ease-in-out infinite`,
              },
            }}
          >
            <path
              className="provider-terminal-prompt"
              d="M3.5 5.5 7 9l-3.5 3.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <path
              className="provider-terminal-caret"
              d="M9.5 12.5h5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
            />
          </Box>
        );
      case "asset_signal": {
        const asset = assets.get(node.indicator.asset);
        if (!asset) return <CircularProgress size={16} aria-hidden />;
        return (
          <Box
            component="span"
            aria-hidden
            sx={{
              width: 18,
              height: 18,
              position: "relative",
              display: "inline-block",
              flexShrink: 0,
              "& .provider-signal-base": {
                opacity: reducedMotion ? 0.82 : 0.28,
              },
              "& .provider-signal-sweep": reducedMotion
                ? { display: "none" }
                : {
                  WebkitMaskImage:
                    "linear-gradient(115deg, transparent 32%, #000 46%, #000 54%, transparent 68%)",
                  maskImage:
                    "linear-gradient(115deg, transparent 32%, #000 46%, #000 54%, transparent 68%)",
                  WebkitMaskSize: "300% 100%",
                  maskSize: "300% 100%",
                  WebkitMaskRepeat: "no-repeat",
                  maskRepeat: "no-repeat",
                  animation:
                    `${assetSignalMotion} ${node.indicator.interval_ms}ms ease-in-out infinite`,
                },
            }}
          >
            <Box
              className="provider-signal-base"
              sx={{ position: "absolute", inset: 0 }}
            >
              <ProviderAssetGraphic
                asset={asset}
                size={18}
              />
            </Box>
            <Box
              className="provider-signal-sweep"
              sx={{ position: "absolute", inset: 0 }}
            >
              <ProviderAssetGraphic
                asset={asset}
                size={18}
              />
            </Box>
          </Box>
        );
      }
      case "asset_pulse": {
        const asset = assets.get(node.indicator.asset);
        if (!asset) return <CircularProgress size={16} aria-hidden />;
        return (
          <Box
            component="span"
            aria-hidden
            sx={{
              width: 18,
              height: 18,
              display: "inline-flex",
              animation: reducedMotion
                ? "none"
                : `${assetPulseMotion} ${node.indicator.interval_ms}ms ease-in-out infinite`,
            }}
          >
            <ProviderAssetGraphic
              asset={asset}
              size={18}
            />
          </Box>
        );
      }
      default:
        return assertNever(node.indicator);
    }
  })();
  const label = node.label.kind === "text"
    ? resolveText(node.label.value, state, host)
    : `${
      node.label.phrases[phraseIndex % node.label.phrases.length]
    }${node.label.suffix}`;
  const effect = node.label.effect;
  const muted = theme.palette.text.secondary;
  return (
    <Stack
      role="status"
      aria-label={node.accessible_label}
      direction="row"
      spacing={0.8}
      alignItems="center"
      sx={{ py: 0.5, alignSelf: "flex-start", color: "text.secondary" }}
    >
      {indicator}
      <Typography
        key={node.label.kind === "phrase_cycle" ? phraseIndex : undefined}
        aria-hidden
        variant="caption"
        sx={{
          fontWeight: 550,
          letterSpacing: "0.015em",
          ...(effect === "shimmer"
            ? {
              background:
                `linear-gradient(100deg, ${muted} 0%, ${muted} 24%, ${manifest.display.accent} 44%, ${manifest.display.secondary_accent} 56%, ${muted} 74%, ${muted} 100%)`,
              backgroundSize: "220% 100%",
              WebkitBackgroundClip: "text",
              backgroundClip: "text",
              color: "transparent",
              animation: reducedMotion
                ? "none"
                : `${activityShimmer} 3.2s linear infinite`,
            }
            : effect === "fade"
            ? {
              animation: reducedMotion
                ? "none"
                : `${activityFade} ${
                  Math.max(1200, phraseInterval || 2400)
                }ms ease-in-out infinite`,
            }
            : {}),
          ...(reducedMotion && effect === "shimmer"
            ? {
              background: "none",
              color: muted,
              WebkitTextFillColor: muted,
            }
            : {}),
        }}
      >
        {label}
      </Typography>
    </Stack>
  );
}

function toneColor(
  tone: UiNode extends never ? never : ToneOrUndefined,
): string | undefined {
  if (!tone || tone === "neutral") return "text.secondary";
  return `${tone}.main`;
}

type ToneOrUndefined =
  | "neutral"
  | "primary"
  | "success"
  | "warning"
  | "error"
  | undefined;

function toneChip(
  tone: Exclude<ToneOrUndefined, undefined>,
): "default" | "primary" | "success" | "warning" | "error" {
  return tone === "neutral" ? "default" : tone;
}

function toneAlert(
  tone: Exclude<ToneOrUndefined, undefined>,
): "info" | "success" | "warning" | "error" {
  if (tone === "success" || tone === "warning" || tone === "error") return tone;
  return "info";
}
