/* Passkey account.panel slot. Renders the host-owned panel through the kit. */
function host() {
  const value = globalThis.__COWBOY_PLUGIN_HOST;
  if (!value) throw new Error("Cowboy plugin host is not installed");
  return value;
}

export default function PasskeyAccountPanel() {
  const { React, components } = host();
  const Panel = components.PasskeysPanel;
  if (typeof Panel !== "function") {
    throw new Error("PasskeysPanel host component is not ready");
  }
  return React.createElement(Panel);
}

export const slots = { "account.panel": PasskeyAccountPanel };
