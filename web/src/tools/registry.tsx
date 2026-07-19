import { Component, type ReactNode, useEffect, useMemo, useState } from "react";
import { Box, ButtonBase, Skeleton, Stack, Typography } from "@mui/material";
import CheckCircleRounded from "@mui/icons-material/CheckCircleRounded";
import RadioButtonUncheckedRounded from "@mui/icons-material/RadioButtonUncheckedRounded";
import AutorenewRounded from "@mui/icons-material/AutorenewRounded";
import {
  CodeView,
  CopyTextButton,
  FileChip,
  hasDiff,
  KeyValues,
  Labeled,
  langFromPath,
  OutputBlocks,
  PreBlock,
  ShellCommandView,
  textOfContent,
} from "./blocks";
import { mcpIdentity } from "./presentation";
import { outputPrefersHorizontalScroll } from "./outputLayout";
import { terminalDisplaySegments } from "./terminalHighlight";

// The dispatch layer: given a tool call, pick how to render its body. Two tiers,
// both leaning on the provider-agnostic primitives in blocks.tsx:
//
//  1. `BY_TOOL` — keyed by provider + upstream tool name. Only tools that need bespoke
//     rendering beyond their ACP kind live here (e.g. TodoWrite → a checklist).
//  2. `BY_KIND` — keyed by the ACP `kind`, which the adapters NORMALIZE across
//     providers (codex `shell`→execute, `apply_patch`→edit, …). This is where the
//     bulk of the work happens, so one renderer serves every provider's tools of
//     that kind. Falls back to a generic args+result view for unknown kinds.
//
// The Raw-JSON escape hatch lives in the card shell (Transcript.tsx), so it wraps
// whatever these return and is always one tap away.

export interface ToolCtx {
  /** Session provider; tool names and argument shapes differ across adapters. */
  provider: string;
  /** Upstream tool name (`_meta.<provider>.toolName`), or "" if absent. */
  toolName: string;
  /** ACP kind: read | edit | execute | search | fetch | think | other | … */
  kind: string;
  title: string;
  rawInput: Record<string, unknown>;
  content: unknown;
  running: boolean;
}

type Renderer = (ctx: ToolCtx) => ReactNode;

function RunningHint(): React.JSX.Element {
  return (
    <Stack spacing={0.5}>
      <Skeleton animation="wave" width="80%" />
      <Skeleton animation="wave" width="55%" />
    </Stack>
  );
}

/** The shell command, whether a string (claude `Bash`) or argv array (codex). */
function commandText(raw: Record<string, unknown>): string {
  const c = raw["command"] ?? raw["cmd"];
  if (typeof c === "string") return c;
  if (Array.isArray(c)) return c.map((x) => String(x)).join(" ");
  return "";
}

function terminalText(content: unknown): string {
  const text = textOfContent(content);
  const fenced = text.match(/^```[^\n]*\n([\s\S]*?)\n```\s*$/);
  return fenced?.[1] ?? text;
}

function TerminalOutput({ text, running }: { text: string; running: boolean }): React.JSX.Element {
  const [wrapped, setWrapped] = useState(() => !outputPrefersHorizontalScroll(text));
  const segments = useMemo(() => terminalDisplaySegments(text), [text]);
  useEffect(() => setWrapped(!outputPrefersHorizontalScroll(text)), [text]);
  const control = text
    ? (
      <Box role="group" aria-label="Output line layout" sx={{ display: "flex", bgcolor: "action.hover", borderRadius: 1, p: 0.25 }}>
        {([true, false] as const).map((value) => (
          <ButtonBase
            key={String(value)}
            aria-pressed={wrapped === value}
            onClick={(): void => setWrapped(value)}
            sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: wrapped === value ? "text.primary" : "text.disabled", bgcolor: wrapped === value ? "background.paper" : "transparent", boxShadow: wrapped === value ? 1 : 0 }}
          >
            {value ? "Wrap" : "Scroll"}
          </ButtonBase>
        ))}
      </Box>
    )
    : undefined;
  return (
    <Labeled label="Output" action={control}>
      {text
        ? segments.some((segment) => segment.language)
          ? (
            <Stack spacing={0.5} data-terminal-structured-output>
              {segments.map((segment, index) =>
                segment.language
                  ? (
                    <CodeView
                      key={index}
                      code={segment.text}
                      lang={segment.language}
                      maxHeight={240}
                      wrap={wrapped}
                      wrapControl={false}
                      hideCopy
                    />
                  )
                  : <PreBlock key={index} text={segment.text} wrap={wrapped} />
              )}
            </Stack>
          )
          : <PreBlock text={text} wrap={wrapped} />
        : running
        ? <RunningHint />
        : <Empty />}
    </Labeled>
  );
}

