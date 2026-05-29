// Markdown renderer used by message bubbles. GitHub-Flavored Markdown
// (tables, strikethrough, task lists) + syntax-highlighted code fences via
// `react-syntax-highlighter` (Prism Light + per-language async loading).
//
// Mobile-first concerns:
// - Code blocks `overflow-x: auto` so they never stretch the bubble width.
// - Tables get `display: block; overflow-x: auto` for the same reason.
// - Long URLs `word-break` so they don't push the bubble off-screen.
//
// Heavy stuff (Prism, language defs) is dynamic-imported by RSH on first
// use so the initial page load stays light.

import { memo, type ReactNode } from "react";
import { Box, Link, useTheme } from "@mui/material";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { PrismLight as SyntaxHighlighter } from "react-syntax-highlighter";
import {
  oneDark,
  oneLight,
} from "react-syntax-highlighter/dist/esm/styles/prism";
import bash from "react-syntax-highlighter/dist/esm/languages/prism/bash";
import diff from "react-syntax-highlighter/dist/esm/languages/prism/diff";
import javascript from "react-syntax-highlighter/dist/esm/languages/prism/javascript";
import json from "react-syntax-highlighter/dist/esm/languages/prism/json";
import jsx from "react-syntax-highlighter/dist/esm/languages/prism/jsx";
import markdown from "react-syntax-highlighter/dist/esm/languages/prism/markdown";
import python from "react-syntax-highlighter/dist/esm/languages/prism/python";
import rust from "react-syntax-highlighter/dist/esm/languages/prism/rust";
import toml from "react-syntax-highlighter/dist/esm/languages/prism/toml";
import tsx from "react-syntax-highlighter/dist/esm/languages/prism/tsx";
import typescript from "react-syntax-highlighter/dist/esm/languages/prism/typescript";
import yaml from "react-syntax-highlighter/dist/esm/languages/prism/yaml";

SyntaxHighlighter.registerLanguage("bash", bash);
SyntaxHighlighter.registerLanguage("sh", bash);
SyntaxHighlighter.registerLanguage("shell", bash);
SyntaxHighlighter.registerLanguage("diff", diff);
SyntaxHighlighter.registerLanguage("javascript", javascript);
SyntaxHighlighter.registerLanguage("js", javascript);
SyntaxHighlighter.registerLanguage("json", json);
SyntaxHighlighter.registerLanguage("jsx", jsx);
SyntaxHighlighter.registerLanguage("markdown", markdown);
SyntaxHighlighter.registerLanguage("md", markdown);
SyntaxHighlighter.registerLanguage("python", python);
SyntaxHighlighter.registerLanguage("py", python);
SyntaxHighlighter.registerLanguage("rust", rust);
SyntaxHighlighter.registerLanguage("rs", rust);
SyntaxHighlighter.registerLanguage("toml", toml);
SyntaxHighlighter.registerLanguage("tsx", tsx);
SyntaxHighlighter.registerLanguage("typescript", typescript);
SyntaxHighlighter.registerLanguage("ts", typescript);
SyntaxHighlighter.registerLanguage("yaml", yaml);
SyntaxHighlighter.registerLanguage("yml", yaml);

/// Render markdown text. Memoized on `text` so streamed updates don't
/// re-parse from scratch on every chunk (React reconciliation already helps,
/// but the markdown AST is the expensive bit).
///
/// Default export so `React.lazy(() => import('./MarkdownImpl'))` works.
/// Don't import this directly — go through `./Markdown` which provides a
/// Suspense fallback so the heavy syntax-highlighter chunk loads on demand.
const MarkdownImpl = memo(function MarkdownImpl({
  text,
  invert = false,
}: {
  /** Raw markdown source. */
  text: string;
  /** When true, render on a primary-colored bubble (the user's own
   *  messages). Switches code-block theme to light-on-dark inverse. */
  invert?: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const codeTheme = theme.palette.mode === "dark" || invert ? oneDark : oneLight;

  const components: Components = {
    // Inline `code` uses a subtle tint; fenced blocks get Prism.
    code({ className, children, ...rest }) {
      const inline = !className?.startsWith("language-");
      const text = String(children).replace(/\n$/, "");
      if (inline) {
        return (
          <Box
            component="code"
            sx={{
              px: 0.5,
              py: 0.1,
              borderRadius: 0.5,
              bgcolor: invert ? "rgba(255,255,255,0.18)" : "action.hover",
              fontSize: "0.85em",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              wordBreak: "break-word",
            }}
            {...rest}
          >
            {children}
          </Box>
        );
      }
      const lang = className?.replace("language-", "") ?? "";
      return (
        <Box sx={{ my: 1, "& pre": { borderRadius: 1, fontSize: "0.8em" } }}>
          <SyntaxHighlighter
            language={lang || "text"}
            style={codeTheme}
            customStyle={{ margin: 0, padding: 12 }}
            wrapLongLines={false}
            PreTag="pre"
          >
            {text}
          </SyntaxHighlighter>
        </Box>
      );
    },
    a({ children, href }) {
      return (
        <Link href={href ?? "#"} target="_blank" rel="noopener noreferrer">
          {children}
        </Link>
      );
    },
    table({ children }) {
      return (
        <Box
          sx={{ overflowX: "auto", my: 1, "& table": { borderCollapse: "collapse" } }}
        >
          <table>{children as ReactNode}</table>
        </Box>
      );
    },
    th({ children }) {
      return (
        <Box
          component="th"
          sx={{ border: 1, borderColor: "divider", px: 1, py: 0.5, textAlign: "left" }}
        >
          {children}
        </Box>
      );
    },
    td({ children }) {
      return (
        <Box
          component="td"
          sx={{ border: 1, borderColor: "divider", px: 1, py: 0.5 }}
        >
          {children}
        </Box>
      );
    },
    p({ children }) {
      return <Box sx={{ my: 0.5, lineHeight: 1.5 }}>{children}</Box>;
    },
    ul({ children }) {
      return <Box component="ul" sx={{ pl: 3, my: 0.5 }}>{children}</Box>;
    },
    ol({ children }) {
      return <Box component="ol" sx={{ pl: 3, my: 0.5 }}>{children}</Box>;
    },
    blockquote({ children }) {
      return (
        <Box
          sx={{
            borderLeft: 3,
            borderColor: invert ? "rgba(255,255,255,0.4)" : "primary.light",
            pl: 1.5,
            my: 1,
            color: invert ? "rgba(255,255,255,0.85)" : "text.secondary",
            fontStyle: "italic",
          }}
        >
          {children}
        </Box>
      );
    },
  };

  return (
    <Box
      sx={{
        wordBreak: "break-word",
        "& :first-child": { mt: 0 },
        "& :last-child": { mb: 0 },
      }}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </Box>
  );
});

export default MarkdownImpl;
