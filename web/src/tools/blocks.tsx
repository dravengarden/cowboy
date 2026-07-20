import { type ReactNode, useCallback, useEffect, useState } from "react";
import { Box, ButtonBase, Chip, CircularProgress, IconButton, Stack, Tooltip, Typography, useMediaQuery, useTheme } from "@mui/material";
import { CheckRounded, ContentCopyRounded, SwapHorizRounded, WrapTextRounded } from "@mui/icons-material";
import { Markdown } from "../Markdown";
import { copyText } from "../clipboard";
import { Collapsible } from "./Collapsible";
import { nestedMarkerColor } from "./nestedMarkerColors";
import { unifiedDiff } from "./diff";
import { languageFromPath } from "../syntaxLanguages";
import { addShellPathBreaks, formatShellForDisplay } from "../shellFormatter";
import {
  chunkCodeForRendering,
  previewCodeForRendering,
  shouldUseLightweightCode,
} from "../codeRendering";

// Reusable presentational primitives for tool cards. They compose the existing
// lazy `Markdown` (which wraps react-syntax-highlighter) for all syntax
// highlighting — so there's ONE highlighter in the bundle and tool code gets the
// same theme/copy affordance as message code. Plain command output isn't a
// language, so it uses a lightweight <pre> (no highlighter) instead. Everything
// long is wrapped in `Collapsible`. Provider adapters (registry.tsx) reference
// these; they hold no provider knowledge themselves.

export function langFromPath(path: string): string {
  return languageFromPath(path);
}

/** A small caption above a block (Input / Output / Command …). */
export function Labeled({
  label,
  action,
  children,
}: {
  label: string;
  action?: ReactNode;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <Box sx={{ "& + &": { mt: 1 } }}>
      <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ minHeight: action ? 36 : 0, mb: 0.25 }}>
        <Typography variant="caption" sx={{ color: "text.secondary", fontWeight: 600 }}>
          {label}
        </Typography>
        {action}
      </Stack>
      {children}
    </Box>
  );
}

/** Compact semantic copy action for a labelled value. The icon is visually
 * small while retaining a 44px touch target on mobile. */
export function CopyTextButton({ text, label }: { text: string; label: string }): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const onCopy = useCallback(() => {
    void copyText(text).then((ok) => {
      if (!ok) return;
      setCopied(true);
      globalThis.setTimeout(() => setCopied(false), 1400);
    });
  }, [text]);
  return (
    <Tooltip title={copied ? "Copied" : `Copy ${label.toLowerCase()}`}>
      <IconButton
        aria-label={copied ? `${label} copied` : `Copy ${label.toLowerCase()}`}
        onClick={onCopy}
        sx={{ width: 44, height: 36, mr: -0.75 }}
      >
        {copied
          ? <CheckRounded color="success" sx={{ fontSize: 18 }} />
          : <ContentCopyRounded sx={{ fontSize: 18 }} />}
      </IconButton>
    </Tooltip>
  );
}