function FileReadContent({
  content,
  language,
}: {
  content: unknown;
  language: string;
}): React.JSX.Element | null {
  const text = terminalText(content);
  const [wrapped, setWrapped] = useState(true);
  if (!text) return null;
  if (language === "markdown") return <OutputBlocks content={content} lang={language} />;
  const control = (
    <Box role="group" aria-label="File line layout" sx={{ display: "flex", bgcolor: "action.hover", borderRadius: 1, p: 0.25 }}>
      {([true, false] as const).map((value) => (
        <ButtonBase
          key={String(value)}
          aria-pressed={wrapped === value}
          onClick={(): void => setWrapped(value)}
          sx={{ minHeight: 28, px: 0.875, borderRadius: 0.75, fontSize: "0.6875rem", color: wrapped === value ? "text.primary" : "text.disabled", bgcolor: wrapped === value ? "background.paper" : "transparent", boxShadow: wrapped === value ? 1 : 0 }}
        >
          {value ? "Wrap" : "Scroll"}
        </ButtonBase>
      ))}
    </Box>
  );
  return (
    <Labeled label={language ? "Source" : "Contents"} action={control}>
      {language
        ? <CodeView code={text} lang={language} maxHeight={420} wrap={wrapped} wrapControl={false} />
        : <PreBlock text={text} maxHeight={420} wrap={wrapped} />}
    </Labeled>
  );
}

// --- kind renderers (provider-agnostic via the normalized ACP kind) ----------

const executeTool: Renderer = ({ rawInput, content, running }) => {
  const cmd = commandText(rawInput);
  const out = terminalText(content);
  const cwd = typeof rawInput["cwd"] === "string" ? rawInput["cwd"] : "";
  return (
    <>
      {cwd && (
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.75 }}>
          {cwd}
        </Typography>
      )}
      {cmd && (
        <Labeled label="Command" action={<CopyTextButton text={cmd} label="Command" />}>
          <ShellCommandView command={cmd} />
        </Labeled>
      )}
      <TerminalOutput text={out} running={running} />
    </>
  );
};

const editTool: Renderer = ({ rawInput, content, running }) => {
  const fallbackPath = String(rawInput["file_path"] ?? rawInput["path"] ?? "");
  const out = <OutputBlocks content={content} fallbackPath={fallbackPath} />;
  if (out && (hasDiff(content) || textOfContent(content))) return out;
  return running ? <RunningHint /> : <Empty />;
};

const readTool: Renderer = ({ rawInput, content, running }) => {
  const path = String(rawInput["file_path"] ?? rawInput["path"] ?? "");
  const lang = path ? langFromPath(path) : "";
  const has = textOfContent(content);
  const offset = rawInput["offset"];
  const limit = rawInput["limit"];
  return (
    <Stack spacing={0.75}>
      {path && (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ flexWrap: "wrap" }}>
          <FileChip path={path} />
          {(offset !== undefined || limit !== undefined) && (
            <Typography variant="caption" color="text.disabled">
              {offset !== undefined ? `from line ${String(offset)}` : ""}
              {limit !== undefined ? ` · ${String(limit)} lines` : ""}
            </Typography>
          )}
        </Stack>
      )}
      {has ? <FileReadContent content={content} language={lang} /> : running ? <RunningHint /> : <Empty />}
    </Stack>
  );
};

const genericTool: Renderer = ({ rawInput, content, running }) => {
  const result = <OutputBlocks content={content} />;
  const hasResult = Boolean(textOfContent(content) || hasDiff(content));
  return (
    <Stack spacing={1}>
      {Object.keys(rawInput).length > 0 && (
        <Labeled label="Arguments">
          <KeyValues data={rawInput} />
        </Labeled>
      )}
      {hasResult
        ? <Labeled label="Result">{result}</Labeled>
        : running
        ? <RunningHint />
        : null}
    </Stack>
  );
};

const searchTool: Renderer = ({ rawInput, content, running }) => {
  const action = rawInput["action"] && typeof rawInput["action"] === "object"
    ? rawInput["action"] as Record<string, unknown>
    : {};
  const query = typeof rawInput["query"] === "string"
    ? rawInput["query"]
    : typeof action["query"] === "string"
    ? action["query"]
    : "";
  const details = Object.fromEntries(
    Object.entries(rawInput).filter(([key]) => !["type", "id", "query", "action"].includes(key)),
  );
  const hasResult = Boolean(textOfContent(content) || hasDiff(content));
  return (
    <Stack spacing={1}>
      {query && (
        <Labeled label="Query" action={<CopyTextButton text={query} label="Query" />}>
          <CodeView code={query} lang="text" maxHeight={160} touchWrap hideCopy />
        </Labeled>
      )}
      {Object.keys(details).length > 0 && (
        <Labeled label="Options"><KeyValues data={details} /></Labeled>
      )}
      {hasResult
        ? <Labeled label="Result"><OutputBlocks content={content} /></Labeled>
        : running
        ? <RunningHint />
        : null}
    </Stack>
  );
};

interface McpWidget {
  primary: string;
  label: string;
  language?: string;
}

