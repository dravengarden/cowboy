// Lazy wrapper around `./MarkdownImpl`. Splits the markdown parser +
// syntax-highlighter into their own chunks (see vite.config.ts), so the
// initial page load stays small on mobile. The Suspense fallback shows the
// raw markdown text in a monospace pre so streaming still feels live during
// the (one-time) chunk fetch.

import { lazy, Suspense } from "react";
import { Box } from "@mui/material";

const MarkdownImpl = lazy(() => import("./MarkdownImpl"));

export function Markdown({
  text,
  invert = false,
}: {
  text: string;
  invert?: boolean;
}): React.JSX.Element {
  return (
    <Suspense
      fallback={
        <Box
          component="pre"
          sx={{
            m: 0,
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: "0.875rem",
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
            opacity: invert ? 1 : 0.85,
          }}
        >
          {text}
        </Box>
      }
    >
      <MarkdownImpl text={text} invert={invert} />
    </Suspense>
  );
}
