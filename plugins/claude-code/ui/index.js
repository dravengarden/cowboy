/* Shared agent host UI. Host owns presentation components; this module maps slots. */
function host() {
  const value = globalThis.__COWBOY_PLUGIN_HOST;
  if (!value) throw new Error("Cowboy plugin host is not installed");
  return value;
}

export default function ProviderUsageSlot(props) {
  const context = props.context;
  if (!context || context.kind !== "provider.usage") {
    throw new Error("provider.usage context is missing");
  }
  const { React, components } = host();
  if (typeof components.ProviderUsage !== "function") {
    throw new Error("host ProviderUsage component is missing");
  }
  return React.createElement(components.ProviderUsage, {
    context,
    pluginId: props.pluginId,
  });
}

export function ProviderLifecycleSlot(props) {
  const context = props.context;
  if (
    !context ||
    (context.kind !== "provider.setup" &&
      context.kind !== "provider.empty" &&
      context.kind !== "provider.settings")
  ) {
    throw new Error("provider lifecycle context is missing");
  }
  const { React, components } = host();
  if (typeof components.ProviderSurface !== "function") {
    throw new Error("host ProviderSurface component is missing");
  }
  return React.createElement(components.ProviderSurface, {
    manifest: context.manifest,
    slot: context.slot,
    host: context.host,
    blockedCapabilities: context.blockedCapabilities,
    onEffect: context.onEffect,
  });
}

export const slots = {
  "provider.usage": ProviderUsageSlot,
  "provider.setup": ProviderLifecycleSlot,
  "provider.empty": ProviderLifecycleSlot,
  "provider.settings": ProviderLifecycleSlot,
};
