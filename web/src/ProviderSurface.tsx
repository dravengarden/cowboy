import {
  Alert,
  AlertTitle,
  Button,
  Chip,
  CircularProgress,
  Divider,
  Stack,
  SvgIcon,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
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
  providerEntryForIdentity,
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
  const entry = providerEntryForIdentity(
    entries,
    provider,
    providerVersion,
    providerDigest,
  );
  if (!entry) return <>{fallback}</>;
  const authentication = catalog?.authentications.find((candidate) =>
    candidate.provider_id === provider
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
}: {
  manifest: ProviderUiManifest;
  role?: UiAsset["role"];
  size?: number;
}): React.JSX.Element | null {
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
  if (asset.content.kind === "vector_path") {
    return (
      <SvgIcon
        role="img"
        aria-label={asset.accessible_label}
        viewBox={asset.content.view_box}
        sx={{
          width: size,
          height: size,
          color: asset.content.fill ?? manifest.display.accent,
        }}
      >
        <path
          d={asset.content.path}
          fill={asset.content.fill ?? "currentColor"}
        />
      </SvgIcon>
    );
  }
  return (
    <img
      alt={asset.accessible_label}
      src={`data:${asset.media_type};base64,${asset.content.base64}`}
      width={size}
      height={size}
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
      const direction = node.direction === "responsive"
        ? { xs: "column", sm: "row" } as const
        : node.direction;
      const spacing = { xs: 0.5, sm: 1, md: 2, lg: 3 }[node.gap];
      return (
        <Stack
          direction={direction}
          spacing={spacing}
          alignItems={node.direction === "row" ? "center" : undefined}
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
      const size = { sm: 18, md: 28, lg: 48, fill: 72 }[node.size];
      if (asset.content.kind === "vector_path") {
        return (
          <SvgIcon
            role="img"
            aria-label={asset.accessible_label}
            viewBox={asset.content.view_box}
            sx={{
              width: size,
              height: size,
              color: asset.content.fill ?? manifest.display.accent,
              flexShrink: 0,
            }}
          >
            <path
              d={asset.content.path}
              fill={asset.content.fill ?? "currentColor"}
            />
          </SvgIcon>
        );
      }
      return (
        <img
          alt={asset.accessible_label}
          src={`data:${asset.media_type};base64,${asset.content.base64}`}
          width={size}
          height={size}
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
