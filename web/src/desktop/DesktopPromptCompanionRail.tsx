// Adaptive Prompt Companion Rail: when the Desktop Prompt pane has enough
// width (>= 580px) and holds auxiliary state (Plan, Queue, Drafts, Staged
// Review Notes, or Active Tasks), places those panels in a calm vertical side
// column beside the writing canvas instead of stacking them above the editor.
//
// IMPORTANT: The writing canvas (`children`), `planSlot`, and `pendingSlot`
// ALWAYS stay under the exact same React parent elements regardless of width,
// layout toggle, or companion item count; placement switches purely with CSS
// flexbox `flexDirection` + `order`, so live Prompt / Draft edits never remount.

import { persisted, useStore } from "@cowboy/state-store";
import {
  AccountTreeOutlined,
  Close,
  RateReviewOutlined,
  SendOutlined,
  ViewSidebarOutlined,
} from "@mui/icons-material";
import { alpha, Box, Button, ButtonBase, Chip, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { type ReactNode, type RefObject, useLayoutEffect, useState } from "react";
import { OPEN_BACKGROUND_TASKS_EVENT } from "../backgroundTaskInspectorState";
import { desktopSize } from "../surface/desktopSize";
import { useSessionTasksAvailability } from "../tasks/taskEvents";
import { desktopScrollbarSx } from "./desktopScrollbar";
import {
  formatCommentLocation,
  removeReviewComment,
  type StagedReviewComment,
  submitReviewCommentsToPrompt,
  useReviewComments,
  useReviewCommentsUnsaved,
} from "./review/reviewCommentsStore";

export const PROMPT_COMPANION_SPLIT_MIN_PX = 580;

const companionLayoutPref = persisted<"auto" | "stack">(
  "cowboy:desktop.prompt_companion_layout",
  "auto",
  {
    serialize: (value) => value,
    deserialize: (raw) => raw === "stack" ? "stack" : "auto",
  },
);

export function usePromptCompanionWidth(ref: RefObject<HTMLDivElement | null>): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return undefined;
    const measure = (): void => {
      const next = Math.round(node.getBoundingClientRect().width);
      if (next > 0) setWidth(next);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

export function usePromptCompanionMode(width: number, hasContent: boolean): {
  split: boolean;
  canSplit: boolean;
  toggle: () => void;
} {
  const pref = useStore(companionLayoutPref);
  const canSplit = hasContent && width >= PROMPT_COMPANION_SPLIT_MIN_PX;
  return {
    split: canSplit && pref === "auto",
    canSplit,
    toggle: () => companionLayoutPref.set(pref === "auto" ? "stack" : "auto"),
  };
}

function ReviewCommentsTray({
  sessionId,
  comments,
  unsaved,
  compact,
  onFocusPrompt,
}: {
  sessionId: string;
  comments: readonly StagedReviewComment[];
  unsaved: boolean;
  compact: boolean;
  onFocusPrompt: () => "focused" | "deferred" | void;
}): React.JSX.Element | null {
  if (comments.length === 0) return null;
  const submit = (): void => {
    submitReviewCommentsToPrompt(sessionId, onFocusPrompt);
  };
  if (compact) {
    return (
      <Box
        data-desktop-review-notes-strip
        sx={(theme) => ({
          display: "flex",
          alignItems: "center",
          gap: 0.75,
          px: 1.25,
          py: 0.5,
          borderBottom: 1,
          borderColor: "divider",
          bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.08 : 0.04),
          flexShrink: 0,
        })}
      >
        <RateReviewOutlined sx={{ fontSize: "0.95rem", color: "primary.main", flexShrink: 0 }} />
        <Typography variant="caption" noWrap sx={{ flex: 1, fontWeight: 600 }}>
          {comments.length === 1 ? "1 staged review note" : `${String(comments.length)} staged review notes`}
        </Typography>
        {unsaved && <Chip size="small" label="Memory only" color="warning" variant="outlined" sx={{ height: 18, fontSize: "0.64rem", flexShrink: 0 }} />}
        <Button
          size="small"
          variant="text"
          data-desktop-review-notes-submit
          onClick={submit}
          startIcon={<SendOutlined sx={{ fontSize: "0.85rem !important" }} />}
          sx={{ textTransform: "none", fontSize: "0.72rem", py: 0.15, minHeight: desktopSize(24) }}
        >
          Add to Prompt
        </Button>
      </Box>
    );
  }
  return (
    <Box
      data-desktop-review-notes-card
      sx={{
        borderBottom: 1,
        borderColor: "divider",
        px: 1.25,
        py: 1,
      }}
    >
      <Stack direction="row" alignItems="center" spacing={0.5} sx={{ mb: 0.75 }}>
        <RateReviewOutlined sx={{ fontSize: "0.9rem", color: "primary.main" }} />
        <Typography variant="caption" sx={{ fontWeight: 700, flex: 1 }}>
          Review notes ({String(comments.length)})
        </Typography>
        {unsaved && <Chip size="small" label="Memory only" color="warning" variant="outlined" sx={{ height: 18, fontSize: "0.64rem", flexShrink: 0 }} />}
        <Button
          size="small"
          variant="contained"
          disableElevation
          data-desktop-review-notes-submit
          onClick={submit}
          sx={{
            textTransform: "none",
            fontSize: "0.7rem",
            px: 1,
            py: 0.15,
            minHeight: desktopSize(22),
            borderRadius: "6px",
          }}
        >
          Send all
        </Button>
      </Stack>
      <Stack spacing={0.5}>
        {comments.map((item) => {
          const shortLoc = formatCommentLocation(item);
          const fileName = shortLoc.split("/").at(-1) ?? shortLoc;
          return (
            <Box
              key={item.id}
              data-desktop-review-note-item={item.id}
              sx={(theme) => ({
                px: 0.75,
                py: 0.5,
                borderRadius: "6px",
                bgcolor: alpha(theme.palette.action.hover, theme.palette.mode === "dark" ? 0.45 : 0.7),
                display: "flex",
                alignItems: "flex-start",
                gap: 0.5,
              })}
            >
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Typography variant="caption" noWrap sx={{ display: "block", fontFamily: "monospace", fontSize: "0.68rem", color: "primary.main", fontWeight: 600 }}>
                  {fileName}
                </Typography>
                <Typography variant="caption" sx={{ display: "block", color: "text.primary", fontSize: "0.72rem", lineHeight: 1.35, overflowWrap: "anywhere" }}>
                  {item.body}
                </Typography>
              </Box>
              <IconButton
                size="small"
                aria-label="Remove staged review note"
                onClick={(): void => removeReviewComment(sessionId, item.id)}
                sx={{ p: 0.25, color: "text.secondary" }}
              >
                <Close sx={{ fontSize: "0.8rem" }} />
              </IconButton>
            </Box>
          );
        })}
      </Stack>
    </Box>
  );
}

function TasksPulseCard({ sessionId }: { sessionId: string }): React.JSX.Element | null {
  const tasks = useSessionTasksAvailability(sessionId);
  if (!tasks.hasTasks) return null;
  const parts: string[] = [];
  if (tasks.needsYouCount > 0) parts.push(`${String(tasks.needsYouCount)} needs you`);
  if (tasks.activeCount > 0) parts.push(`${String(tasks.activeCount)} running`);
  if (tasks.failedUnseenCount > 0) parts.push(`${String(tasks.failedUnseenCount)} failed`);
  const tone = tasks.needsYouCount > 0
    ? "warning.main"
    : tasks.failedUnseenCount > 0
    ? "error.main"
    : tasks.activeCount > 0
    ? "info.main"
    : "text.secondary";
  return (
    <ButtonBase
      data-desktop-companion-tasks-pulse
      onClick={(): void => {
        // Use the session-scoped Background Tasks opener (`␣G`) so a folded
        // work pane unfolds and receives focus instead of switching silently.
        globalThis.dispatchEvent(
          new CustomEvent(OPEN_BACKGROUND_TASKS_EVENT, { detail: { sessionId } }),
        );
      }}
      sx={{
        width: "100%",
        px: 1.25,
        py: 0.85,
        gap: 0.75,
        justifyContent: "flex-start",
        textAlign: "left",
        borderBottom: 1,
        borderColor: "divider",
        transition: "background-color 120ms ease",
        "&:hover": { bgcolor: "action.hover" },
      }}
    >
      <AccountTreeOutlined sx={{ fontSize: "0.95rem", color: tone, flexShrink: 0 }} />
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Typography variant="caption" sx={{ display: "block", fontWeight: 650, color: "text.primary", lineHeight: 1.2 }}>
          Background tasks
        </Typography>
        <Typography variant="caption" noWrap sx={{ display: "block", color: tone, fontSize: "0.7rem" }}>
          {parts.length > 0 ? parts.join(" · ") : "History available · ␣G"}
        </Typography>
      </Box>
    </ButtonBase>
  );
}

export function DesktopPromptCompanionLayout({
  sessionId,
  containerRef,
  planSlot,
  pendingSlot,
  hasPending,
  onFocusPrompt,
  children,
}: {
  sessionId: string;
  containerRef: RefObject<HTMLDivElement | null>;
  planSlot?: ReactNode;
  pendingSlot: ReactNode;
  hasPending: boolean;
  onFocusPrompt: () => "focused" | "deferred" | void;
  children: ReactNode;
}): React.JSX.Element {
  const comments = useReviewComments(sessionId);
  const commentsUnsaved = useReviewCommentsUnsaved(sessionId);
  const tasks = useSessionTasksAvailability(sessionId);
  const hasCompanionContent = Boolean(planSlot) || hasPending || comments.length > 0 || tasks.hasTasks;
  const width = usePromptCompanionWidth(containerRef);
  const { split, canSplit, toggle } = usePromptCompanionMode(width, hasCompanionContent);

  return (
    <Box
      ref={containerRef}
      data-desktop-prompt-layout={split ? "split" : "stack"}
      sx={{
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        display: "flex",
        flexDirection: split ? "row" : "column",
        overflow: "hidden",
        position: "relative",
        ...(split && {
          "& [data-desktop-prompt-companion='column'] [data-desktop-aux-list]": {
            maxHeight: "min(44vh, 420px) !important",
            paddingBottom: "4px !important",
          },
        }),
      }}
    >
      {/* Stable primary writing-canvas slot: NEVER changes React parent when
          crossing 580px, toggling layout, or gaining companion content. */}
      <Box
        data-desktop-prompt-canvas-slot
        sx={{
          order: split ? 1 : 2,
          flex: 1,
          minWidth: 0,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          position: "relative",
        }}
      >
        {children}
      </Box>
      {/* Stable companion panel slot: CSS order & styling switch between
          stacked top strip and right-hand column without remounting children. */}
      <Box
        data-desktop-prompt-companion={split ? "column" : "stack"}
        sx={(theme) => ({
          order: split ? 2 : 1,
          display: hasCompanionContent ? "flex" : "none",
          flexDirection: "column",
          flexShrink: 0,
          minHeight: 0,
          ...(split
            ? {
              width: "clamp(220px, 34%, 272px)",
              borderLeft: 1,
              borderColor: "divider",
              bgcolor: alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.14 : 0.015),
              overflowY: "auto",
              ...desktopScrollbarSx,
            }
            : {
              width: "100%",
              overflow: "visible",
            }),
        })}
      >
        <Stack
          direction="row"
          alignItems="center"
          sx={{
            display: split ? "flex" : "none",
            px: 1.25,
            minHeight: desktopSize(30),
            borderBottom: 1,
            borderColor: "divider",
            flexShrink: 0,
          }}
        >
          <Typography variant="caption" sx={{ fontWeight: 700, color: "text.secondary", letterSpacing: "0.04em", flex: 1 }}>
            WORKSPACE
          </Typography>
          <Tooltip title="Switch to single-column stack">
            <IconButton
              size="small"
              aria-label="Switch companion to stacked layout"
              data-desktop-companion-toggle
              onClick={toggle}
              sx={{ p: 0.35, color: "text.secondary" }}
            >
              <ViewSidebarOutlined sx={{ fontSize: "0.95rem" }} />
            </IconButton>
          </Tooltip>
        </Stack>
        {planSlot}
        <ReviewCommentsTray sessionId={sessionId} comments={comments} unsaved={commentsUnsaved} compact={!split} onFocusPrompt={onFocusPrompt} />
        {pendingSlot}
        <Box sx={{ display: split ? "block" : "none" }}>
          <TasksPulseCard sessionId={sessionId} />
        </Box>
      </Box>
      {!split && canSplit && (
        <Box sx={{ position: "absolute", top: 4, right: 34, zIndex: 5 }}>
          <Tooltip title="Open side-by-side companion column">
            <IconButton
              size="small"
              aria-label="Open companion column"
              data-desktop-companion-toggle
              onClick={toggle}
              sx={{ p: 0.35, color: "text.secondary" }}
            >
              <ViewSidebarOutlined sx={{ fontSize: "0.95rem" }} />
            </IconButton>
          </Tooltip>
        </Box>
      )}
    </Box>
  );
}