/** Highlighted source code (via Markdown's fenced highlighter), folded if long. */
export function CodeView({
  code,
  lang,
  maxHeight,
  centerCopy = false,
  tokenSafeWrap = false,
  wrapControl = true,
  hideCopy = false,
  wrap,
}: {
  code: string;
  lang?: string;
  maxHeight?: number;
  centerCopy?: boolean;
  touchWrap?: boolean;
  tokenSafeWrap?: boolean;
  wrapControl?: boolean;
  hideCopy?: boolean;
  /** Optional parent-controlled line layout (for mixed terminal output). */
  wrap?: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const [localWrapped, setLocalWrapped] = useState(false);
  const wrapped = wrap ?? localWrapped;
  const lightweight = shouldUseLightweightCode(code);
  const lightweightWrap = wrapped;
  const lightweightSx = {
    m: 0,
    p: 1.5,
    maxWidth: "100%",
    overflowX: lightweightWrap ? "hidden" : "auto",
    whiteSpace: lightweightWrap ? "pre-wrap" : "pre",
    overflowWrap: lightweightWrap ? "anywhere" : "normal",
    wordBreak: lightweightWrap ? "break-word" : "normal",
    bgcolor: theme.palette.mode === "dark" ? "#282c34" : "#fafafa",
    color: theme.palette.mode === "dark" ? "#abb2bf" : "#383a42",
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    fontSize: "0.8em",
    lineHeight: 1.5,
    WebkitOverflowScrolling: "touch",
  } as const;
  return (
    <Box
      sx={{
        ...(wrapped && {
          "& pre, & code": {
            whiteSpace: "pre-wrap !important",
            // Readable shell keeps each parsed token intact. Ordinary spaces
            // remain wrap opportunities, while a truly viewport-wide token
            // scrolls instead of being rendered as misleading fragments such
            // as `--no-` / `pager`.
            overflowWrap: tokenSafeWrap ? "normal" : "anywhere",
            wordBreak: tokenSafeWrap ? "normal" : "break-word",
            overflowX: tokenSafeWrap ? "auto !important" : "hidden !important",
          },
          ...(tokenSafeWrap && {
            // Prism marks flags as `.parameter`; keep those atomic. Quoted
            // strings and other free-form highlighted values may still be
            // wider than a phone, so let only those tokens make an emergency
            // break instead of forcing every subsequent line to scroll.
            "& code .token:not(.parameter)": {
              overflowWrap: "anywhere",
              wordBreak: "break-word",
            },
          }),
        }),
        ...(hideCopy && { "& .cowboy-copy-btn": { display: "none" } }),
      }}
    >
      {wrapControl && (
        <Stack direction="row" justifyContent="flex-end" sx={{ mb: 0.375 }}>
          <Box role="group" aria-label="Code line layout" sx={{ display: "flex", bgcolor: "action.hover", borderRadius: 1, p: 0.25 }}>
            {([false, true] as const).map((value) => (
              <ButtonBase
                key={String(value)}
                aria-pressed={wrapped === value}
                onClick={(): void => setLocalWrapped(value)}
                sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: wrapped === value ? "text.primary" : "text.disabled", bgcolor: wrapped === value ? "background.paper" : "transparent", boxShadow: wrapped === value ? 1 : 0 }}
              >
                {value ? "Wrap" : "Scroll"}
              </ButtonBase>
            ))}
          </Box>
        </Stack>
      )}
      {lightweight
        ? (
          <Box sx={{ position: "relative", "&:hover .cowboy-copy-btn": { opacity: 1 } }}>
            <Collapsible
              maxHeight={maxHeight ?? 280}
              forceOverflow
              collapsedChildren={
                <Box component="pre" data-code-renderer="lightweight-preview" sx={lightweightSx}>
                  {previewCodeForRendering(code)}
                </Box>
              }
            >
              <Box component="pre" data-code-renderer="lightweight" sx={lightweightSx}>
                {chunkCodeForRendering(code).map((chunk, index) => (
                  <Box
                    component="span"
                    key={index}
                    sx={{ display: "block", contentVisibility: "auto", containIntrinsicSize: "auto 1920px" }}
                  >
                    {chunk}
                  </Box>
                ))}
              </Box>
            </Collapsible>
            {!hideCopy && (
              <Box className="cowboy-copy-btn" sx={{ position: "absolute", top: 2, right: 4 }}>
                <CopyTextButton text={code} label="Code" />
              </Box>
            )}
          </Box>
        )
        : (
          <Collapsible maxHeight={maxHeight ?? 280}>
            <Markdown
              text={"```" + (lang ?? "") + "\n" + code + "\n```"}
              centerCopy={centerCopy}
              touchWrap={false}
            />
          </Collapsible>
        )}
    </Box>
  );
}

/** A shell-aware, display-only view. mvdan/sh is loaded only when this component
 * is opened; parser failure leaves the exact source view untouched. */
