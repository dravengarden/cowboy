import type {
  EffectCapability,
  ProviderHostContext,
  ProviderUiManifest,
} from "@cowboy/provider-ui";
import type { OidcLoginContext } from "../auth/ProductLoginPage.tsx";
import type { ProviderUsageSlotContext } from "../usageLimits.ts";
import type { PluginHostRelease } from "./identity.ts";
import type { ProviderUiEffectHandler } from "../providerUiOwner.ts";

export type LifecycleSlot = "setup" | "empty" | "settings";
export interface LifecycleContext {
  readonly providerId: string;
  readonly ownerKey: string;
  readonly manifest: ProviderUiManifest;
  readonly host: ProviderHostContext;
  readonly blockedCapabilities?: ReadonlySet<EffectCapability> | undefined;
  readonly onEffect?: ProviderUiEffectHandler;
}
export type LifecycleSlotInput = {
  [S in LifecycleSlot]: {
    readonly slot: `provider.${S}`;
    readonly context: LifecycleContext & {
      readonly kind: `provider.${S}`;
      readonly slot: S;
    };
  };
}[LifecycleSlot];
/** The slot discriminant selects the whole context. Callbacks are trusted
 * core-local references; none are decoded from Plugin data. */
export type PluginSlotInput =
  | { readonly slot: "login.method"; readonly context: OidcLoginContext }
  | {
    readonly slot: "provider.card";
    readonly context: {
      readonly kind: "provider.card";
      readonly providerId: string;
    };
  }
  | {
    readonly slot: "provider.usage";
    readonly context: ProviderUsageSlotContext;
  }
  | LifecycleSlotInput;
export type PluginSlotProps = PluginSlotInput & {
  readonly pluginId: string;
  readonly release?: PluginHostRelease;
};
export function lifecycleSlotInput(
  slot: LifecycleSlot,
  context: LifecycleContext,
): LifecycleSlotInput {
  switch (slot) {
    case "setup":
      return {
        slot: "provider.setup",
        context: { ...context, kind: "provider.setup", slot },
      };
    case "empty":
      return {
        slot: "provider.empty",
        context: { ...context, kind: "provider.empty", slot },
      };
    case "settings":
      return {
        slot: "provider.settings",
        context: { ...context, kind: "provider.settings", slot },
      };
  }
}
