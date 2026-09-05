import {
  Component,
  type ErrorInfo,
  type ReactNode,
  useEffect,
  useState,
} from "react";
import {
  loadPluginSlot,
  type PluginSlotComponent,
  type PluginSlotId,
  type PluginSlotProps,
} from "./types.ts";

interface BoundaryProps {
  fallback: ReactNode;
  children: ReactNode;
}

interface BoundaryState {
  failed: boolean;
}

export class PluginSlotBoundary
  extends Component<BoundaryProps, BoundaryState> {
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
  pluginVersion,
  artifactDigest,
  slot,
  context,
  children,
  placeholder,
}: {
  pluginId: string;
  /** Exact trusted release identity. Both fields are required together. */
  pluginVersion?: string;
  artifactDigest?: string;
  slot: PluginSlotId;
  context?: unknown;
  children?: ReactNode;
  /** Shown while the signed renderer declaration resolves. Defaults to `children`. */
  placeholder?: ReactNode;
}) {
  const [renderer, setRenderer] = useState<
    PluginSlotComponent | null | undefined
  >(undefined);
  useEffect(() => {
    let cancelled = false;
    setRenderer(undefined);
    void loadPluginSlot(
      pluginId,
      slot,
      pluginVersion,
      artifactDigest,
    ).then((loaded) => {
      if (!cancelled) setRenderer(() => loaded);
    });
    return () => {
      cancelled = true;
    };
  }, [pluginId, pluginVersion, artifactDigest, slot]);
  const core = children ?? null;
  if (renderer === undefined) {
    return (
      <div style={{ display: "contents" }}>
        {placeholder === undefined ? core : placeholder}
      </div>
    );
  }
  if (renderer === null) {
    return <div style={{ display: "contents" }}>{core}</div>;
  }
  const Renderer = renderer as (props: PluginSlotProps) => ReactNode;
  return (
    <div style={{ display: "contents" }}>
      <PluginSlotBoundary fallback={core}>
        <Renderer
          pluginId={pluginId}
          {...(pluginVersion === undefined ? {} : { pluginVersion })}
          {...(artifactDigest === undefined ? {} : { artifactDigest })}
          slot={slot}
          context={context}
        />
      </PluginSlotBoundary>
    </div>
  );
}