export function ShellCommandView({ command }: { command: string }): React.JSX.Element {
  const touch = useMediaQuery("(hover:none), (pointer:coarse)");
  const phone = useMediaQuery("(max-width:600px)");
  const theme = useTheme();
  const [readable, setReadable] = useState<Awaited<ReturnType<typeof formatShellForDisplay>> | undefined>(undefined);
  const [mode, setMode] = useState<"readable" | "nested" | "source">("source");
  const [wrapped, setWrapped] = useState(false);

  useEffect(() => {
    let active = true;
    setReadable(undefined);
    setMode("source");
    void formatShellForDisplay(command, phone ? 46 : 88).then((formatted) => {
      if (!active) return;
      setReadable(formatted);
      if (formatted) setMode(formatted.frames.length > 1 ? "nested" : "readable");
    });
    return () => {
      active = false;
    };
  }, [command, phone]);

  const enhanced = readable != null;
  const nested = !!readable && readable.frames.length > 1;
  return (
    <Box>
      <Stack direction="row" justifyContent="flex-end" alignItems="center" spacing={0.5} sx={{ mb: 0.5 }}>
        {(readable === undefined || enhanced) && (
          <Box role="group" aria-label="Shell command presentation" sx={{ display: "flex", bgcolor: "action.hover", borderRadius: 1, p: 0.25 }}>
            <ButtonBase
              aria-pressed={mode === "readable"}
              disabled={!enhanced}
              onClick={(): void => setMode("readable")}
              sx={{ minHeight: 28, minWidth: 70, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: mode === "readable" ? "text.primary" : "text.disabled", bgcolor: mode === "readable" ? "background.paper" : "transparent", boxShadow: mode === "readable" ? 1 : 0 }}
            >
              {readable === undefined ? <CircularProgress size="0.8125rem" /> : "Readable"}
            </ButtonBase>
            {nested && (
              <ButtonBase
                aria-pressed={mode === "nested"}
                onClick={(): void => setMode("nested")}
                sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: mode === "nested" ? "text.primary" : "text.disabled", bgcolor: mode === "nested" ? "background.paper" : "transparent", boxShadow: mode === "nested" ? 1 : 0 }}
              >
                Nested
              </ButtonBase>
            )}
            <ButtonBase
              aria-pressed={mode === "source"}
              onClick={(): void => setMode("source")}
              sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: mode === "source" ? "text.primary" : "text.disabled", bgcolor: mode === "source" ? "background.paper" : "transparent", boxShadow: mode === "source" ? 1 : 0 }}
            >
              Source
            </ButtonBase>
          </Box>
        )}
        <Tooltip title={wrapped ? "Use horizontal scrolling" : "Wrap long lines"}>
          <span>
            <ButtonBase
              aria-label={wrapped ? "Use horizontal scrolling" : "Wrap long lines"}
              aria-pressed={wrapped}
              onClick={(): void => setWrapped((value) => !value)}
              sx={{
                minHeight: 32,
                px: 0.875,
                gap: 0.375,
                borderRadius: 1,
                fontSize: "0.6875rem",
                color: "text.secondary",
                bgcolor: "action.hover",
                "&:active": { bgcolor: "action.selected" },
              }}
            >
              {wrapped
                ? <WrapTextRounded sx={{ fontSize: "1.25em" }} />
                : <SwapHorizRounded sx={{ fontSize: "1.25em" }} />}
              {wrapped ? "Wrap" : "Scroll"}
            </ButtonBase>
          </span>
        </Tooltip>
      </Stack>
      {mode === "nested" && readable
        ? (
          <Collapsible maxHeight={touch ? 360 : 320}>
            <Stack
              data-shell-frame-count={readable.frames.length}
              sx={{
                gap: 0,
                pb: readable.frames.length > 1 ? 0.625 : 0,
                borderRadius: 1,
                overflow: "hidden",
                bgcolor: theme.palette.mode === "dark" ? "#282c34" : "#fafafa",
              }}
            >
            {readable.frames.map((frame, index) => {
              const depth = frame.depth ?? index;
              const markerColor = nestedMarkerColor(frame.marker, theme.palette.mode === "dark", frame.color);
              // Every nested frame redraws the still-active ancestor rails in
              // the same coordinate system. This makes a parent rail continue
              // through its descendants instead of disappearing when the
              // child adds another level of indentation.
              const rails = Array.from({ length: depth }, (_, railIndex) => {
                const level = railIndex + 1;
                let owner = frame;
                for (let candidateIndex = index; candidateIndex >= 0; candidateIndex -= 1) {
                  const candidate = readable.frames[candidateIndex];
                  if (!candidate) continue;
                  const candidateDepth = candidate.depth ?? candidateIndex;
                  if (candidateDepth < level) break;
                  if (candidateDepth === level) {
                    owner = candidate;
                    break;
                  }
                }
                return {
                  level,
                  color: nestedMarkerColor(owner.marker, theme.palette.mode === "dark", owner.color),
                  current: level === depth,
                };
              });
              return (
              <Box
                key={`${index}:${frame.launcher}`}
                data-shell-frame={index}
                data-shell-depth={depth}
                sx={{
                  position: "relative",
                  // One compact gutter carries the hierarchy. Code blocks
                  // already have internal padding, so a second full indent
                  // wastes scarce phone width without adding information.
                  pl: depth === 0 ? 0 : `${10 + (depth - 1) * 8}px`,
                  py: depth === 0 ? 0 : 0.25,
                  ...(depth > 0 && {
                    "& pre": { paddingLeft: "6px !important" },
                  }),
                }}
              >
                {rails.map((rail) => (
                  <Box
                    key={rail.level}
                    aria-hidden="true"
                    data-shell-rail={rail.level}
                    sx={{
                      position: "absolute",
                      left: `${4 + (rail.level - 1) * 8}px`,
                      top: rail.current ? 8 : 0,
                      bottom: 0,
                      width: 2,
                      borderRadius: 999,
                      bgcolor: rail.color,
                      opacity: rail.current ? 0.72 : 0.48,
                    }}
                  />
                ))}
                {depth > 0 && (
                  <>
                    {[{ top: 8 }, { bottom: 0 }].map((edge, edgeIndex) => (
                      <Box
                        key={edgeIndex}
                        aria-hidden="true"
                        data-shell-rail-cap={edgeIndex === 0 ? "start" : "end"}
                        sx={{
                          position: "absolute",
                          left: `${4 + (depth - 1) * 8}px`,
                          ...edge,
                          width: 6,
                          height: 2,
                          borderRadius: 999,
                          bgcolor: markerColor,
                          opacity: 0.72,
                        }}
                      />
                    ))}
                  </>
                )}
                {depth > 0 && frame.marker && !frame.label && (
                  <Box
                    component="span"
                    aria-label={`${frame.label ?? frame.language ?? "Shell"} nested program ${frame.marker}`}
                    sx={{
                      position: "absolute",
                      zIndex: 1,
                      left: `${-3 + (depth - 1) * 8}px`,
                      top: 0,
                      display: "grid",
                      placeItems: "center",
                      minWidth: 18,
                      height: 16,
                      px: 0.25,
                      borderRadius: "50%",
                      bgcolor: theme.palette.mode === "dark" ? "#282c34" : "#fafafa",
                      fontSize: 8,
                      lineHeight: 1,
                    }}
                  >
                    {frame.marker}
                  </Box>
                )}
                {depth > 0 && frame.label && (
                  <Box
                    data-shell-language={frame.language ?? "unknown"}
                    sx={{
                      display: "flex",
                      alignItems: "center",
                      gap: 0.5,
                      minHeight: 20,
                      pl: 0.75,
                      pb: 0.125,
                      color: markerColor,
                      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                      fontSize: "0.6875rem",
                      fontWeight: 650,
                      letterSpacing: "0.015em",
                    }}
                  >
                    <Box component="span" aria-hidden="true" sx={{ fontSize: "0.875rem", lineHeight: 1 }}>
                      {frame.marker}
                    </Box>
                    {frame.language === "bash" && frame.label === "Shell" ? "Bash" : frame.label}
                    {frame.dialect && frame.dialect !== frame.label.toLowerCase() && (
                      <Box component="span" sx={{ color: "text.disabled", fontWeight: 500 }}>
                        · {frame.dialect}
                      </Box>
                    )}
                  </Box>
                )}
                {frame.text && (
                  <CodeView
                    code={phone && (!frame.language || frame.language === "bash") ? addShellPathBreaks(frame.text) : frame.text}
                    lang={frame.language ?? "bash"}
                    // Nested mode is one semantic disclosure. Its outer fold
                    // owns expansion so a parent frame cannot collapse while
                    // its children remain floating below it.
                    maxHeight={100000}
                    touchWrap
                    tokenSafeWrap
                    wrapControl={false}
                    wrap={wrapped}
                    hideCopy
                  />
                )}
              </Box>
              );
            })}
            </Stack>
          </Collapsible>
        )
        : mode === "readable" && readable
        ? (
          <CodeView
            key={mode}
            code={phone ? addShellPathBreaks(readable.flatText) : readable.flatText}
            lang="bash"
            maxHeight={touch ? 260 : 180}
            touchWrap
            tokenSafeWrap
            wrapControl={false}
            wrap={wrapped}
            hideCopy
          />
        )
        : (
          <CodeView
            key={mode}
            code={command}
            lang="bash"
            maxHeight={touch ? 260 : 180}
            wrapControl={false}
            wrap={wrapped}
            hideCopy
          />
        )}
    </Box>
  );
}

