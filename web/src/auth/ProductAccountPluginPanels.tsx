import { Fragment } from "react";
import { PluginSlot } from "@cowboy/plugin-api";
import { useProductAuth } from "./ProductAuthGate";

/** Mount every signed authentication plugin that claims an account panel. */
export function ProductAccountPluginPanels(): React.JSX.Element {
  const { hostPlugins } = useProductAuth();
  const panels = hostPlugins.filter((host) =>
    host.slots.includes("account.panel")
  );
  return (
    <>
      {panels.map((host) => (
        <Fragment key={host.id}>
          <PluginSlot pluginId={host.id} slot="account.panel" />
        </Fragment>
      ))}
    </>
  );
}
