import { Close, DeleteOutline, RateReviewOutlined, SendOutlined, UndoOutlined } from "@mui/icons-material";
import { alpha, Box, Button, Chip, IconButton, InputBase, Popover, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { DesktopKeycap, LeaderKeycap } from "../commands/DesktopKeycap";
import {
  addReviewComment,
  clearReviewComments,
  removeReviewComment,
  submitReviewCommentsToPrompt,
  useReviewComments,
} from "./reviewCommentsStore";

export interface ReviewCommentTarget {
  readonly path: string;
  readonly startLine: number;
  readonly endLine: number;
  readonly codeSnippet: string;
  readonly kind: "diff" | "source" | "commit" | "pr";
  readonly root?: string | undefined;
  readonly comparison?: string | undefined;
}

export function ReviewInlineCommentBar({
  sessionId,
  target,
  onSaved,
  onCancel,
}: {
  sessionId: string;
  target: ReviewCommentTarget;
  onSaved: () => void;
  onCancel: () => void;
}): React.JSX.Element {
  const [body, setBody] = useState("");
  const inputRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    setBody("");
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [target.path, target.startLine, target.endLine]);

  const loc = target.startLine === target.endLine
    ? `L${String(target.startLine)}`
    : `L${String(target.startLine)}-${String(target.endLine)}`;
  const save = (overrideText?: string): void => {
    const text = (overrideText ?? body).trim();
    if (!text) return;
    addReviewComment(sessionId, {
      path: target.path,
      startLine: target.startLine,
      endLine: target.endLine,
      codeSnippet: target.codeSnippet,
      kind: target.kind,
      body: text,
      root: target.root,
      comparison: target.comparison,
    });
    setBody("");
    onSaved();
  };

  return (
    <Box
      data-review-comment-composer
      sx={(theme) => ({
        borderTop: 1,
        borderColor: "divider",
        bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.08 : 0.04),
        px: 1.5,
        py: 1,
        flexShrink: 0,
      })}
    >
      <Stack direction="row" alignItems="center" spacing={0.75} sx={{ mb: 0.6 }}>
        <RateReviewOutlined sx={{ fontSize: "0.95rem", color: "primary.main" }} />
        <Typography variant="caption" noWrap sx={{ fontWeight: 650, fontFamily: "monospace", flex: 1 }}>
          {target.path}:{loc}
        </Typography>
        <Chip
          size="small"
          clickable
          icon={<UndoOutlined sx={{ fontSize: "0.85rem !important" }} />}
          label="Ask to revert"
          data-review-comment-revert-chip
          onClick={(): void => save("Please revert this change.")}
          sx={{ height: desktopSize(22), fontSize: "0.7rem" }}
        />
        <IconButton size="small" aria-label="Cancel review note" onClick={onCancel} sx={{ p: 0.25 }}>
          <Close sx={{ fontSize: "0.9rem" }} />
        </IconButton>
      </Stack>
      <InputBase
        inputRef={inputRef}
        multiline
        minRows={2}
        maxRows={5}
        fullWidth
        value={body}
        onChange={(e): void => setBody(e.target.value)}
        placeholder="Add a review note (stays in Review; batch-send to Agent when done)…"
        inputProps={{ "aria-label": "Review note", "data-desktop-vim": "off", "data-review-comment-input": "true" }}
        onKeyDown={(e): void => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            e.stopPropagation();
            save();
          }
        }}
        sx={(theme) => ({
          px: 1,
          py: 0.6,
          borderRadius: "6px",
          fontSize: "0.8rem",
          bgcolor: theme.palette.background.default,
          border: 1,
          borderColor: "divider",
          "&.Mui-focused": { borderColor: "primary.main" },
        })}
      />
      <Stack direction="row" alignItems="center" justifyContent="flex-end" spacing={0.75} sx={{ mt: 0.6 }}>
        <Typography variant="caption" color="text.secondary" sx={{ mr: "auto", fontSize: "0.68rem" }}>
          <DesktopKeycap keyLabel="⌘⏎" quiet /> save · <DesktopKeycap keyLabel="Esc" quiet /> cancel
        </Typography>
        <Button size="small" color="inherit" onClick={onCancel} sx={{ textTransform: "none", fontSize: "0.72rem" }}>
          Cancel
        </Button>
        <Button
          size="small"
          variant="contained"
          disableElevation
          disabled={body.trim().length === 0}
          data-review-comment-save
          onClick={(): void => save()}
          sx={{ textTransform: "none", fontSize: "0.72rem", borderRadius: "6px" }}
        >
          Stage note
        </Button>
      </Stack>
    </Box>
  );
}