/** Plain monospace output (command stdout, generic text) — NOT a language, so a
 *  lightweight scrollable <pre> rather than the highlighter. Folded if long. */
export function PreBlock({
  text,
  maxHeight,
  wrap,
}: {
  text: string;
  maxHeight?: number;
  wrap?: boolean;
}): React.JSX.Element {
  const explicitLayout = wrap !== undefined;
  return (
    <Collapsible maxHeight={maxHeight ?? 240}>
      <Box
        component="pre"
        sx={{
          m: 0,
          p: 1,
          borderRadius: 1,
          bgcolor: "action.hover",
          fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
          fontSize: "0.8em",
          lineHeight: 1.5,
          whiteSpace: wrap ? "pre-wrap" : "pre",
          overflowWrap: wrap ? "anywhere" : "normal",
          wordBreak: wrap ? "break-word" : "normal",
          overflowX: wrap ? "hidden" : "auto",
          overscrollBehaviorX: "contain",
          WebkitOverflowScrolling: "touch",
          "@media (hover: none), (pointer: coarse)": {
            ...(!explicitLayout && {
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
              wordBreak: "break-word",
              overflowX: "hidden",
            }),
          },
        }}
      >
        {text || "—"}
      </Box>
    </Collapsible>
  );
}

/** A unified diff (old → new) coloured by the Markdown ```diff fence, with a
 *  compact +adds / −removes stat. Folded if long. */
