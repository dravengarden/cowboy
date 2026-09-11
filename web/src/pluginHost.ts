import * as React from "react";
import {
  installPluginRenderers,
  type PluginRendererRegistry,
  type PluginSlotProps,
} from "@cowboy/plugin-api";
import { ProviderSurface } from "./ProviderSurface";
import { ProviderUsage, ProviderUsageActivity } from "./pluginUsage";
import type { ProviderUsageSlotContext } from "./usageLimits";
import {
  type LoginMethodContext,
  LoginMethodFallback,
} from "./auth/ProductLoginPage";

function contextRecord(context: unknown): Record<string, unknown> | null {
  return context != null && typeof context === "object"
    ? context as Record<string, unknown>
    : null;
}

function RetiredLocalAuthenticationRenderer(): null {
  // Keep the old closed SDK table readable during the ownership migration,
  // but never let a Plugin descriptor mount a local security ceremony.
  return null;
}

function LoginOidcRenderer({ context }: PluginSlotProps): unknown {
  if (contextRecord(context)?.kind !== "oidc") {
    throw new TypeError("login-oidc-v1 requires an OIDC context");
  }
  return React.createElement(LoginMethodFallback, {
    context: context as LoginMethodContext,
  });
}

function ProviderSurfaceRenderer({ context }: PluginSlotProps): unknown {
  const row = contextRecord(context);
  if (
    !row ||
    !["provider.setup", "provider.empty", "provider.settings"].includes(
      String(row.kind),
    )
  ) {
    throw new TypeError("provider-surface-v1 requires a lifecycle context");
  }
  const surface = context as React.ComponentProps<typeof ProviderSurface> & {
    kind: string;
  };
  return React.createElement(ProviderSurface, {
    manifest: surface.manifest,
    slot: surface.slot,
    host: surface.host,
    ...(surface.onEffect ? { onEffect: surface.onEffect } : {}),
    ...(surface.blockedCapabilities
      ? { blockedCapabilities: surface.blockedCapabilities }
      : {}),
  });
}

function ProviderUsageRenderer({ context }: PluginSlotProps): unknown {
  if (contextRecord(context)?.kind !== "provider.usage") {
    throw new TypeError("provider-usage-v1 requires a usage context");
  }
  return React.createElement(ProviderUsage, {
    context: context as ProviderUsageSlotContext,
  });
}

function ProviderUsageActivityRenderer({ context }: PluginSlotProps): unknown {
  if (contextRecord(context)?.kind !== "provider.usage") {
    throw new TypeError(
      "provider-usage-activity-v1 requires a usage context",
    );
  }
  return React.createElement(ProviderUsageActivity, {
    context: context as ProviderUsageSlotContext,
  });
}

const COWBOY_RENDERERS: PluginRendererRegistry = {
  "login-password-v1": RetiredLocalAuthenticationRenderer,
  "login-oidc-v1": LoginOidcRenderer,
  "account-passkeys-v1": RetiredLocalAuthenticationRenderer,
  "provider-surface-v1": ProviderSurfaceRenderer,
  "provider-usage-v1": ProviderUsageRenderer,
  "provider-usage-activity-v1": ProviderUsageActivityRenderer,
};

export function installCowboyPluginRenderers(): void {
  installPluginRenderers(COWBOY_RENDERERS);
}
