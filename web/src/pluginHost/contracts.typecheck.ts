/** Compiled by the production strict TypeScript gate, never imported at runtime.
 * Unused @ts-expect-error directives fail that gate if a boundary is widened. */
import type { OidcLoginContext } from "../auth/ProductLoginPage.tsx";
import type { ProviderUsageSlotContext } from "../usageLimits.ts";
import type { PluginHostRelease } from "./identity.ts";
import {
  type LifecycleContext,
  lifecycleSlotInput,
  type PluginSlotProps,
} from "./slotContracts.ts";

function slot(_props: PluginSlotProps): void {}
function release(_release: PluginHostRelease): void {}

export function verifyCoreSlotTypes(
  oidc: OidcLoginContext,
  usage: ProviderUsageSlotContext,
  lifecycle: LifecycleContext,
): void {
  slot({ pluginId: "example", slot: "login.method", context: oidc });
  slot({ pluginId: "example", slot: "provider.usage", context: usage });
  slot({ pluginId: "example", ...lifecycleSlotInput("settings", lifecycle) });
  release({
    kind: "exact",
    version: "1.0.0",
    artifactDigest: "sha256:validated-at-runtime",
  });
  const wrongDomain = {
    pluginId: "example",
    slot: "provider.usage" as const,
    context: oidc,
  };
  // @ts-expect-error A usage slot cannot accept an OIDC context.
  slot(wrongDomain);
  const missingContext = {
    pluginId: "example",
    slot: "provider.usage" as const,
  };
  // @ts-expect-error No unknown/optional context escape hatch.
  slot(missingContext);
  const wrongLifecycle = {
    pluginId: "example",
    slot: "provider.setup" as const,
    context: {
      ...lifecycle,
      kind: "provider.settings" as const,
      slot: "settings" as const,
    },
  };
  // @ts-expect-error Lifecycle slot and context must agree, not just share a union.
  slot(wrongLifecycle);
  const wrongSurface = {
    pluginId: "example",
    slot: "provider.empty" as const,
    context: {
      ...lifecycle,
      kind: "provider.empty" as const,
      slot: "setup" as const,
    },
  };
  // @ts-expect-error Install and login surfaces cannot be interchanged.
  slot(wrongSurface);
  const native = {
    pluginId: "example",
    slot: "account.panel" as const,
    context: usage,
  };
  // @ts-expect-error Local account security is not a Plugin mount surface.
  slot(native);
  const password = {
    pluginId: "example",
    slot: "login.method" as const,
    context: { kind: "password" as const },
  };
  // @ts-expect-error Plugin login presentation accepts external OIDC only.
  slot(password);
  const incomplete = { kind: "exact" as const, version: "1.0.0" };
  // @ts-expect-error Exact identity always pairs version and digest.
  release(incomplete);
  const wrongEffect = {
    ...lifecycle,
    onEffect: async (_request: { arbitrary: string }) => {},
  };
  // @ts-expect-error Lifecycle callbacks consume the closed effect IR.
  lifecycleSlotInput("settings", wrongEffect);
}