export function DiffView({
  oldText,
  newText,
  path,
}: {
  oldText: string;
  newText: string;
  path?: string | undefined;
}): React.JSX.Element {
  const { text, added, removed } = unifiedDiff(oldText, newText);
  const sourceLanguage = path ? languageFromPath(path) : "";
  return (
    <Box>
      <Stack direction="row" spacing={0.75} sx={{ mb: 0.5 }}>
        <Typography variant="caption" sx={{ color: "success.main", fontWeight: 700 }}>
          +{added}
        </Typography>
        <Typography variant="caption" sx={{ color: "error.main", fontWeight: 700 }}>
          −{removed}
        </Typography>
      </Stack>
      <CodeView
        code={text}
        lang={sourceLanguage ? `diff-${sourceLanguage}` : "diff"}
        maxHeight={320}
        touchWrap
        wrapControl={false}
      />
    </Box>
  );
}

/** A file path as a code-styled chip; the basename shows, full path on hover. */
export function FileChip({ path }: { path: string }): React.JSX.Element {
  const name = path.split("/").pop() || path;
  return (
    <Chip
      size="small"
      label={name}
      title={path}
      sx={{
        height: 20,
        fontSize: "0.6875rem",
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        "& .MuiChip-label": { px: 0.75 },
      }}
    />
  );
}

/** A compact key→value table for a tool's raw args (the generic fallback). Long
 *  string values are folded individually; nested objects fall back to JSON. */
export function KeyValues({ data }: { data: Record<string, unknown> }): React.JSX.Element {
  const entries = Object.entries(data);
  if (entries.length === 0) return <Typography variant="caption" color="text.disabled">No arguments</Typography>;
  return (
    <Stack spacing={0.5}>
      {entries.map(([k, v]) => {
        const structured = typeof v !== "string";
        const str = structured ? JSON.stringify(v, null, 2) ?? String(v) : v;
        const multiline = str.includes("\n") || str.length > 80;
        return (
          <Box key={k} sx={multiline ? {} : { display: "flex", gap: 1, alignItems: "baseline", minWidth: 0 }}>
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", fontWeight: 600, fontFamily: "ui-monospace, monospace", flexShrink: 0 }}
            >
              {k}
            </Typography>
            {multiline
              ? structured
                ? <CodeView code={str} lang="json" maxHeight={200} touchWrap />
                : <PreBlock text={str} maxHeight={160} />
              : (
                <Typography
                  variant="caption"
                  sx={{ fontFamily: "ui-monospace, monospace", wordBreak: "break-word", color: "text.primary" }}
                >
                  {str}
                </Typography>
              )}
          </Box>
        );
      })}
    </Stack>
  );
}