export function ReviewNotesTrayButton({
  sessionId,
  onFocusPrompt,
}: {
  sessionId: string | undefined;
  onFocusPrompt: () => void;
}): React.JSX.Element | null {
  const comments = useReviewComments(sessionId);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  if (!sessionId || comments.length === 0) return null;

  const submitAll = (): void => {
    setAnchor(null);
    submitReviewCommentsToPrompt(sessionId, onFocusPrompt);
  };

  return (
    <>
      <Button
        size="small"
        variant="contained"
        disableElevation
        data-review-notes-tray
        startIcon={<RateReviewOutlined sx={{ fontSize: "0.9rem !important" }} />}
        onClick={(e): void => setAnchor(e.currentTarget)}
        sx={{
          textTransform: "none",
          fontSize: "0.72rem",
          flexShrink: 0,
          gap: 0.5,
          borderRadius: "6px",
          minHeight: desktopSize(24),
          py: 0,
          px: 1,
        }}
      >
        {String(comments.length)} {comments.length === 1 ? "note" : "notes"}
        <LeaderKeycap leaderKey="VB" />
      </Button>
      <Popover
        open={anchor !== null}
        anchorEl={anchor}
        onClose={(): void => setAnchor(null)}
        anchorOrigin={{ vertical: "top", horizontal: "right" }}
        transformOrigin={{ vertical: "bottom", horizontal: "right" }}
        slotProps={{ paper: { sx: { width: 340, maxWidth: "90vw", p: 1.25, borderRadius: "10px" } } }}
      >
        <Stack direction="row" alignItems="center" sx={{ mb: 0.75 }}>
          <Typography variant="subtitle2" sx={{ fontWeight: 700, flex: 1 }}>
            Staged review notes ({String(comments.length)})
          </Typography>
          <Button
            size="small"
            color="inherit"
            onClick={(): void => {
              clearReviewComments(sessionId);
              setAnchor(null);
            }}
            sx={{ textTransform: "none", fontSize: "0.7rem", color: "text.secondary" }}
          >
            Clear
          </Button>
        </Stack>
        <Stack spacing={0.6} sx={{ maxHeight: 260, overflowY: "auto", mb: 1 }}>
          {comments.map((item) => {
            const loc = item.startLine === item.endLine
              ? `L${String(item.startLine)}`
              : `L${String(item.startLine)}-${String(item.endLine)}`;
            return (
              <Box
                key={item.id}
                sx={(theme) => ({
                  p: 0.75,
                  borderRadius: "6px",
                  bgcolor: alpha(theme.palette.action.hover, 0.7),
                  display: "flex",
                  alignItems: "flex-start",
                  gap: 0.5,
                })}
              >
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="caption" noWrap sx={{ display: "block", fontFamily: "monospace", color: "primary.main", fontWeight: 650 }}>
                    {item.path}:{loc}
                  </Typography>
                  <Typography variant="caption" sx={{ display: "block", whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
                    {item.body}
                  </Typography>
                </Box>
                <IconButton
                  size="small"
                  aria-label="Delete note"
                  onClick={(): void => removeReviewComment(sessionId, item.id)}
                  sx={{ p: 0.25 }}
                >
                  <DeleteOutline sx={{ fontSize: "0.9rem" }} />
                </IconButton>
              </Box>
            );
          })}
        </Stack>
        <Button
          fullWidth
          size="small"
          variant="contained"
          disableElevation
          data-review-notes-submit-all
          startIcon={<SendOutlined sx={{ fontSize: "0.9rem !important" }} />}
          onClick={submitAll}
          sx={{ textTransform: "none", fontWeight: 650, borderRadius: "6px" }}
        >
          Send all {String(comments.length)} notes to Prompt
        </Button>
      </Popover>
    </>
  );
}
