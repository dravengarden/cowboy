import { createElement, type ReactNode } from "react";
import { ProviderSurface } from "./ProviderSurface";
import { ProviderUsage, ProviderUsageActivity } from "./pluginUsage";
import { PluginSlot as ObservedPluginSlot } from "./pluginHost/PluginSlot";
import type { HostRendererId } from "./pluginHost/identity.ts";
import type { PluginSlotProps } from "./pluginHost/slotContracts.ts";

type ProviderSlotProps = Exclude<PluginSlotProps, { slot: "login.method" }>;

/** Provider views load only when their domain mounts. The authentication tree
 * never imports Provider panels, application stores or app-shell sheets. */
export function PluginSlot(
  props: ProviderSlotProps & {
    children?: ReactNode;
    placeholder?: ReactNode;
  },
): ReactNode {
  return createElement(ObservedPluginSlot, {
    ...props,
    render: (renderer) =>
      createElement(CowboyPluginRenderer, {
        ...props,
        renderer,
        fallback: props.children ?? null,
      }),
  });
}

/** Closed implementation in the Web artifact. No renderer registration,
 * unknown context cast, Plugin JS loader or native dispatch. */
export function CowboyPluginRenderer(
  props: ProviderSlotProps & { renderer: HostRendererId; fallback: ReactNode },
): ReactNode {
  switch (props.slot) {
    case "provider.card":
      // The card shell is core-owned. The old lifecycle renderer threw when
      // given this card context; that exception was not a useful extension.
      return props.fallback;
    case "provider.setup":
    case "provider.empty":
    case "provider.settings":
      return props.renderer === "provider-surface-v1"
        ? createElement(ProviderSurface, {
          manifest: props.context.manifest,
          ownerKey: props.context.ownerKey,
          slot: props.context.slot,
          host: props.context.host,
          blockedCapabilities: props.context.blockedCapabilities,
          ...(props.context.onEffect
            ? { onEffect: props.context.onEffect }
            : {}),
        })
        : props.fallback;
    case "provider.usage":
      if (props.renderer === "provider-usage-v1") {
        return createElement(ProviderUsage, { context: props.context });
      }
      if (props.renderer === "provider-usage-activity-v1") {
        return createElement(ProviderUsageActivity, { context: props.context });
      }
      return props.fallback;
  }
}
