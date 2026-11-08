import { desktopSize } from "./surface/desktopSize";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Box, Button, ButtonBase, CircularProgress, Dialog, DialogContent, IconButton, keyframes, Skeleton, Stack, Tooltip, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import { desktopImeOwnsKey } from "./desktop/commands/imeShortcut";
import { sequentialShortcutAvailability } from "./desktop/commands/shortcutAvailability";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
import { Close, Code, Construction, ExpandMore, ExtensionOutlined, Folder, KeyboardArrowDown, MyLocation, NavigateBefore, NavigateNext, Search, Terminal } from "@mui/icons-material";
import { Markdown } from "./Markdown";
import { CodeView, Labeled } from "./tools/blocks";
import { ToolBody, type ToolCtx } from "./tools/registry";
import { toolCopyText, toolHeading, toolUsesRawOnly, toolVariantLabel } from "./tools/presentation";
import { type ToolItem, type ToolRun } from "./tools/runs";
import { type ContentChunk, isHumanPrompt, type RenderItem } from "./derive";
import { ToolStatusChip, toolStatusLabel } from "./tools/ToolStatusChip";
import { ScrollEdges, FloatingActionIsland } from "@cowboy/app-shell";
import { Sheet } from "./Sheet";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { scrollableAncestor, useToolDetailReveal } from "./useToolDetailReveal";

export { scrollableAncestor };
export function toolIcon(kind: string): React.ReactElement {
  // The tool-card leading icon tracks the theme accent (`color="primary"`,
  // was the default text colour) and the reading-size setting
  // (`fontSize="inherit"` sizes to the row's em like the title does, instead of
  // a fixed 24px "medium") — so it adapts to both theme and font.
  const props = { fontSize: "inherit" as const, color: "primary" as const };
  switch (kind) {
    case "read":
      return <Folder {...props} />;
    case "edit":
      return <Code {...props} />;
    case "execute":
      return <Terminal {...props} />;
    case "search":
      return <Search {...props} />;
    case "mcp":
      return <ExtensionOutlined {...props} />;
    default:
      return <Construction {...props} />;
  }
}

export type ToolContextBlock = {
  key: string;
  tone: "prose" | "thought" | "user";
  text: string;
};

export type FollowingToolPhase = "settled" | "running" | "ready";

export const nextToolArrival = keyframes`
  0% { opacity: .68; transform: translateY(3px); }
  100% { opacity: 1; transform: translateY(0); }
`;

export function toolContextBlocks(items: RenderItem[]): ToolContextBlock[] {
  const blocks: ToolContextBlock[] = [];
  for (const item of items) {
    let tone: ToolContextBlock["tone"] | null = null;
    let text = "";
    if (item.kind === "message") {
      tone = item.role === "user"
        ? (isHumanPrompt(item.origin) ? "user" : "thought")
        : "prose";
      text = item.chunks
        .filter((chunk): chunk is Extract<ContentChunk, { type: "text" }> =>
          chunk.type === "text"
        )
        .map((chunk) => chunk.text)
        .join("\n\n");
    } else if (item.kind === "thought") {
      tone = "thought";
      text = item.sections.join("\n\n");
    } else if (item.kind === "lifecycle" && item.detail) {
      tone = "thought";
      text = item.detail;
    }
    if (!tone || !text.trim()) continue;
    const previous = blocks.at(-1);
    if (previous?.tone === tone) {
      previous.text += `\n\n${text}`;
    } else {
      blocks.push({ key: item.key, tone, text });
    }
  }
  return blocks;
}

