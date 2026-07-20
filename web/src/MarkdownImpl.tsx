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

import {
  Component,
  type HTMLAttributes,
  memo,
  type ReactNode,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Box, IconButton, Link, useMediaQuery, useTheme } from "@mui/material";
import { Check, ContentCopy } from "@mui/icons-material";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkFrontmatter from "remark-frontmatter";
import { ImageLightbox } from "./_shell";
import { openExternalUrl, shouldRouteExternalClick } from "./openExternal";
import { copyText } from "./clipboard";
import { Collapsible } from "./tools/Collapsible";
import { PrismAsyncLight as SyntaxHighlighter } from "react-syntax-highlighter";
import bash from "react-syntax-highlighter/dist/esm/languages/prism/bash";
import {
  oneDark,
  oneLight,
} from "react-syntax-highlighter/dist/esm/styles/prism";
import { normalizeSyntaxLanguage } from "./syntaxLanguages";
import {
  chunkCodeForRendering,
  previewCodeForRendering,
  shouldUseLightweightCode,
} from "./codeRendering";
import { SHELL_COMMENT_PATTERN, SHELL_SYNTAX_LANGUAGE } from "./shellLanguage";

// Extend Prism's Bash grammar inside the already-lazy Markdown bundle. Tool
// cards only import SHELL_SYNTAX_LANGUAGE, so this semantic enhancement never
// pulls the highlighter into the eager application chunk.
function cowboyShell(prism: Parameters<typeof bash>[0]): void {
  bash(prism);
  const languages = (prism as {
    languages: Record<string, unknown>;
  }).languages as Record<string, unknown> & {
    extend(language: string, redef: Record<string, unknown>): Record<string, unknown>;
    insertBefore(
      language: string,
      before: string,
      insert: Record<string, unknown>,
    ): Record<string, unknown>;
  };
  languages[SHELL_SYNTAX_LANGUAGE] = languages.extend("bash", {});
  // Prism's stock Bash grammar treats any `#` outside the few string shapes it
  // recognizes as a comment. Shell comments, however, only begin where a new
  // word may start. Keep URI/flake fragments such as `.#packages.x86_64-linux`
  // and `https://host/#fragment` as ordinary arguments while retaining real
  // comments after whitespace or a control operator.
  (languages[SHELL_SYNTAX_LANGUAGE] as Record<string, unknown>).comment = {
    pattern: SHELL_COMMENT_PATTERN,
    lookbehind: true,
    greedy: true,
  };
  languages.insertBefore(SHELL_SYNTAX_LANGUAGE, "string", {
    "dsl-sed": {
      pattern:
        /((?:^|[;&|]\s*|\n\s*)(?:sudo\s+)?sed\b(?:\s+(?:-{1,2}[\w-]+(?:=[^\s]+)?))*\s+)(["'])(?:\\.|(?!\2)[^\\\r\n])*\2/gm,
      lookbehind: true,
      greedy: true,
      alias: ["string", "dsl-expression", "dsl-sed-expression"],
    },
    "dsl-regex": {
      pattern:
        /((?:^|[;&|]\s*|\n\s*)(?:sudo\s+)?(?:rg|ripgrep|grep|egrep|pgrep|pkill)\b(?:\s+(?:-{1,2}[\w-]+(?:=[^\s]+)?))*\s+)(["'])(?:\\.|(?!\2)[^\\\r\n])*\2/gm,
      lookbehind: true,
      greedy: true,
      alias: ["string", "dsl-expression", "dsl-regex-expression"],
    },
  });
}

(cowboyShell as typeof cowboyShell & { displayName?: string }).displayName =
  SHELL_SYNTAX_LANGUAGE;
SyntaxHighlighter.registerLanguage(SHELL_SYNTAX_LANGUAGE, cowboyShell);

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
  centerCopy,
  touchWrap,
}: {
  code: string;
  lang: string;
  codeTheme: typeof oneDark;
  dark: boolean;
  centerCopy: boolean;
  touchWrap: boolean;
}): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const coarse = useMediaQuery("(hover:none), (pointer:coarse)");
  const onCopy = useCallback(() => {
    void copyText(code).then((ok) => {
      if (!ok) return;
      setCopied(true);
      globalThis.setTimeout(() => setCopied(false), 1500);
    });
  }, [code]);
  const diffLanguage = lang.startsWith("diff-")
    ? normalizeSyntaxLanguage(lang.slice("diff-".length))
    : "";
  const syntaxLanguage = diffLanguage || normalizeSyntaxLanguage(lang) || "text";
  const diffLines = diffLanguage ? code.split("\n") : [];
  // Only apply diff row treatment to a real prefixed patch. This also makes a
  // hand-written ```diff-typescript fence degrade safely if it contains plain
  // source rather than the structured diff format Cowboy emits.
  const sourceAwareDiff = Boolean(diffLanguage) &&
    diffLines.some((line) => line.startsWith("+") || line.startsWith("-")) &&
    diffLines.every((line) => line === "" || /^[ +-]/u.test(line));
  // A horizontally panned diff loses both its line prefix and the left edge of
  // every changed row. That is especially easy to trigger while vertically
  // scrolling a sheet on a phone, and leaves the coloured row backgrounds
  // looking detached from their text. Keep ordinary code user-selectable, but
  // make structured diffs wrap by default on touch surfaces: the +/- and full
  // statement remain visible together, while Desktop retains exact-line
  // horizontal scrolling.
  const wrapOnTouch = touchWrap || (sourceAwareDiff && coarse);
  const diffSigns = sourceAwareDiff ? diffLines.map((line) => line[0] ?? " ") : [];
  // A source-aware diff needs two independent layers: Prism should tokenize
  // the underlying file language, while the row keeps its +/- diff meaning.
  // Feeding the prefix into (for example) TypeScript makes the first token on
  // every changed line invalid, so remove it for tokenization and paint it back
  // with a row pseudo-element. Copy still uses the untouched `code` above.
  const highlightedCode = sourceAwareDiff
    ? diffLines.map((line) => line.slice(1)).join("\n")
    : code;
  const lightweight = shouldUseLightweightCode(code);
  const lightweightChunks = lightweight ? chunkCodeForRendering(code) : [];
  const lightweightPreview = lightweight ? previewCodeForRendering(code) : "";
  const lightweightSx = {
    m: 0,
    p: 1.5,
    maxWidth: "100%",
    overflowX: wrapOnTouch ? "hidden" : "auto",
    whiteSpace: wrapOnTouch ? "pre-wrap" : "pre",
    overflowWrap: wrapOnTouch ? "anywhere" : "normal",
    wordBreak: wrapOnTouch ? "break-word" : "normal",
    bgcolor: dark ? "#282c34" : "#fafafa",
    color: dark ? "#abb2bf" : "#383a42",
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    fontSize: "0.8em",
    lineHeight: 1.5,
    WebkitOverflowScrolling: "touch",
  } as const;
  // Tool details reuses this component while stepping between transcript
  // entries. A native horizontally scrolled <pre> otherwise keeps its old
  // scrollLeft and makes the next file/diff appear clipped from the left.
  // Reset only when the underlying block changes; ordinary reading scrolls are
  // left untouched.
  useLayoutEffect(() => {
    const pre = rootRef.current?.querySelector("pre");
    if (pre) pre.scrollLeft = 0;
  }, [highlightedCode, syntaxLanguage, wrapOnTouch]);
  return (
    <Box
      ref={rootRef}
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
        "& code > [data-diff-sign]::before": {
          content: "attr(data-diff-sign)",
          display: "inline-block",
          width: "1ch",
          color: "currentColor",
          fontWeight: 700,
        },
        ...(wrapOnTouch && {
          "@media (hover: none), (pointer: coarse)": {
            "& pre, & code": {
              whiteSpace: "pre-wrap !important",
              overflowWrap: "anywhere",
              wordBreak: "break-word",
              overflowX: "hidden !important",
            },
          },
        }),
        ...(syntaxLanguage === "cowboy-shell" && {
          "& .token.dsl-expression": {
            display: "inline",
            borderRadius: "0.28em",
            paddingInline: "0.14em",
            boxDecorationBreak: "clone",
            WebkitBoxDecorationBreak: "clone",
            fontWeight: 520,
          },
          "& .token.dsl-regex-expression": {
            backgroundColor: dark ? "rgba(86, 156, 214, 0.12)" : "rgba(9, 105, 218, 0.08)",
            boxShadow: dark
              ? "inset 0 -1px rgba(86, 156, 214, 0.5)"
              : "inset 0 -1px rgba(9, 105, 218, 0.35)",
          },
          "& .token.dsl-sed-expression": {
            backgroundColor: dark ? "rgba(220, 220, 170, 0.1)" : "rgba(154, 103, 0, 0.08)",
            boxShadow: dark
              ? "inset 0 -1px rgba(220, 220, 170, 0.48)"
              : "inset 0 -1px rgba(154, 103, 0, 0.34)",
          },
        }),
      }}
    >
      {lightweight
        ? (
          <Collapsible
            maxHeight={280}
            forceOverflow
            collapsedChildren={
              <Box component="pre" data-code-renderer="lightweight-preview" sx={lightweightSx}>
                {lightweightPreview}
              </Box>
            }
          >
          <Box
            component="pre"
            data-code-renderer="lightweight"
            sx={lightweightSx}
          >
            {lightweightChunks.map((chunk, index) => (
              <Box
                component="span"
                key={index}
                sx={{
                  display: "block",
                  // The browser lays out only chunks near the sheet viewport.
                  // The intrinsic height keeps the parent fold measurable while
                  // thousands of off-screen lines remain out of the render path.
                  contentVisibility: "auto",
                  containIntrinsicSize: "auto 1920px",
                }}
              >
                {chunk}
              </Box>
            ))}
          </Box>
          </Collapsible>
        )
        : (
          <SyntaxHighlighter
            language={syntaxLanguage}
            style={codeTheme}
            // overflowX on the inline customStyle (highest specificity) so it wins
            // over the prism theme's own pre style — the long-line scroll must not
            // depend on the emotion class losing/winning the cascade.
            customStyle={{
              margin: 0,
              padding: 12,
              overflowX: wrapOnTouch ? "hidden" : "auto",
              maxWidth: "100%",
            }}
            wrapLongLines={wrapOnTouch}
            wrapLines={sourceAwareDiff}
            // RSH only supplies an actual row index to `lineProps` when line
            // numbers are enabled. Keep that indexing contract, but hide the
            // number gutter: the diff prefix itself is the useful marker here.
            showLineNumbers={sourceAwareDiff}
            lineNumberStyle={sourceAwareDiff ? { display: "none" } : undefined}
            lineProps={sourceAwareDiff
              ? (lineNumber: number): HTMLAttributes<HTMLElement> & { "data-diff-sign": string } => {
                const sign = diffSigns[lineNumber - 1] ?? " ";
                return {
                  "data-diff-sign": sign,
                  style: {
                    display: "block",
                    marginInline: -12,
                    paddingInline: 12,
                    background: sign === "+"
                      ? dark ? "rgba(46, 160, 67, 0.15)" : "rgba(46, 160, 67, 0.09)"
                      : sign === "-"
                      ? dark ? "rgba(248, 81, 73, 0.15)" : "rgba(248, 81, 73, 0.09)"
                      : "transparent",
                  },
                };
              }
              : undefined}
            PreTag="pre"
          >
            {highlightedCode}
          </SyntaxHighlighter>
        )}
      <IconButton
        className="cowboy-copy-btn"
        onClick={onCopy}
        aria-label={copied ? "Copied" : "Copy code"}
        size="small"
        sx={{
          position: "absolute",
          top: centerCopy ? "50%" : 6,
          right: 6,
          transform: centerCopy ? "translateY(-50%)" : "none",
          height: 32,
          minWidth: 32,
          // Explicit width to OVERRIDE the global MuiIconButton `width: 44` —
          // without it the button is locked at 44px and the morphed "Copied" label
          // overflows / gets clipped at the code block's right edge. Resting: a
          // 32px square; copied: `auto` so it grows to fit "✓ Copied".
          width: copied ? "auto" : 32,
          // Resting: a compact frosted square (opt out of the global 44px icon
          // button). On success it MORPHS into a labelled green pill — an
          // icon-only swap is too easy to miss; "Copied" is unmistakable. The
          // padding/gap animate the morph; right-anchored so it grows leftward
          // over the code for the ~1.5s it's shown, never shifting layout.
          px: copied ? 0.9 : 0,
          gap: copied ? 0.5 : 0,
          borderRadius: 1.5,
          fontSize: "0.72rem",
          fontWeight: 600,
          lineHeight: 1,
          whiteSpace: "nowrap",
          "& .MuiSvgIcon-root": { fontSize: "1rem" },
          color: copied
            ? "success.main"
            : dark
              ? "rgba(255,255,255,0.82)"
              : "rgba(0,0,0,0.55)",
          bgcolor: copied
            ? dark
              ? "rgba(63,185,80,0.18)"
              : "rgba(46,160,67,0.12)"
            : dark
              ? "rgba(40,44,52,0.7)"
              : "rgba(255,255,255,0.82)",
          backdropFilter: "blur(4px)",
          border: 1,
          borderColor: copied ? "success.main" : "divider",
          // Mouse: faint until the block is hovered (the wrapper's &:hover rule
          // raises it). Touch (no hover): clearly visible. Copied: full opacity.
          opacity: copied ? 1 : 0.5,
          "@media (hover: none)": { opacity: copied ? 1 : 0.9 },
          transition:
            "opacity .15s, background-color .18s, color .18s, border-color .18s, padding .18s, gap .18s",
          ...(!copied && {
            "&:hover": {
              bgcolor: dark ? "rgba(40,44,52,0.95)" : "rgba(255,255,255,0.98)",
            },
          }),
        }}
      >
        {copied ? <Check sx={{ fontSize: 16 }} /> : <ContentCopy sx={{ fontSize: 15 }} />}
        {copied && <Box component="span">Copied</Box>}
      </IconButton>
    </Box>
  );
}

