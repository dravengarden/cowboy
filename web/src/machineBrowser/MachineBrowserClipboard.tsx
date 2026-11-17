/** Bringing what the remote browser copied to this device.
 *
 * A copy in the remote browser raises a small prompt at the bottom of the
 * frame; one tap copies it here (iOS and Safari allow a clipboard write only
 * inside a tap, so nothing is copied by itself). When this device refuses
 * the write, the clip opens in the Remote clipboard sheet, selected in a
 * read-only field for the system's own Copy. The sheet also lists the
 * panel's recent copies (`remoteClips.ts`).
 * See docs/machine-browser.md, "Remote clipboard". */
import { CheckRounded, CloseRounded, ContentCopyRounded } from "@mui/icons-material";
import {
  Box,
  Button,
  IconButton,
  List,
  ListItemButton,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { copyText } from "../clipboard";
import { ConfirmSheet } from "../Sheet";
import { relativeAge } from "../syncStatus";
import type { BrowserEvent, BrowserModel } from "./browserModel";
import { NO_CLIPS, type RemoteClip, clipLength, clipPreview, offeredClip } from "./remoteClips";

/** How long the prompt stays up; the clip stays in the sheet after. */
export const CLIP_PROMPT_MS = 10_000;
const COPIED_MS = 1_500;
/** Longer than the sheet's entering transition. */
const SHEET_SETTLE_MS = 400;

function characters(text: string): string {
  const count = clipLength(text);
  return `${String(count)} character${count === 1 ? "" : "s"}`;
}

function CopyPrompt({
  clip,
  copied,
  onCopy,
  onDismiss,
}: {
  clip: RemoteClip;
  copied: boolean;
  onCopy: () => void;
  onDismiss: () => void;
}): React.JSX.Element {
  return (
    <Paper
      elevation={8}
      role="status"
      data-machine-browser-clip-prompt
      sx={{
        position: "absolute",
        left: 8,
        right: 8,
        bottom: 12,
        zIndex: 2,
        mx: "auto",
        maxWidth: 480,
        display: "flex",
        alignItems: "center",
        gap: 1,
        pl: 1.5,
        pr: 0.5,
        py: 0.5,
        borderRadius: 2,
      }}
    >
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Typography variant="caption" color="text.secondary" component="p">
          {copied ? "Copied to this device" : "Copied in the browser"}
        </Typography>
        <Typography variant="body2" noWrap>
          {clipPreview(clip.text)}
        </Typography>
      </Box>
      <Button
        size="small"
        variant="contained"
        disabled={copied}
        startIcon={copied ? <CheckRounded /> : <ContentCopyRounded />}
        onClick={onCopy}
        sx={{ flexShrink: 0 }}
      >
        {copied ? "Copied" : "Copy"}
      </Button>
      <IconButton size="small" aria-label="Dismiss" onClick={onDismiss}>
        <CloseRounded fontSize="small" />
      </IconButton>
    </Paper>
  );
}

/** The full text, read-only and selectable: the fallback when this device
 * refuses a clipboard write. Focusing it selects everything. */
function ClipText({ text, autoSelect }: { text: string; autoSelect: boolean }): React.JSX.Element {
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const selectAll = (): void => {
    const field = ref.current;
    if (!field) return;
    field.focus({ preventScroll: true });
    field.setSelectionRange(0, field.value.length);
  };
  // Again once the sheet has finished opening: its focus handling would
  // otherwise take the selection back.
  useEffect(() => {
    if (!autoSelect) return undefined;
    selectAll();
    const timer = setTimeout(selectAll, SHEET_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [autoSelect]);
  return (
    <Stack spacing={0.5}>
      <TextField
        multiline
        minRows={2}
        maxRows={10}
        fullWidth
        value={text}
        inputRef={ref}
        onFocus={selectAll}
        slotProps={{
          htmlInput: {
            readOnly: true,
            "aria-label": "Copied text",
            "data-machine-browser-clip-text": true,
            style: { fontSize: 16 },
          },
        }}
      />
      <Box>
        <Button size="small" onClick={selectAll}>
          Select all
        </Button>
      </Box>
    </Stack>
  );
}

function ClipRow({
  clip,
  now,
  expanded,
  selectText,
  copied,
  onToggle,
  onCopy,
}: {
  clip: RemoteClip;
  now: number;
  expanded: boolean;
  selectText: boolean;
  copied: boolean;
  onToggle: () => void;
  onCopy: () => void;
}): React.JSX.Element {
  return (
    <Box component="li" data-machine-browser-clip={clip.id} sx={{ listStyle: "none" }}>
      <Stack direction="row" alignItems="center" spacing={1}>
        <ListItemButton onClick={onToggle} sx={{ borderRadius: 1, minWidth: 0, py: 0.75 }}>
          <Box sx={{ minWidth: 0 }}>
            <Typography
              variant="body2"
              sx={{
                display: "-webkit-box",
                WebkitLineClamp: 2,
                WebkitBoxOrient: "vertical",
                overflow: "hidden",
                wordBreak: "break-word",
              }}
            >
              {clipPreview(clip.text, 160)}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {characters(clip.text)} · {relativeAge(clip.at, now)}
            </Typography>
          </Box>
        </ListItemButton>
        <Button
          size="small"
          startIcon={copied ? <CheckRounded /> : <ContentCopyRounded />}
          onClick={onCopy}
          sx={{ flexShrink: 0 }}
        >
          {copied ? "Copied" : "Copy"}
        </Button>
      </Stack>
      {expanded && (
        <Box sx={{ px: 2, pb: 1.5 }}>
          <ClipText text={clip.text} autoSelect={selectText} />
        </Box>
      )}
    </Box>
  );
}

/** A shown clip: which one is expanded, and whether its text was selected
 * for the system's Copy because this device refused the write. */
interface Shown {
  id: number | undefined;
  refused: boolean;
}

/** The prompt, the menu entry's action and the sheet for one panel. */
export function useRemoteClipboard(
  model: BrowserModel,
  dispatch: (event: BrowserEvent) => void,
): { prompt: ReactNode; openSheet: () => void; dialog: ReactNode } {
  const clips = model.clips ?? NO_CLIPS;
  const offered = offeredClip(clips);
  const [sheet, setSheet] = useState<Shown | undefined>(undefined);
  const [copied, setCopied] = useState<number | undefined>(undefined);
  const offerId = offered?.id;
  useEffect(() => {
    if (offerId === undefined) return undefined;
    const timer = setTimeout(() => dispatch({ type: "clip_settled", id: offerId }), CLIP_PROMPT_MS);
    return () => clearTimeout(timer);
  }, [offerId, dispatch]);
  useEffect(() => {
    if (copied === undefined) return undefined;
    const timer = setTimeout(() => {
      setCopied(undefined);
      dispatch({ type: "clip_settled", id: copied });
    }, COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied, dispatch]);
  // Runs inside the tap: the write needs its user activation.
  const copy = (clip: RemoteClip): void => {
    void copyText(clip.text).then((ok) => {
      if (ok) {
        setCopied(clip.id);
        return;
      }
      dispatch({ type: "clip_settled", id: clip.id });
      setSheet({ id: clip.id, refused: true });
    });
  };
  const prompt =
    offered && sheet === undefined ? (
      <CopyPrompt
        clip={offered}
        copied={copied === offered.id}
        onCopy={() => copy(offered)}
        onDismiss={() => dispatch({ type: "clip_settled", id: offered.id })}
      />
    ) : null;
  const close = (): void => setSheet(undefined);
  const now = Date.now();
  const dialog = sheet ? (
    <ConfirmSheet
      open
      onClose={close}
      title="Remote clipboard"
      actions={
        <Stack direction="row" spacing={1} sx={{ width: "100%" }}>
          <Button
            color="inherit"
            disabled={clips.items.length === 0}
            onClick={() => {
              dispatch({ type: "clips_clear" });
              close();
            }}
          >
            Clear all
          </Button>
          <Box sx={{ flex: 1 }} />
          <Button variant="contained" onClick={close}>
            Done
          </Button>
        </Stack>
      }
    >
      <Typography variant="body2" color="text.secondary">
        {sheet.refused
          ? "This device did not allow copying. Select the text below and use Copy."
          : "Text copied in the remote browser while this panel is open. It stays on this device only and is cleared when you close the browser."}
      </Typography>
      {clips.items.length === 0 ? (
        <Typography variant="body2" sx={{ py: 2 }}>
          Nothing copied yet. Copy text in the remote browser and it shows up here.
        </Typography>
      ) : (
        <List disablePadding>
          {clips.items.map((clip) => (
            <ClipRow
              key={clip.id}
              clip={clip}
              now={now}
              expanded={sheet.id === clip.id}
              selectText={sheet.refused && sheet.id === clip.id}
              copied={copied === clip.id}
              onToggle={() =>
                setSheet({
                  id: sheet.id === clip.id ? undefined : clip.id,
                  refused: false,
                })
              }
              onCopy={() => copy(clip)}
            />
          ))}
        </List>
      )}
    </ConfirmSheet>
  ) : null;
  return {
    prompt,
    openSheet: () => setSheet({ id: clips.items[0]?.id, refused: false }),
    dialog,
  };
}
