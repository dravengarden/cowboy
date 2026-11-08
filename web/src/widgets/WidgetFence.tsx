// The read-only inline carrier (docs/widgets.md §6.4): a ```cowboy-widget
// fence renders as a widget once its source is complete and valid. While it
// streams, or when it is invalid or fails to render, the ordinary code block
// stays — readable source is never replaced by a broken widget.

import { Component, lazy, type ReactNode, Suspense, useMemo } from "react";
import { Typography } from "@mui/material";
import { parseWidgetSource } from "./widgetDocument";

const WidgetView = lazy(() => import("./WidgetView"));

class WidgetBoundary extends Component<
  { children: ReactNode; fallback: ReactNode; source: string },
  { failed: boolean }
> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override componentDidCatch(error: Error): void {
    console.warn("Widget rendering fell back to source", error);
  }

  override componentDidUpdate(previous: Readonly<{ source: string }>): void {
    if (this.state.failed && previous.source !== this.props.source)
      this.setState({ failed: false });
  }

  override render(): ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

export function WidgetFence({
  source,
  pending,
  fallback,
}: {
  source: string;
  pending: boolean;
  fallback: ReactNode;
}): React.JSX.Element {
  const parsed = useMemo(() => (pending ? null : parseWidgetSource(source)), [pending, source]);
  if (!parsed) return <>{fallback}</>;
  if (!parsed.ok) {
    return (
      <>
        <Typography
          variant="caption"
          color="text.secondary"
          data-widget-invalid=""
          sx={{ display: "block", mt: 1 }}
        >
          {`Widget preview unavailable (${parsed.error}). Showing the source.`}
        </Typography>
        {fallback}
      </>
    );
  }
  return (
    <WidgetBoundary fallback={fallback} source={source}>
      <Suspense fallback={fallback}>
        <WidgetView document={parsed.document} />
      </Suspense>
    </WidgetBoundary>
  );
}
