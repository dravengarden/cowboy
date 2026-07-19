import { type ReactNode, useCallback, useEffect, useState } from "react";
import { Box, ButtonBase, Chip, CircularProgress, IconButton, Stack, Tooltip, Typography, useMediaQuery } from "@mui/material";
import { CheckRounded, ContentCopyRounded } from "@mui/icons-material";
import { Markdown } from "../Markdown";
import { copyText } from "../clipboard";
import { Collapsible } from "./Collapsible";
import { unifiedDiff } from "./diff";
import { languageFromPath } from "../syntaxLanguages";
import { addShellPathBreaks, formatShellForDisplay } from "../shellFormatter";

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
  touchWrap = false,
  tokenSafeWrap = false,
  wrapControl = true,
  hideCopy = false,
}: {
  code: string;
  lang?: string;
  maxHeight?: number;
  centerCopy?: boolean;
  touchWrap?: boolean;
  tokenSafeWrap?: boolean;
  wrapControl?: boolean;
  hideCopy?: boolean;
}): React.JSX.Element {
  const coarse = useMediaQuery("(hover:none), (pointer:coarse)");
  const [wrapped, setWrapped] = useState((touchWrap || tokenSafeWrap) && coarse);
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
                onClick={(): void => setWrapped(value)}
                sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: 11, color: wrapped === value ? "text.primary" : "text.disabled", bgcolor: wrapped === value ? "background.paper" : "transparent", boxShadow: wrapped === value ? 1 : 0 }}
              >
                {value ? "Wrap" : "Scroll"}
              </ButtonBase>
            ))}
          </Box>
        </Stack>
      )}
      <Collapsible maxHeight={maxHeight ?? 280}>
        <Markdown
          text={"```" + (lang ?? "") + "\n" + code + "\n```"}
          centerCopy={centerCopy}
          touchWrap={false}
        />
      </Collapsible>
    </Box>
  );
}

/** A shell-aware, display-only view. mvdan/sh is loaded only when this component
 * is opened; parser failure leaves the exact source view untouched. */
export function ShellCommandView({ command }: { command: string }): React.JSX.Element {
  const touch = useMediaQuery("(hover:none), (pointer:coarse)");
  const phone = useMediaQuery("(max-width:600px)");
  const [readable, setReadable] = useState<{ text: string; context: string } | null | undefined>(undefined);
  const [mode, setMode] = useState<"readable" | "source">("source");

  useEffect(() => {
    let active = true;
    setReadable(undefined);
    setMode("source");
    void formatShellForDisplay(command, phone ? 46 : 88).then((formatted) => {
      if (!active) return;
      setReadable(formatted);
      if (formatted) setMode("readable");
    });
    return () => {
      active = false;
    };
  }, [command, phone]);

  const enhanced = readable != null;
  return (
    <Box>
      {(readable === undefined || enhanced) && (
        <Stack direction="row" justifyContent="flex-end" sx={{ mb: 0.375 }}>
          <Box role="group" aria-label="Shell command presentation" sx={{ display: "flex", bgcolor: "action.hover", borderRadius: 1, p: 0.25 }}>
            <ButtonBase
              aria-pressed={mode === "readable"}
              disabled={!enhanced}
              onClick={(): void => setMode("readable")}
              sx={{ minHeight: 28, minWidth: 70, px: 0.875, borderRadius: 0.75, fontSize: 11, color: mode === "readable" ? "text.primary" : "text.disabled", bgcolor: mode === "readable" ? "background.paper" : "transparent", boxShadow: mode === "readable" ? 1 : 0 }}
            >
              {readable === undefined ? <CircularProgress size={13} /> : "Readable"}
            </ButtonBase>
            <ButtonBase
              aria-pressed={mode === "source"}
              onClick={(): void => setMode("source")}
              sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: 11, color: mode === "source" ? "text.primary" : "text.disabled", bgcolor: mode === "source" ? "background.paper" : "transparent", boxShadow: mode === "source" ? 1 : 0 }}
            >
              Source
            </ButtonBase>
          </Box>
        </Stack>
      )}
      {mode === "readable" && readable?.context && (
        <Typography
          variant="caption"
          sx={{
            display: "block",
            mb: 0.5,
            color: "text.secondary",
            fontFamily: "ui-monospace, monospace",
            fontWeight: 600,
          }}
        >
          {readable.context}
        </Typography>
      )}
      <CodeView
        key={mode}
        code={mode === "readable" && readable
          ? (phone ? addShellPathBreaks(readable.text) : readable.text)
          : command}
        lang="bash"
        // A phone sheet has enough vertical room for roughly five more shell
        // lines before disclosure is useful. Keep Desktop compact, where Tool
        // details may share the screen with other work.
        maxHeight={touch ? 260 : 180}
        touchWrap={mode === "readable"}
        tokenSafeWrap={mode === "readable"}
        wrapControl={false}
        hideCopy
      />
    </Box>
  );
}

/** Plain monospace output (command stdout, generic text) — NOT a language, so a
 *  lightweight scrollable <pre> rather than the highlighter. Folded if long. */
export function PreBlock({ text, maxHeight }: { text: string; maxHeight?: number }): React.JSX.Element {
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
          whiteSpace: "pre",
          overflowX: "auto",
          overscrollBehaviorX: "contain",
          WebkitOverflowScrolling: "touch",
          "@media (hover: none), (pointer: coarse)": {
            whiteSpace: "pre-wrap",
            overflowWrap: "anywhere",
            wordBreak: "break-word",
            overflowX: "hidden",
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
      <Collapsible maxHeight={320}>
        <Markdown text={"```" + (sourceLanguage ? `diff-${sourceLanguage}` : "diff") + "\n" + text + "\n```"} />
      </Collapsible>
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
        fontSize: 11,
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
      if (text) rendered.push(<PreBlock key={i} text={text} maxHeight={420} />);
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
