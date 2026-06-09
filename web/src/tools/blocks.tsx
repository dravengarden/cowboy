import { type ReactNode } from "react";
import { Box, Chip, Stack, Typography } from "@mui/material";
import { Markdown } from "../Markdown";
import { Collapsible } from "./Collapsible";
import { unifiedDiff } from "./diff";

// Reusable presentational primitives for tool cards. They compose the existing
// lazy `Markdown` (which wraps react-syntax-highlighter) for all syntax
// highlighting — so there's ONE highlighter in the bundle and tool code gets the
// same theme/copy affordance as message code. Plain command output isn't a
// language, so it uses a lightweight <pre> (no highlighter) instead. Everything
// long is wrapped in `Collapsible`. Provider adapters (registry.tsx) reference
// these; they hold no provider knowledge themselves.

// File extension → a Prism language id the Markdown highlighter has registered
// (see MarkdownImpl). Unregistered extensions fall through to plaintext, which
// renders fine (just uncoloured), so the map only needs the common ones.
const LANG_BY_EXT: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  py: "python",
  rs: "rust",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  markdown: "markdown",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
};

export function langFromPath(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return LANG_BY_EXT[ext] ?? "";
}

/** A small caption above a block (Input / Output / Command …). */
export function Labeled({ label, children }: { label: string; children: ReactNode }): React.JSX.Element {
  return (
    <Box sx={{ "& + &": { mt: 1 } }}>
      <Typography variant="caption" sx={{ color: "text.secondary", fontWeight: 600, display: "block", mb: 0.25 }}>
        {label}
      </Typography>
      {children}
    </Box>
  );
}

/** Highlighted source code (via Markdown's fenced highlighter), folded if long. */
export function CodeView({ code, lang, maxHeight }: { code: string; lang?: string; maxHeight?: number }): React.JSX.Element {
  return (
    <Collapsible maxHeight={maxHeight ?? 280}>
      <Markdown text={"```" + (lang ?? "") + "\n" + code + "\n```"} />
    </Collapsible>
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
          WebkitOverflowScrolling: "touch",
        }}
      >
        {text || "—"}
      </Box>
    </Collapsible>
  );
}

/** A unified diff (old → new) coloured by the Markdown ```diff fence, with a
 *  compact +adds / −removes stat. Folded if long. */
export function DiffView({ oldText, newText }: { oldText: string; newText: string }): React.JSX.Element {
  const { text, added, removed } = unifiedDiff(oldText, newText);
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
        <Markdown text={"```diff\n" + text + "\n```"} />
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
        const str = typeof v === "string" ? v : JSON.stringify(v, null, 2);
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
              ? <PreBlock text={str} maxHeight={160} />
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
export function OutputBlocks({ content, lang }: { content: unknown; lang?: string }): React.JSX.Element | null {
  const blocks = asBlocks(content);
  const rendered: ReactNode[] = [];
  blocks.forEach((b, i) => {
    if (b.type === "diff") {
      const d = b as DiffBlock;
      rendered.push(<DiffView key={i} oldText={d.oldText ?? ""} newText={d.newText ?? ""} />);
      return;
    }
    const text = b.type === "content" ? (b as TextBlock).content?.text ?? "" : (b as { text?: string }).text ?? "";
    if (text) {
      rendered.push(
        <Collapsible key={i} maxHeight={300}>
          <Markdown text={withFenceLang(text, lang)} />
        </Collapsible>,
      );
    }
  });
  if (rendered.length === 0) return null;
  return <Stack spacing={1}>{rendered}</Stack>;
}
