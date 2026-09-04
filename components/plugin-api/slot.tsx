import {
  Component,
  type ErrorInfo,
  type JSX,
  type ReactNode,
  useEffect,
  useState,
} from "react";
import {
  type PluginSlotComponent,
  type PluginSlotId,
  loadPluginSlot,
} from "./types.ts";

interface BoundaryProps {
  fallback: ReactNode;
  children: ReactNode;
}

interface BoundaryState {
  failed: boolean;
}

export class PluginSlotBoundary extends Component<BoundaryProps, BoundaryState> {
  override state: BoundaryState = { failed: false };

  static getDerivedStateFromError(): BoundaryState {
    return { failed: true };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("plugin slot crashed", error, info.componentStack);
  }

  override componentDidUpdate(prevProps: BoundaryProps): void {
    if (prevProps.children !== this.props.children && this.state.failed) {
      this.setState({ failed: false });
    }
  }

  override render(): ReactNode {
    if (this.state.failed) return this.props.fallback;
    return this.props.children;
  }
}

export function PluginSlot({
  pluginId,
  slot,
  context,
  children,
  placeholder,
}: {
  pluginId: string;
  slot: PluginSlotId;
  context?: unknown;
  children?: ReactNode;
  /** Shown while the slot module loads. Defaults to `children`. */
  placeholder?: ReactNode;
}): JSX.Element {
  const [module, setModule] = useState<PluginSlotComponent | null>(null);
  useEffect(() => {
    let cancelled = false;
    setModule(null);
    void loadPluginSlot(pluginId, slot).then((loaded) => {
      if (!cancelled) setModule(() => loaded);
    });
    return () => {
      cancelled = true;
    };
  }, [pluginId, slot]);
  const core = children ?? null;
  if (!module) {
    return (
      <div style={{ display: "contents" }}>
        {placeholder === undefined ? core : placeholder}
      </div>
    );
  }
  const Module = module;
  return (
    <div style={{ display: "contents" }}>
      <PluginSlotBoundary fallback={core}>
        <Module pluginId={pluginId} slot={slot} context={context} />
      </PluginSlotBoundary>
    </div>
  );
}