// --- ACP content-block plumbing ---------------------------------------------

interface TextBlock { type: "content"; content?: { type?: string; text?: string } }
interface DiffBlock { type: "diff"; path?: string; oldText?: string; newText?: string }
type AnyBlock = TextBlock | DiffBlock | { type?: string; text?: string; [k: string]: unknown };

function asBlocks(content: unknown): AnyBlock[] {
  if (Array.isArray(content)) return content as AnyBlock[];
  if (content && typeof content === "object") return [content as AnyBlock];
  return [];
}

/** Concatenated plain text from a tool's content blocks (for output rendering). */
export function textOfContent(content: unknown): string {
  return asBlocks(content)
    .map((b) => {
      if (b.type === "content") return (b as TextBlock).content?.text ?? "";
      if (typeof (b as { text?: string }).text === "string") return (b as { text: string }).text;
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

/** Does this content carry a structured diff block? (edit/write/apply_patch) */
export function hasDiff(content: unknown): boolean {
  return asBlocks(content).some((b) => b.type === "diff");
}

// A tool's text content is ALREADY markdown as the agent sent it — a Read result
// is a ```-fenced code block, a sub-agent's result is prose, etc. So render it
// THROUGH Markdown verbatim; the old code wrapped it in ANOTHER ```lang fence,
// which double-fenced a Read: the agent's inner ``` immediately closed the outer
// fence, leaving an empty block (React rendered `String(undefined)` → the literal
// "undefined") and dumping the code below it as plain prose. When we DO know the
// language (a file read) and the agent's leading fence is bare (```), inject it so
// the block highlights instead of rendering plain.
function withFenceLang(text: string, lang?: string): string {
  if (!lang) return text;
  return text.replace(/^```[ \t]*(\r?\n)/, "```" + lang + "$1");
}

/** Render a tool's `content` array. Diff blocks → DiffView; text blocks → the
 *  agent's markdown (verbatim, lang injected for a bare code fence); anything else
 *  is skipped (the Raw escape hatch in the shell still shows it verbatim). */
export function OutputBlocks({
  content,
  lang,
  fallbackPath,
}: {
  content: unknown;
  lang?: string;
  fallbackPath?: string;
}): React.JSX.Element | null {
  const blocks = asBlocks(content);
  const rendered: ReactNode[] = [];
  blocks.forEach((b, i) => {
    if (b.type === "diff") {
      const d = b as DiffBlock;
      rendered.push(
        <DiffView
          key={i}
          oldText={d.oldText ?? ""}
          newText={d.newText ?? ""}
          path={d.path ?? fallbackPath}
        />,
      );
      return;
    }
    if (b.type === "raw_output") {
      const text = typeof b.text === "string" ? b.text : "";
      if (text) {
        // Codex Read events expose file bytes as rawOutput.formatted_output,
        // which derive normalizes to raw_output. "Raw" describes the ACP
        // transport shape, not the file's presentation: a known Markdown file
        // should still use the document renderer in Formatted mode.
        rendered.push(
          lang === "markdown"
            ? <Collapsible key={i} maxHeight={300}><Markdown text={text} /></Collapsible>
            : <PreBlock key={i} text={text} maxHeight={420} />,
        );
      }
      return;
    }
    const text = b.type === "content" ? (b as TextBlock).content?.text ?? "" : (b as { text?: string }).text ?? "";
    if (text) {
      // Markdown files are documents, not source-code samples. Feed them to
      // the existing GFM renderer directly; Raw mode remains the exact-source
      // escape hatch in Tool details. Other known file languages stay fenced
      // for syntax highlighting.
      const markdownDocument = lang === "markdown";
      const source = !markdownDocument && lang && !text.trimStart().startsWith("```");
      rendered.push(
        <Collapsible key={i} maxHeight={300}>
          <Markdown
            text={markdownDocument
              ? text
              : source
              ? "```" + lang + "\n" + text + "\n```"
              : withFenceLang(text, lang)}
          />
        </Collapsible>,
      );
    }
  });
  if (rendered.length === 0) return null;
  return <Stack spacing={1}>{rendered}</Stack>;
}