export function ToolTranscriptContext({
  items,
  position,
  phase = "settled",
  defaultOpen = false,
}: {
  items: RenderItem[];
  position: "before" | "after";
  phase?: FollowingToolPhase;
  defaultOpen?: boolean;
}): React.JSX.Element | null {
  const blocks = toolContextBlocks(items);
  const [open, setOpen] = useState(defaultOpen);
  const activeFollowing = position === "after" && phase !== "settled";
  if (blocks.length === 0 && !activeFollowing) return null;
  const label = position === "before" ? "Before this tool" : "After this tool";
  const preview = blocks
    .map((block) =>
      block.text
        .replace(/```[\s\S]*?```/gu, "Code")
        .replace(/[*_~`>#]/gu, "")
        .replaceAll("[", "")
        .replaceAll("]", "")
        .replace(/^[-+]\s+/gu, "")
        .replace(/\s+/gu, " ")
        .trim()
    )
    .filter(Boolean)
    .join(" · ");
  const statusText = phase === "running"
    ? "Working on the next step…"
    : phase === "ready"
    ? "Next tool ready"
    : "";
  const summary = preview || statusText;
  const canExpand = blocks.length > 0;
  return (
    <Box
      aria-label={`${
        position === "before" ? "Previous" : "Following"
      } transcript context`}
      sx={{
        border: 1,
        borderColor: open ? "divider" : "transparent",
        bgcolor: open ? "action.hover" : "transparent",
        borderRadius: 1.5,
        overflow: "hidden",
        transition: "background-color .18s ease, border-color .18s ease",
        ...(phase === "ready" &&
          { animation: `${nextToolArrival} .28s cubic-bezier(.2,.8,.2,1) 1` }),
      }}
    >
      <ButtonBase
        aria-expanded={open}
        disabled={!canExpand}
        onClick={(): void => setOpen((value) => !value)}
        sx={{
          width: "100%",
          minHeight: 48,
          px: 1.25,
          py: 0.75,
          display: "grid",
          gridTemplateColumns: "1fr auto",
          columnGap: 1,
          textAlign: "left",
          alignItems: "center",
        }}
      >
        <Box sx={{ minWidth: 0 }}>
          <Typography
            variant="overline"
            sx={{
              display: "flex",
              alignItems: "center",
              gap: 0.625,
              color: "text.disabled",
              lineHeight: 1.35,
            }}
          >
            {label}
            {phase === "running" && (
              <CircularProgress size={desktopSize(10)} thickness={5} color="inherit" />
            )}
          </Typography>
          {!open && (
            <Typography
              variant="body2"
              sx={{
                mt: 0.25,
                color: "text.secondary",
                fontWeight: 600,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {summary}
            </Typography>
          )}
        </Box>
        {canExpand && (
          <ExpandMore
            sx={{
              color: "text.secondary",
              transform: open ? "rotate(180deg)" : "none",
              transition: "transform .2s cubic-bezier(.2,.8,.2,1)",
            }}
          />
        )}
      </ButtonBase>
      {open && (
        <Stack
          spacing={1}
          sx={{
            px: 1.25,
            pb: 1.25,
          }}
        >
          {blocks.map((block) => (
            <Box
              key={block.key}
              sx={block.tone === "user"
                ? {
                  bgcolor: "action.selected",
                  borderRadius: 1.25,
                  px: 1,
                  py: 0.5,
                  color: "text.primary",
                }
                : block.tone === "thought"
                ? { fontSize: "0.88em", color: "text.secondary" }
                : { color: "text.primary" }}
            >
              <Markdown text={block.text} />
            </Box>
          ))}
        </Stack>
      )}
    </Box>
  );
}

export function ToolDetailsBrowser({
  items,
  runs,
  selectedKey,
  desktop,
  provider,
  providerVersion,
  providerDigest,
  onSelect,
  onClose,
  onLocate,
  historyComplete,
}: {
  items: RenderItem[];
  runs: ToolRun[];
  selectedKey: string | null;
  desktop: boolean;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  onSelect: (key: string) => void;
  onClose: () => void;
  onLocate: (key: string) => void;
  historyComplete: boolean;
}): React.JSX.Element | null {
  const runIndex = selectedKey === null
    ? -1
    : runs.findIndex((candidate) =>
      candidate.tools.some((tool) => tool.key === selectedKey)
    );
  const run = runIndex >= 0 ? runs[runIndex] : undefined;
  const itemIndex = run?.tools.findIndex((tool) => tool.key === selectedKey) ??
    -1;
  const item = itemIndex >= 0 ? run?.tools[itemIndex] : undefined;
  const [rawByKey, setRawByKey] = useState<Record<string, boolean>>({});
  const bodyRef = useRef<HTMLDivElement>(null);
  const currentRef = useRef<HTMLDivElement>(null);
  const anchorSpacerRef = useRef<HTMLDivElement>(null);
  const historyRef = useRef<HTMLElement>(null);
  const scrollByKey = useRef(new Map<string, number>());
  const navigationChord = useRef<number | null>(null);
  const [navigationPrefixArmed, setNavigationPrefixArmed] = useState(false);
  const rawOnly = item
    ? toolUsesRawOnly({
      kind: item.toolKind,
      toolName: item.toolName,
      title: item.title,
      rawInput: item.rawInput,
    })
    : false;

  const { detailReadyKey, markDetailReady } = useToolDetailReveal({
    itemKey: item?.key,
    bodyRef,
    currentRef,
    anchorSpacerRef,
    scrollByKey,
  });

  useLayoutEffect(() => {
    if (!desktop || runIndex < 0) return;
    historyRef.current
      ?.querySelector<HTMLElement>(`[data-tool-run-index="${runIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [desktop, runIndex]);

  const select = (next: ToolItem | undefined): void => {
    if (!next || !item) return;
    const scroller = scrollableAncestor(bodyRef.current);
    if (scroller) {
      scrollByKey.current.set(item.key, scroller.scrollTop);
      if (run?.tools.some((tool) => tool.key === next.key)) {
        // A continuous MCP run is one browser page. Selecting one of its rows
        // swaps the page's active call in place: retain the run list's viewport
        // and skip the delayed-detail skeleton. Only cross-run navigation gets
        // the normal focus/anchor alignment performed by the layout effect.
        scrollByKey.current.set(next.key, scroller.scrollTop);
        markDetailReady(next.key);
      }
    }
    onSelect(next.key);
  };

  useEffect(() => {
    if (!desktop || !item) return undefined;
    const clearNavigationChord = (): void => {
      if (navigationChord.current !== null) {
        globalThis.clearTimeout(navigationChord.current);
        navigationChord.current = null;
      }
      setNavigationPrefixArmed(false);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (desktopImeOwnsKey(event)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) {
        clearNavigationChord();
        return;
      }
      const target = event.target instanceof Element ? event.target : null;
      if (target?.matches("input, textarea, [contenteditable='true']")) return;
      const key = workspaceCommandKey(event);
      if (navigationChord.current !== null) {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        clearNavigationChord();
        if (key === "g") select(runs[0]?.tools[0]);
        return;
      }
      if (key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (key === "[") {
        event.preventDefault();
        select(runs[runIndex - 1]?.tools.at(-1));
      } else if (key === "]") {
        event.preventDefault();
        select(runs[runIndex + 1]?.tools[0]);
      } else if (key === "j") {
        event.preventDefault();
        select(runs[Math.min(runs.length - 1, runIndex + 1)]?.tools[0]);
      } else if (key === "k") {
        event.preventDefault();
        select(runs[Math.max(0, runIndex - 1)]?.tools.at(-1));
      } else if (!rawOnly && (key === "h" || key === "l")) {
        event.preventDefault();
        setRawByKey((state) => ({ ...state, [item.key]: key === "l" }));
      } else if (key === "Enter") {
        event.preventDefault();
        onLocate(item.key);
      } else if (key === "g") {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        setNavigationPrefixArmed(true);
        navigationChord.current = globalThis.setTimeout(() => {
          navigationChord.current = null;
          setNavigationPrefixArmed(false);
        }, 1200);
      } else if (key === "G") {
        event.preventDefault();
        select(runs.at(-1)?.tools.at(-1));
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      clearNavigationChord();
    };
  }, [desktop, runIndex, item, rawOnly, runs, onClose, onLocate]);

  if (!item) return null;
  // A search query has no useful alternate presentation: formatting it only
  // duplicates the query while adding two layers of layout controls. Keep the
  // transport JSON as the single, inspectable representation so input and
  // output retain their exact shape across Codex, Claude and future ACPs.
  const raw = rawOnly || (rawByKey[item.key] ?? false);
  const firstRunItemIndex = items.findIndex((candidate) =>
    candidate.key === run?.tools[0]?.key
  );
  const lastRunItemIndex = items.findIndex((candidate) =>
    candidate.key === run?.tools.at(-1)?.key
  );
  const previousToolIndex = runIndex > 0
    ? items.findIndex((candidate) =>
      candidate.key === runs[runIndex - 1]?.tools.at(-1)?.key
    )
    : -1;
  const nextToolIndex = runIndex < runs.length - 1
    ? items.findIndex((candidate) =>
      candidate.key === runs[runIndex + 1]?.tools[0]?.key
    )
    : items.length;
  const before = items.slice(previousToolIndex + 1, firstRunItemIndex);
  const after = items.slice(lastRunItemIndex + 1, nextToolIndex);
  const running = item.status === "in_progress" || item.status === "pending";
  const ctx: ToolCtx = {
    provider,
    providerVersion,
    providerDigest,
    toolName: item.toolName,
    kind: item.toolKind,
    title: item.title,
    rawInput: item.rawInput && typeof item.rawInput === "object" &&
        !Array.isArray(item.rawInput)
      ? (item.rawInput as Record<string, unknown>)
      : {},
    content: item.content,
    running,
    status: item.status,
  };
  const heading = toolHeading({
    provider,
    toolName: item.toolName,
    kind: item.toolKind,
    title: item.title,
    rawInput: item.rawInput,
  });
  const runTitle = run?.server
    ? run.server.split(/[-_]/u).filter(Boolean).map((word) =>
      word.charAt(0).toUpperCase() + word.slice(1)
    ).join(" ")
    : null;

  const mobileNavigationItems = (
    <>
      <IconButton
        aria-label="Previous tool run"
        disabled={runIndex === 0}
        onClick={(): void => select(runs[runIndex - 1]?.tools.at(-1))}
        sx={{ width: 44, height: 44, justifySelf: "start" }}
      >
        <NavigateBefore />
      </IconButton>
      <Typography
        aria-label={`Tool run ${runIndex + 1} of ${runs.length}${
          historyComplete ? "" : " loaded"
        }`}
        variant="caption"
        sx={{
          px: 0.75,
          fontWeight: 700,
          color: "text.secondary",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {runIndex + 1} / {runs.length}
        {historyComplete ? "" : "+"}
      </Typography>
      <Tooltip title="Close details">
        <IconButton
          aria-label="Close tool details"
          onClick={onClose}
          sx={{ width: 44, height: 44, justifySelf: "center" }}
        >
          <Close />
        </IconButton>
      </Tooltip>
      <IconButton
        aria-label="Next tool run"
        disabled={runIndex >= runs.length - 1}
        onClick={(): void => select(runs[runIndex + 1]?.tools[0])}
        sx={{ width: 44, height: 44, justifySelf: "end" }}
      >
        <NavigateNext />
      </IconButton>
      <Tooltip title="Locate in transcript">
        <IconButton
          aria-label="Locate tool in transcript"
          onClick={(): void => onLocate(item.key)}
          sx={{ width: 44, height: 44 }}
        >
          <MyLocation fontSize="small" />
        </IconButton>
      </Tooltip>
    </>
  );

  const navigation = (
    <Box sx={{ width: "min(100%, 282px)", mx: "auto" }}>
      <FloatingActionIsland columns="44px minmax(58px, 1fr) 44px 44px 44px">
        {mobileNavigationItems}
      </FloatingActionIsland>
    </Box>
  );

  const details = (
    <Box
      ref={bodyRef}
      sx={{
        position: "relative",
        maxWidth: desktop ? 1280 : "none",
        mx: desktop ? "auto" : 0,
      }}
    >
      <Stack spacing={1.25}>
        <ToolTranscriptContext
          key={`${item.key}-before`}
          items={before}
          position="before"
          defaultOpen={desktop}
        />
        {run && run.tools.length > 1 && (
          <Box
            aria-label={`${
              runTitle ?? "MCP"
            } run with ${run.tools.length} calls`}
            sx={{
              border: 1,
              borderColor: "divider",
              borderRadius: 1.5,
              overflow: "hidden",
              bgcolor: "action.hover",
            }}
          >
            <Stack
              direction="row"
              alignItems="baseline"
              justifyContent="space-between"
              sx={{ px: 1.25, pt: 0.875, pb: 0.5 }}
            >
              <Typography
                variant="caption"
                sx={{ color: "text.secondary", fontWeight: 700 }}
              >
                {runTitle}
              </Typography>
              <Typography
                variant="caption"
                sx={{
                  color: "text.disabled",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {itemIndex + 1} / {run.tools.length} calls
              </Typography>
            </Stack>
            <Stack sx={{ pb: 0.375 }}>
              {run.tools.map((tool, callIndex) => {
                const selected = tool.key === item.key;
                const callHeading = toolHeading({
                  provider,
                  toolName: tool.toolName,
                  kind: tool.toolKind,
                  title: tool.title,
                  rawInput: tool.rawInput,
                });
                return (
                  <ButtonBase
                    key={tool.key}
                    aria-current={selected ? "step" : undefined}
                    onClick={(): void => select(tool)}
                    sx={{
                      width: "100%",
                      minHeight: 42,
                      px: 1.25,
                      display: "grid",
                      gridTemplateColumns: "20px minmax(0, 1fr) auto",
                      gap: 0.75,
                      alignItems: "center",
                      textAlign: "left",
                      bgcolor: selected ? "action.selected" : "transparent",
                    }}
                  >
                    <Typography
                      variant="caption"
                      sx={{
                        color: selected ? "primary.main" : "text.disabled",
                        fontWeight: 700,
                      }}
                    >
                      {callIndex + 1}
                    </Typography>
                    <Typography
                      variant="body2"
                      noWrap
                      sx={{ fontWeight: selected ? 700 : 500 }}
                    >
                      {callHeading}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: tool.status === "failed"
                          ? "error.main"
                          : tool.status === "in_progress" ||
                              tool.status === "pending"
                          ? "warning.main"
                          : "text.disabled",
                      }}
                    >
                      {toolStatusLabel(tool.status)}
                    </Typography>
                  </ButtonBase>
                );
              })}
            </Stack>
          </Box>
        )}
        <Stack ref={currentRef} direction="row" spacing={1} alignItems="center">
          <Box sx={{ pt: 0.25, color: "text.secondary" }}>
            {toolIcon(run?.server ? "mcp" : item.toolKind)}
          </Box>
          <Typography
            sx={{
              flex: 1,
              minWidth: 0,
              overflowWrap: "anywhere",
              fontFamily: !run?.server && item.toolKind === "execute"
                ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                : "var(--cowboy-reading-font, inherit)",
            }}
          >
            {heading}
          </Typography>
          <ToolStatusChip status={item.status} />
        </Stack>
        <Box sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}>
          {!rawOnly && (
            <Stack
              direction="row"
              alignItems="center"
              justifyContent="space-between"
              sx={{ mb: 0.75, flexWrap: "wrap", gap: 0.75 }}
            >
              <Typography variant="caption" color="text.disabled">
                {toolVariantLabel({
                  provider,
                  providerVersion,
                  providerDigest,
                  toolName: item.toolName,
                  kind: item.toolKind,
                  title: item.title,
                  rawInput: item.rawInput,
                })}
              </Typography>
              <Stack direction="row" alignItems="center" spacing={0.25}>
                <Box
                  role="group"
                  aria-label="Tool detail view"
                  sx={{
                    display: "flex",
                    p: 0.25,
                    borderRadius: 1.25,
                    bgcolor: "action.hover",
                  }}
                >
                  {[false, true].map((isRaw) => (
                    <ButtonBase
                      key={String(isRaw)}
                      aria-pressed={raw === isRaw}
                      onClick={(): void =>
                        setRawByKey((state) => ({
                          ...state,
                          [item.key]: isRaw,
                        }))}
                      sx={{
                        minHeight: 32,
                        "@media (pointer: coarse)": { minHeight: 44 },
                        px: 1,
                        borderRadius: 1,
                        fontSize: "0.6875rem",
                        fontWeight: raw === isRaw ? 700 : 500,
                        color: raw === isRaw ? "text.primary" : "text.disabled",
                        bgcolor: raw === isRaw
                          ? "background.paper"
                          : "transparent",
                        boxShadow: raw === isRaw ? 1 : 0,
                      }}
                    >
                      {isRaw ? "Raw" : "Formatted"}
                    </ButtonBase>
                  ))}
                </Box>
              </Stack>
            </Stack>
          )}
          {detailReadyKey !== item.key
            ? (
              <Stack spacing={0.75} aria-label="Loading tool details">
                <Skeleton animation="wave" width="92%" />
                <Skeleton animation="wave" width="78%" />
                <Skeleton animation="wave" width="86%" />
              </Stack>
            )
            : raw
            ? (
              <Stack spacing={1}>
                {item.rawInput !== undefined && (
                  <Labeled label="Input">
                    <CodeView
                      code={JSON.stringify(item.rawInput, null, 2) ??
                        String(item.rawInput)}
                      lang="json"
                      maxHeight={desktop ? 640 : 360}
                      touchWrap={!rawOnly}
                    />
                  </Labeled>
                )}
                {item.content !== undefined && (
                  <Labeled label="Output">
                    <CodeView
                      code={JSON.stringify(item.content, null, 2) ??
                        String(item.content)}
                      lang="json"
                      maxHeight={desktop ? 640 : 360}
                      touchWrap={!rawOnly}
                    />
                  </Labeled>
                )}
                {item.content === undefined && (
                  <Typography variant="body2" color="text.secondary" role="status">
                    {running ? "Waiting for tool result…" : "No result was recorded for this call."}
                  </Typography>
                )}
              </Stack>
            )
            : (
              <Box>
                <ToolBody key={item.key} ctx={ctx} />
              </Box>
            )}
        </Box>
        <ToolTranscriptContext
          key={`${item.key}-after`}
          items={after}
          position="after"
          phase={running
            ? "running"
            : runIndex < runs.length - 1
            ? "ready"
            : "settled"}
          defaultOpen={desktop}
        />
        <Box ref={anchorSpacerRef} aria-hidden />
      </Stack>
    </Box>
  );

  const desktopHistory = (
    <Box
      component="nav"
      ref={historyRef}
      aria-label="Tool run history"
      sx={{
        minWidth: 0,
        minHeight: 0,
        overflowY: "auto",
        borderRight: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.default, 0.52),
        userSelect: "none",
      }}
    >
      <Box
        sx={{
          position: "sticky",
          top: 0,
          zIndex: 1,
          px: 1.5,
          py: 1.25,
          bgcolor: "background.paper",
          borderBottom: 1,
          borderColor: "divider",
        }}
      >
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ fontWeight: 800 }}
        >
          Agent history
        </Typography>
        <Typography variant="caption" display="block" color="text.disabled">
          {runs.length} runs{historyComplete ? "" : "+"}{" "}
          · newest agent activity last
        </Typography>
      </Box>
      <Stack sx={{ py: 0.5 }}>
        {runs.map((candidate, candidateIndex) => {
          const active = candidateIndex === runIndex;
          const candidateTool = candidate.tools[0];
          if (!candidateTool) return null;
          const candidateHeading = toolHeading({
            provider,
            toolName: candidateTool.toolName,
            kind: candidateTool.toolKind,
            title: candidateTool.title,
            rawInput: candidateTool.rawInput,
          });
          const candidateSummary = toolCopyText({
            title: candidateTool.title,
            toolName: candidateTool.toolName,
            rawInput: candidateTool.rawInput,
          }).split("\n", 1)[0]?.replace(/\s+/g, " ").trim() || candidateHeading;
          return (
            <ButtonBase
              key={candidate.key}
              data-tool-run-index={candidateIndex}
              aria-current={active ? "step" : undefined}
              onClick={(): void => select(candidateTool)}
              sx={{
                minHeight: 50,
                px: 1.5,
                py: 0.75,
                display: "grid",
                gridTemplateColumns: "32px minmax(0, 1fr) auto",
                gap: 1,
                alignItems: "center",
                textAlign: "left",
                bgcolor: active ? "action.selected" : "transparent",
                "&:hover": { bgcolor: "action.hover" },
                "&:focus-visible": {
                  outline: "2px solid",
                  outlineColor: "primary.main",
                  outlineOffset: -2,
                },
              }}
            >
              <Typography
                variant="caption"
                color={active ? "primary.main" : "text.disabled"}
                sx={{ fontWeight: 800, fontVariantNumeric: "tabular-nums" }}
              >
                {candidateIndex + 1}
              </Typography>
              <Box sx={{ minWidth: 0 }}>
                <Typography
                  variant="body2"
                  noWrap
                  sx={{
                    fontFamily: candidateTool.toolKind === "execute"
                      ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                      : "inherit",
                    fontWeight: active ? 750 : 550,
                  }}
                >
                  {candidateSummary}
                </Typography>
                {/* A caption is an inline span: without block display noWrap
                    cannot ellipsize, so a long description widened the rail and
                    scrollIntoView panned the whole history sideways. */}
                <Typography
                  variant="caption"
                  color="text.disabled"
                  display="block"
                  noWrap
                >
                  {candidateHeading}
                  {candidate.tools.length > 1
                    ? ` · ${candidate.tools.length} calls`
                    : ""}
                </Typography>
              </Box>
              <Box
                sx={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  bgcolor: candidateTool.status === "failed"
                    ? "error.main"
                    : candidateTool.status === "in_progress" ||
                        candidateTool.status === "pending"
                    ? "warning.main"
                    : "success.main",
                }}
              />
            </ButtonBase>
          );
        })}
      </Stack>
    <ScrollEdges top={false} /></Box>
  );

  return createPortal(
    desktop
      ? (
        <Dialog
          open
          onClose={onClose}
          maxWidth={false}
          fullWidth
          PaperProps={{
            sx: {
              width: "calc(100vw - 32px)",
              maxWidth: 1600,
              height: "calc(100dvh - 32px)",
              maxHeight: 1100,
              m: 2,
              overflow: "hidden",
              borderRadius: 2.5,
              backgroundImage: "none",
            },
          }}
        >
          <Box
            sx={{
              height: "100%",
              minHeight: 0,
              display: "grid",
              gridTemplateRows: "auto minmax(0, 1fr) auto",
            }}
          >
            <Box
              sx={{
                px: 2.5,
                py: 1.25,
                display: "grid",
                gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)",
                alignItems: "center",
                gap: 2.5,
                borderBottom: 1,
                borderColor: "divider",
                userSelect: "none",
              }}
            >
              <Box>
                <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>
                  Tool history inspector
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  Review what the agent ran, changed, and observed
                </Typography>
              </Box>
              <Stack
                direction="row"
                alignItems="center"
                spacing={1.25}
                sx={{ minWidth: 0 }}
              >
                <Box sx={{ minWidth: 0, flex: 1 }}>
                  <Typography variant="body2" noWrap sx={{ fontWeight: 750 }}>
                    {heading}
                  </Typography>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ fontVariantNumeric: "tabular-nums" }}
                  >
                    Run {runIndex + 1} of {runs.length}
                    {historyComplete ? "" : "+"}
                    {runTitle ? ` · ${runTitle}` : ""}
                  </Typography>
                </Box>
                <Button
                  disabled={runIndex === 0}
                  onClick={(): void => select(runs[runIndex - 1]?.tools.at(-1))}
                  startIcon={<NavigateBefore />}
                  sx={{ textTransform: "none", flexShrink: 0 }}
                >
                  Previous
                </Button>
                <Button
                  disabled={runIndex >= runs.length - 1}
                  onClick={(): void => select(runs[runIndex + 1]?.tools[0])}
                  endIcon={<NavigateNext />}
                  sx={{ textTransform: "none", flexShrink: 0 }}
                >
                  Next
                </Button>
                <Tooltip title="Locate in transcript">
                  <IconButton
                    aria-label="Locate tool in transcript"
                    onClick={(): void => onLocate(item.key)}
                  >
                    <MyLocation fontSize="small" />
                  </IconButton>
                </Tooltip>
                <Button
                  onClick={onClose}
                  endIcon={<KeyboardArrowDown />}
                  sx={{ textTransform: "none", flexShrink: 0 }}
                >
                  Close
                </Button>
              </Stack>
            </Box>
            <Box
              sx={{
                minHeight: 0,
                display: "grid",
                gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)",
              }}
            >
              {desktopHistory}
              <DialogContent
                sx={{
                  minHeight: 0,
                  overflowY: "auto",
                  px: { md: 3, xl: 4 },
                  py: 2.5,
                }}
              >
                {details}
              <ScrollEdges /></DialogContent>
            </Box>
            <DesktopShortcutBar
              groups={[
                {
                  label: "Navigate",
                  slots: [
                    {
                      shortcut: "J/K",
                      label: "Run",
                      availability: navigationPrefixArmed
                        ? "inactive"
                        : "available",
                    },
                    {
                      shortcut: "H/L",
                      label: "View",
                      availability: rawOnly || navigationPrefixArmed
                        ? "inactive"
                        : "available",
                    },
                    {
                      shortcut: "Enter",
                      label: "Locate",
                      availability: navigationPrefixArmed
                        ? "inactive"
                        : "available",
                    },
                  ],
                },
                {
                  label: "Go",
                  slots: [
                    {
                      shortcut: "G",
                      label: "Prefix",
                      availability: sequentialShortcutAvailability({
                        scopeAvailable: true,
                        armed: navigationPrefixArmed,
                        prefix: true,
                      }),
                    },
                    {
                      shortcut: "G",
                      label: "Oldest",
                      availability: sequentialShortcutAvailability({
                        scopeAvailable: true,
                        armed: navigationPrefixArmed,
                        prefix: false,
                      }),
                    },
                    {
                      shortcut: "Shift+G",
                      label: "Latest",
                      availability: navigationPrefixArmed
                        ? "inactive"
                        : "available",
                    },
                  ],
                },
                {
                  slots: [{
                    shortcut: "Esc",
                    label: navigationPrefixArmed ? "Cancel prefix" : "Close",
                  }],
                },
              ]}
            />
          </Box>
        </Dialog>
      )
      : (
        <Sheet
          open
          onClose={onClose}
          title="Tool details" hideMobileTitle
          wide
          forceSheet
          cover
          mobileDismiss="none"
          actions={navigation}
        >
          {details}
        </Sheet>
      ),
    document.body,
  );
}
