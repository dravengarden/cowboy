import {
  Component,
  type ErrorInfo,
  type ReactNode,
  useSyncExternalStore,
} from "react";
import type { HostRendererId } from "./identity.ts";
import { webPluginHosts } from "./inventory.ts";
import type { PluginSlotProps } from "./slotContracts.ts";

interface BoundaryProps {
  readonly fallback: ReactNode;
  readonly children: ReactNode;
}
class PluginSlotBoundary extends Component<BoundaryProps, { failed: boolean }> {
  override state = { failed: false };
  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }
  override componentDidCatch(_error: Error, _info: ErrorInfo): void {
    console.warn("Cowboy Plugin presentation failed");
  }
  override render(): ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}
/** A view borrows the core inventory. Replacements re-resolve mounted views;
 * no async closure can install yesterday's renderer. */
export function PluginSlot(
  props: PluginSlotProps & {
    children?: ReactNode;
    placeholder?: ReactNode;
    /** Compiled domain-core presentation, never a callback supplied by a Plugin. */
    render: (renderer: HostRendererId) => ReactNode;
  },
): ReactNode {
  useSyncExternalStore(
    webPluginHosts.subscribe,
    webPluginHosts.getSnapshot,
    webPluginHosts.getSnapshot,
  );
  const selection = webPluginHosts.resolve(
    props.pluginId,
    props.slot,
    props.release,
  );
  const fallback = props.children ?? null;
  if (selection.kind !== "ready") {
    return (
      <div style={{ display: "contents" }}>
        {selection.kind === "pending" && props.placeholder !== undefined
          ? props.placeholder
          : fallback}
      </div>
    );
  }
  return (
    <div style={{ display: "contents" }}>
      <PluginSlotBoundary key={selection.key} fallback={fallback}>
        <DomainRenderer renderer={selection.renderer} render={props.render} />
      </PluginSlotBoundary>
    </div>
  );
}

/** Keep the callback inside the error boundary's child render, not its parent. */
function DomainRenderer({ renderer, render }: {
  renderer: HostRendererId;
  render: (renderer: HostRendererId) => ReactNode;
}): ReactNode {
  return render(renderer);
}