// Only tool-specific presentation belongs here. Identity parsing, the argument
// frame, results, loading and failure behavior remain shared across ACPs and MCP
// servers. New MCPs therefore get a useful generic view before a tailored
// primary field is added to this small registry.
const MCP_WIDGETS: Record<string, McpWidget> = {
  "chrome-devtools:evaluate_script": { primary: "function", label: "Script", language: "javascript" },
  "chrome-devtools:navigate_page": { primary: "url", label: "Destination" },
  "chrome-devtools:new_page": { primary: "url", label: "Destination" },
  "openaiDeveloperDocs:search_openai_docs": { primary: "query", label: "Query" },
  "openaiDeveloperDocs:fetch_openai_doc": { primary: "url", label: "Document" },
};

const mcpTool: Renderer = (ctx) => {
  const mcp = mcpIdentity(ctx.toolName, ctx.rawInput, ctx.title);
  if (!mcp) return genericTool(ctx);
  const args = { ...mcp.arguments };
  const widget = MCP_WIDGETS[`${mcp.server}:${mcp.tool}`] ??
    (typeof args.function === "string"
      ? { primary: "function", label: "Script", language: "javascript" }
      : typeof args.query === "string"
      ? { primary: "query", label: "Query" }
      : typeof args.url === "string"
      ? { primary: "url", label: "URL" }
      : undefined);
  const primary = widget && typeof args[widget.primary] === "string" ? String(args[widget.primary]) : "";
  if (widget) delete args[widget.primary];
  const hasResult = Boolean(textOfContent(ctx.content) || hasDiff(ctx.content));
  return (
    <Stack spacing={1}>
      {primary && widget && (
        <Labeled label={widget.label} action={<CopyTextButton text={primary} label={widget.label} />}>
          <CodeView
            code={primary}
            lang={widget.language ?? (widget.primary === "query" ? "text" : "uri")}
            maxHeight={260}
            touchWrap={widget.primary !== "function"}
            hideCopy
          />
        </Labeled>
      )}
      {Object.keys(args).length > 0 && (
        <Labeled label="Arguments">
          <KeyValues data={args} />
        </Labeled>
      )}
      {hasResult
        ? <Labeled label="Result"><OutputBlocks content={ctx.content} /></Labeled>
        : ctx.running
        ? <RunningHint />
        : <Empty />}
    </Stack>
  );
};

function Empty(): React.JSX.Element {
  return <Typography variant="caption" color="text.disabled">No output</Typography>;
}

const BY_KIND: Record<string, Renderer> = {
  execute: executeTool,
  edit: editTool,
  read: readTool,
  search: searchTool,
  // search / fetch / think / delete / move / other → the generic args+result.
};

// --- per-tool overrides (the provider layer; name encodes the provider) ------

const todoTool: Renderer = (ctx) => {
  const todos = ctx.rawInput["todos"];
  if (!Array.isArray(todos)) return genericTool(ctx);
  return (
    <Stack spacing={0.5}>
      {todos.map((t, i) => {
        const todo = t as { content?: string; status?: string };
        const status = todo.status ?? "pending";
        const icon = status === "completed"
          ? <CheckCircleRounded sx={{ fontSize: 16, color: "success.main" }} />
          : status === "in_progress"
          ? <AutorenewRounded sx={{ fontSize: 16, color: "primary.main" }} />
          : <RadioButtonUncheckedRounded sx={{ fontSize: 16, color: "text.disabled" }} />;
        return (
          <Stack key={i} direction="row" spacing={0.75} alignItems="center">
            {icon}
            <Typography
              variant="body2"
              sx={{
                fontSize: "0.85em",
                color: status === "completed" ? "text.disabled" : "text.primary",
                textDecoration: status === "completed" ? "line-through" : "none",
              }}
            >
              {todo.content ?? ""}
            </Typography>
          </Stack>
        );
      })}
    </Stack>
  );
};

const BY_TOOL: Record<string, Renderer> = {
  "claude-code:TodoWrite": todoTool,
  // codex / others can add bespoke renderers here; everything else flows through
  // the kind layer, which already covers shell/apply_patch/read/etc.
};

// A tool renderer formats arbitrary agent-supplied data; a malformed payload
// must never crash the whole transcript. This boundary catches a renderer throw
// and degrades to a hint — the card's Raw toggle still shows the verbatim JSON.
class ToolBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  override state = { failed: false };
  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }
  override render(): ReactNode {
    if (this.state.failed) {
      return (
        <Typography variant="caption" color="text.disabled">
          Couldn&apos;t format this tool — tap “{"{ } Raw"}” above to see the data.
        </Typography>
      );
    }
    return this.props.children;
  }
}

/** Render a tool call's expanded body — the public entry the card shell calls. */
export function ToolBody({ ctx }: { ctx: ToolCtx }): React.JSX.Element {
  const renderer = BY_TOOL[`${ctx.provider}:${ctx.toolName}`] ??
    (mcpIdentity(ctx.toolName, ctx.rawInput, ctx.title) ? mcpTool : undefined) ??
    BY_KIND[ctx.kind] ?? genericTool;
  return (
    <Box>
      <ToolBoundary>{renderer(ctx)}</ToolBoundary>
    </Box>
  );
}
