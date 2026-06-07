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

import { memo, type ReactNode, useCallback, useMemo, useState } from "react";
import { Box, IconButton, Link, useTheme } from "@mui/material";
import { Check, ContentCopy } from "@mui/icons-material";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ImageLightbox } from "./_shell";
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

// Markdown images in document order, so the `img` override can find a clicked
// thumbnail's index and the lightbox can page through the whole message's
// images. Only `![alt](url)` syntax — cowboy doesn't enable raw HTML, so there
// are no `<img>` tags to miss.
const IMAGE_RE = /!\[([^\]]*)\]\(\s*(<[^>]+>|[^()\s]+)\s*(?:"[^"]*")?\)/g;
function parseImages(md: string): { src: string; alt: string }[] {
  const out: { src: string; alt: string }[] = [];
  for (const m of md.matchAll(IMAGE_RE)) {
    const alt = m[1] ?? "";
    // `<url>` angle-bracket form strips the brackets.
    const raw = m[2] ?? "";
    const src = raw.startsWith("<") && raw.endsWith(">") ? raw.slice(1, -1) : raw;
    if (src) out.push({ src, alt });
  }
  return out;
}

// Copy to the clipboard, with a legacy fallback. The async Clipboard API needs
// a SECURE context: it's present over the https tailnet (what phones/desktop
// actually use) but ABSENT over the plain http LAN IP (the dev bridge), so the
// hidden-textarea execCommand path keeps copy working there too. Returns success.
async function copyText(text: string): Promise<boolean> {
  try {
    if (globalThis.navigator?.clipboard?.writeText) {
      await globalThis.navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // secure-context denial / not-focused → fall through to the legacy path
  }
  try {
    const doc = globalThis.document;
    const ta = doc.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    doc.body.appendChild(ta);
    ta.select();
    // eslint-disable-next-line @typescript-eslint/no-deprecated -- only non-secure-context copy path
    const ok = doc.execCommand("copy");
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}

// A fenced code block + a top-right copy button. The button sits ABSOLUTELY in
// the (non-scrolling) wrapper, so it stays pinned to the visible top-right even
// as the code scrolls sideways. UX for both surfaces: a comfortable 34px hit
// target (desktop clickability), always visible on touch (`hover: none`), faint
// -until-hover on a mouse so it doesn't clutter the code. Tapping copies and
// flips to a success check for ~1.5s.
function CodeBlock({
  code,
  lang,
  codeTheme,
  dark,
}: {
  code: string;
  lang: string;
  codeTheme: typeof oneDark;
  dark: boolean;
}): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const onCopy = useCallback(() => {
    void copyText(code).then((ok) => {
      if (!ok) return;
      setCopied(true);
      globalThis.setTimeout(() => setCopied(false), 1500);
    });
  }, [code]);
  return (
    <Box
      sx={{
        my: 1,
        maxWidth: "100%",
        position: "relative",
        // Mouse: reveal the button on block hover (keeps the code clean).
        "&:hover .cowboy-copy-btn": { opacity: 1 },
        // `wrapLongLines={false}` keeps lines intact; the pre scrolls sideways
        // (touch momentum) instead of overflowing the bubble.
        "& pre": {
          borderRadius: 1,
          fontSize: "0.8em",
          overflowX: "auto",
          maxWidth: "100%",
          WebkitOverflowScrolling: "touch",
        },
      }}
    >
      <SyntaxHighlighter
        language={lang || "text"}
        style={codeTheme}
        // overflowX on the inline customStyle (highest specificity) so it wins
        // over the prism theme's own pre style — the long-line scroll must not
        // depend on the emotion class losing/winning the cascade.
        customStyle={{ margin: 0, padding: 12, overflowX: "auto", maxWidth: "100%" }}
        wrapLongLines={false}
        PreTag="pre"
      >
        {code}
      </SyntaxHighlighter>
      <IconButton
        className="cowboy-copy-btn"
        onClick={onCopy}
        aria-label={copied ? "Copied" : "Copy code"}
        size="small"
        sx={{
          position: "absolute",
          top: 6,
          right: 6,
          width: 34,
          height: 34,
          borderRadius: 1,
          color: copied
            ? "success.main"
            : dark
              ? "rgba(255,255,255,0.85)"
              : "rgba(0,0,0,0.6)",
          bgcolor: dark ? "rgba(40,44,52,0.72)" : "rgba(255,255,255,0.8)",
          backdropFilter: "blur(3px)",
          border: 1,
          borderColor: "divider",
          // Mouse: faint until the block is hovered (the &:hover rule above
          // raises it to 1). Touch (no hover): always visible.
          opacity: 0.42,
          "@media (hover: none)": { opacity: 0.9 },
          transition: "opacity .15s, background-color .15s",
          "&:hover": {
            bgcolor: dark ? "rgba(40,44,52,0.95)" : "rgba(255,255,255,0.97)",
          },
        }}
      >
        {copied ? <Check sx={{ fontSize: 17 }} /> : <ContentCopy sx={{ fontSize: 16 }} />}
      </IconButton>
    </Box>
  );
}

// Build a heading component renderer at a given em size + line-height. Kept at
// module scope so the `components` map below stays declarative; `mt`/`mb` are
// uniform so consecutive headings/paragraphs keep an even rhythm.
function makeHeading(
  fontSize: string,
  lineHeight: number,
): (props: { children?: ReactNode }) => React.JSX.Element {
  return function Heading({ children }): React.JSX.Element {
    return (
      <Box
        sx={{
          fontSize,
          fontWeight: 600,
          lineHeight,
          mt: 1.2,
          mb: 0.5,
        }}
      >
        {children}
      </Box>
    );
  };
}

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
  const dark = theme.palette.mode === "dark" || invert;
  const codeTheme = dark ? oneDark : oneLight;

  // Images render as capped thumbnails in the bubble; tapping opens the shared
  // fullscreen lightbox. Gallery = this message's images (per-message scope), so
  // swipe/←→ pages within the one message.
  const galleryImages = useMemo(() => parseImages(text), [text]);
  const [lbIndex, setLbIndex] = useState<number | null>(null);

  const components: Components = {
    img({ src, alt }) {
      const url = typeof src === "string" ? src : "";
      const i = galleryImages.findIndex((g) => g.src === url);
      return (
        <Box
          component="img"
          src={url}
          alt={alt ?? ""}
          loading="lazy"
          onClick={() => setLbIndex(i >= 0 ? i : 0)}
          sx={{
            display: "block",
            maxWidth: "100%",
            maxHeight: 200,
            width: "auto",
            my: 0.5,
            borderRadius: 1,
            border: 1,
            borderColor: "divider",
            objectFit: "contain",
            cursor: "zoom-in",
          }}
        />
      );
    },
    // Inline `code` uses a subtle tint; fenced blocks get Prism. A fence with
    // NO language tag carries no `language-*` class, so the className test alone
    // misclassified it as inline — it then rendered as a tinted span inside
    // ReactMarkdown's default `<pre>` (no overflow handling → clipped by the
    // transcript's `overflow-x: hidden`). Treat any multi-line content as a
    // block too, so an un-tagged fence still gets the scrollable highlighter.
    code({ className, children, ...rest }) {
      const text = String(children).replace(/\n$/, "");
      const inline = !className?.startsWith("language-") && !text.includes("\n");
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
      return <CodeBlock code={text} lang={lang} codeTheme={codeTheme} dark={dark} />;
    },
    // ReactMarkdown wraps a fenced block in `<pre><code>`. The `code` override
    // above already renders its own scrollable container (the highlighter's
    // pre), so collapse the outer default `<pre>` to a passthrough — otherwise
    // it's a second, un-styled pre that expands to the code's width and gets
    // clipped by the transcript's `overflow-x: hidden` instead of scrolling.
    pre({ children }) {
      return <>{children}</>;
    },
    a({ children, href }) {
      return (
        <Link href={href ?? "#"} target="_blank" rel="noopener noreferrer">
          {children}
        </Link>
      );
    },
    table({ children }) {
      // Wide tables scroll horizontally inside the bubble rather than wrapping
      // their cells into an unreadable squish or stretching the message off the
      // viewport. The ancestor `wordBreak: break-word` would otherwise force
      // every cell to wrap (so a table never overflows, it just crams) — the
      // `nowrap` on th/td below is what lets the overflow-x actually engage.
      // `maxWidth: 100%` keeps the scroll region inside the bubble; touch
      // momentum scrolling for iOS.
      return (
        <Box
          sx={{
            overflowX: "auto",
            maxWidth: "100%",
            my: 1,
            WebkitOverflowScrolling: "touch",
            "& table": { borderCollapse: "collapse", width: "max-content" },
          }}
        >
          <table>{children as ReactNode}</table>
        </Box>
      );
    },
    th({ children }) {
      return (
        <Box
          component="th"
          sx={{
            border: 1,
            borderColor: "divider",
            px: 1,
            py: 0.5,
            textAlign: "left",
            whiteSpace: "nowrap",
          }}
        >
          {children}
        </Box>
      );
    },
    td({ children }) {
      return (
        <Box
          component="td"
          sx={{ border: 1, borderColor: "divider", px: 1, py: 0.5, whiteSpace: "nowrap" }}
        >
          {children}
        </Box>
      );
    },
    // Headings were unstyled, so they fell back to the browser's `<h1>`/`<h2>`
    // defaults (2em / 1.5em + large margins) — oversized in a chat bubble and
    // especially heavy on a phone. Render them em-relative (so they still scale
    // with the OS base size) but tighter, with compact margins. Sizes step down
    // h1→h4; h5/h6 collapse to body weight-only emphasis.
    h1: makeHeading("1.35em", 1.3),
    h2: makeHeading("1.2em", 1.3),
    h3: makeHeading("1.08em", 1.35),
    h4: makeHeading("1em", 1.4),
    h5: makeHeading("0.92em", 1.4),
    h6: makeHeading("0.85em", 1.4),
    p({ children }) {
      // Inherit the reading line-height set on the transcript scroll container
      // (Settings → Reading), instead of a fixed 1.5. Default container leading
      // is still 1.5, so unset reads unchanged.
      return <Box sx={{ my: 0.5, lineHeight: "inherit" }}>{children}</Box>;
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
    <>
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
      {/* plate={false}: chat images are screenshots/photos, not white-bg
          diagrams, so no white framing behind them. */}
      <ImageLightbox
        images={galleryImages}
        index={lbIndex}
        onIndex={setLbIndex}
        onClose={() => setLbIndex(null)}
        plate={false}
      />
    </>
  );
});

export default MarkdownImpl;
