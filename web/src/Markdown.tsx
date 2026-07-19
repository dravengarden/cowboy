// Lazy wrapper around `./MarkdownImpl`. Splits the markdown parser +
// syntax-highlighter into their own chunks (see vite.config.ts), so the
// initial page load stays small on mobile. The Suspense fallback shows the
// raw markdown text in a monospace pre so streaming still feels live during
// the (one-time) chunk fetch.

import { Component, type ErrorInfo, lazy, memo, type ReactNode, Suspense } from "react";
import { Box } from "@mui/material";

const MarkdownImpl = lazy(() => import("./MarkdownImpl"));

class MarkdownBoundary extends Component<
  { children: ReactNode; text: string; invert: boolean },
  { failed: boolean }
> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    console.warn("Markdown rendering fell back to source", error, info.componentStack);
  }

  override componentDidUpdate(previous: Readonly<{ children: ReactNode; text: string; invert: boolean }>): void {
    if (this.state.failed && previous.text !== this.props.text) this.setState({ failed: false });
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return <MarkdownSourceFallback text={this.props.text} invert={this.props.invert} />;
  }
}

function MarkdownSourceFallback({ text, invert }: { text: string; invert: boolean }): React.JSX.Element {
  return (
    <Box
      component="pre"
      data-markdown-fallback
      sx={{
        m: 0,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        fontSize: "0.875rem",
        whiteSpace: "pre-wrap",
        overflowWrap: "anywhere",
        opacity: invert ? 1 : 0.85,
      }}
    >
      {text}
    </Box>
  );
}

export const Markdown = memo(function Markdown({
  text,
  invert = false,
  centerCopy = false,
  touchWrap = false,
}: {
  text: string;
  invert?: boolean;
  /** Vertically center the copy control in compact single-line code cards. */
  centerCopy?: boolean;
  /** Soft-wrap fenced code on touch surfaces. */
  touchWrap?: boolean;
}): React.JSX.Element {
  return (
    <MarkdownBoundary text={text} invert={invert}>
      <Suspense fallback={<MarkdownSourceFallback text={text} invert={invert} />}>
        <MarkdownImpl text={text} invert={invert} centerCopy={centerCopy} touchWrap={touchWrap} />
      </Suspense>
    </MarkdownBoundary>
  );
});