class MarkdownCodeBoundary extends Component<
  { children: ReactNode; code: string; dark: boolean },
  { failed: boolean }
> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override componentDidCatch(error: Error): void {
    console.warn("Markdown code highlighting fell back to source", error);
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return (
      <Box
        component="pre"
        data-markdown-code-fallback
        sx={{
          m: 0,
          p: 1.5,
          maxWidth: "100%",
          overflowX: "auto",
          bgcolor: this.props.dark ? "#282c34" : "#fafafa",
          color: this.props.dark ? "#abb2bf" : "#383a42",
          fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
          fontSize: "0.8em",
          lineHeight: 1.5,
          WebkitOverflowScrolling: "touch",
        }}
      >
        {this.props.code}
      </Box>
    );
  }
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
  centerCopy = false,
  touchWrap = false,
}: {
  /** Raw markdown source. */
  text: string;
  /** When true, render on a primary-colored bubble (the user's own
   *  messages). Switches code-block theme to light-on-dark inverse. */
  invert?: boolean;
  centerCopy?: boolean;
  touchWrap?: boolean;
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
      return (
        <MarkdownCodeBoundary key={`${lang}:${text}`} code={text} dark={dark}>
          <CodeBlock
            code={text}
            lang={lang}
            codeTheme={codeTheme}
            dark={dark}
            centerCopy={centerCopy}
            touchWrap={touchWrap}
          />
        </MarkdownCodeBoundary>
      );
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
        <Link
          href={href ?? "#"}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event): void => {
            if (!href || !shouldRouteExternalClick(event)) return;
            event.preventDefault();
            openExternalUrl(href);
          }}
          // On an inverted (user) bubble the text is white on the primary fill, so
          // the default theme link colour is near-invisible — a link (e.g. a bare
          // `git@github.com` that remark-gfm auto-linked) then reads as "lost". Make
          // links inherit the bubble's text colour there; the underline still marks
          // them as links.
          sx={invert
            ? { color: "inherit", textDecorationColor: "inherit" }
            : undefined}
        >
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
        <ReactMarkdown remarkPlugins={[remarkFrontmatter, remarkGfm]} components={components}>
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
