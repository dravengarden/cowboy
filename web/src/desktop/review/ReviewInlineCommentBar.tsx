import { Close, DeleteOutline, EditOutlined, RateReviewOutlined, SendOutlined, UndoOutlined } from "@mui/icons-material";
import { alpha, Box, Button, Chip, IconButton, InputBase, Popover, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { ENTER_LABEL, hasSendMod, MOD_LABEL } from "../../platform";
import type { CodeDiffScope } from "../../review/codeApi";
import { desktopSize } from "../../surface/desktopSize";
import { DesktopKeycap, LeaderKeycap } from "../commands/DesktopKeycap";
import { desktopCompositionActive, desktopKeyIntent } from "../commands/keyIntent";
import { commentTargetIdentity } from "./reviewCommentTarget";
import {
  addReviewComment,
  clearReviewComments,
  formatCommentLocation,
  removeReviewComment,
  type ReviewCommentSide,
  type StagedReviewComment,
  submitReviewCommentsToPrompt,
  updateReviewComment,
  useReviewComments,
  useReviewCommentsUnsaved,
} from "./reviewCommentsStore";

export interface ReviewCommentTarget {
  /** Agent session that owns this note; bound when opened so switching
   *  sessions can never stage a note into another session's batch. */
  readonly ownerSessionId: string;
  /** Review target worktree session ID active when opened. */
  readonly reviewSessionId: string;
  readonly path: string;
  readonly startLine: number;
  readonly endLine: number;
  readonly codeSnippet: string;
  readonly kind: "diff" | "source" | "commit" | "pr";
  readonly root?: string | undefined;
  readonly comparison?: string | undefined;
  readonly scope?: CodeDiffScope | undefined;
  readonly side?: ReviewCommentSide | undefined;
  readonly commitOid?: string | undefined;
  readonly pullLabel?: string | undefined;
  readonly revision?: string | undefined;
}

/** Shared IME- and platform-aware key handler for Review note inputs. */
export function reviewNoteKeyAction(
  event: Pick<
    KeyboardEvent,
    "key" | "code" | "keyCode" | "isComposing" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "target"
  >,
  opts?: { composing?: boolean },
): "save" | "cancel" | "ime-escape" | "ignore" {
  const intent = desktopKeyIntent(event, opts);
  if (intent.owner === "ime") {
    return event.key === "Escape" ? "ime-escape" : "ignore";
  }
  if (intent.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey) return "cancel";
  if (intent.key === "Enter" && hasSendMod(event)) return "save";
  return "ignore";
}

export function ReviewInlineCommentBar({
  target,
  initialBody = "",
  onDraftChange,
  onSaved,
  onCancel,
}: {
  target: ReviewCommentTarget;
  initialBody?: string;
  onDraftChange?: (body: string) => void;
  onSaved: () => void;
  onCancel: () => void;
}): React.JSX.Element {
  const [body, setBody] = useState(initialBody);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const targetKey = commentTargetIdentity(target);

  useEffect(() => {
    setBody(initialBody);
    requestAnimationFrame(() => inputRef.current?.focus());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey]);

  const location = formatCommentLocation(target);
  const updateBody = (next: string): void => {
    setBody(next);
    onDraftChange?.(next);
  };
  const save = (overrideText?: string): void => {
    const text = (overrideText ?? body).trim();
    if (!text) return;
    const created = addReviewComment(target.ownerSessionId, {
      path: target.path,
      startLine: target.startLine,
      endLine: target.endLine,
      codeSnippet: target.codeSnippet,
      kind: target.kind,
      body: text,
      root: target.root,
      comparison: target.comparison,
      scope: target.scope,
      side: target.side,
      commitOid: target.commitOid,
      pullLabel: target.pullLabel,
      revision: target.revision,
    });
    if (!created) return;
    updateBody("");
    onSaved();
  };

  return (
    <Box
      data-review-comment-composer
      data-review-comment-owner={target.ownerSessionId}
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
          {location}
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
        onChange={(e): void => updateBody(e.target.value)}
        placeholder="Add a review note (stays in Review; batch-send to Agent when done)…"
        inputProps={{ "aria-label": "Review note", "data-desktop-vim": "off", "data-review-comment-input": "true" }}
        onKeyDown={(e): void => {
          const action = reviewNoteKeyAction(e.nativeEvent);
          if (action === "ime-escape") {
            // Let the IME cancel its candidate popup without letting ancestors dismiss the composer.
            e.stopPropagation();
            return;
          }
          if (action === "cancel") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (action === "save") {
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
          <DesktopKeycap keyLabel={`${MOD_LABEL}${ENTER_LABEL}`} quiet /> save · <DesktopKeycap keyLabel="Esc" quiet /> cancel
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

function ReviewNotePopoverRow({
  sessionId,
  item,
}: {
  sessionId: string;
  item: StagedReviewComment;
}): React.JSX.Element {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(item.body);
  const canSave = draft.trim().length > 0;
  const saveEdit = (): void => {
    if (!canSave) return;
    if (updateReviewComment(sessionId, item.id, draft)) setEditing(false);
  };
  return (
    <Box
      data-review-tray-note={item.id}
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
          {formatCommentLocation(item)}
        </Typography>
        {editing ? (
          <Stack spacing={0.5} sx={{ mt: 0.4 }}>
            <InputBase
              autoFocus
              multiline
              minRows={2}
              maxRows={4}
              fullWidth
              value={draft}
              onChange={(e): void => setDraft(e.target.value)}
              inputProps={{ "aria-label": "Edit review note", "data-desktop-vim": "off" }}
              onKeyDown={(e): void => {
                const action = reviewNoteKeyAction(e.nativeEvent);
                if (action === "ime-escape") {
                  // Never let MUI Popover dismiss the edit tray while the IME cancels a candidate.
                  e.stopPropagation();
                  return;
                }
                if (action === "cancel") {
                  e.preventDefault();
                  e.stopPropagation();
                  setDraft(item.body);
                  setEditing(false);
                } else if (action === "save") {
                  e.preventDefault();
                  e.stopPropagation();
                  saveEdit();
                }
              }}
              sx={{ px: 0.75, py: 0.4, borderRadius: "5px", fontSize: "0.76rem", bgcolor: "background.default", border: 1, borderColor: "primary.main" }}
            />
            <Stack direction="row" justifyContent="flex-end" spacing={0.5}>
              <Button size="small" color="inherit" onClick={(): void => { setDraft(item.body); setEditing(false); }} sx={{ textTransform: "none", fontSize: "0.68rem", minWidth: 0, px: 0.75 }}>
                Cancel
              </Button>
              <Button size="small" variant="contained" disableElevation disabled={!canSave} onClick={saveEdit} sx={{ textTransform: "none", fontSize: "0.68rem", minWidth: 0, px: 0.75, borderRadius: "5px" }}>
                Save
              </Button>
            </Stack>
          </Stack>
        ) : (
          <Typography variant="caption" sx={{ display: "block", whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
            {item.body}
          </Typography>
        )}
      </Box>
      {!editing && (
        <IconButton
          size="small"
          aria-label="Edit note"
          data-review-note-edit={item.id}
          onClick={(): void => { setDraft(item.body); setEditing(true); }}
          sx={{ p: 0.25 }}
        >
          <EditOutlined sx={{ fontSize: "0.85rem" }} />
        </IconButton>
      )}
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
}

export function ReviewNotesTrayButton({
  sessionId,
  onFocusPrompt,
}: {
  sessionId: string | undefined;
  onFocusPrompt: () => "focused" | "deferred" | void;
}): React.JSX.Element | null {
  const comments = useReviewComments(sessionId);
  const unsaved = useReviewCommentsUnsaved(sessionId);
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
        onClose={(_event, reason): void => {
          if (reason === "escapeKeyDown" && desktopCompositionActive()) return;
          setAnchor(null);
        }}
        anchorOrigin={{ vertical: "top", horizontal: "right" }}
        transformOrigin={{ vertical: "bottom", horizontal: "right" }}
        slotProps={{
          paper: {
            onKeyDownCapture: (e: React.KeyboardEvent): void => {
              if (e.key === "Escape" && desktopKeyIntent(e.nativeEvent).owner === "ime") {
                e.stopPropagation();
              }
            },
            sx: { width: 356, maxWidth: "90vw", p: 1.25, borderRadius: "10px" },
          },
        }}
      >
        <Stack direction="row" alignItems="center" spacing={0.5} sx={{ mb: 0.75 }}>
          <Typography variant="subtitle2" sx={{ fontWeight: 700, flex: 1 }}>
            Staged review notes ({String(comments.length)})
          </Typography>
          {unsaved && (
            <Chip size="small" label="Memory only" color="warning" variant="outlined" sx={{ height: 20, fontSize: "0.66rem" }} />
          )}
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
        <Stack spacing={0.6} sx={{ maxHeight: 280, overflowY: "auto", mb: 1 }}>
          {comments.map((item) => (
            <ReviewNotePopoverRow key={item.id} sessionId={sessionId} item={item} />
          ))}
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
