import { useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import { Add, Close, ExpandMore, Send, Stop } from "@mui/icons-material";
import { send, useStore } from "./store";
import type { ConfigOption, ContentBlock, Status } from "./protocol";

// Cmd/Ctrl + Enter = send. Plain Enter = newline.
//
// Why this way (not the reverse): pasting multi-line code / prompts is a
// daily action; making plain Enter send would shred any pasted snippet that
// contains a newline. ChatGPT/Claude.ai/Cursor all default to "Enter =
// newline + Cmd-Enter sends" for the same reason. Touch keyboards inherit
// the same model — their Enter key inserts a newline and the user taps the
// send button.
//
// Layout: mobile-first. Composer sits at the bottom of the viewport with a
// safe-area inset. Action row (attach + agent-advertised config chips for
// mode / model / effort) sits BELOW the textarea so the textarea always
// stays wide and tap-able. The row is horizontally scrollable on narrow
// viewports so any number of dropdowns doesn't force a wrap.
export function Composer({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<ContentBlock[]>([]);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const { configOptions } = useStore();

  const busy = status === "busy";
  const dead = status === "exited" || status === "crashed";
  const sendable = (!!text.trim() || attachments.length > 0) && !dead;

  // Pull the agent-advertised options for this session, if known. Sorted in
  // a fixed display order so dropdowns don't flicker between
  // config_option_update notifications.
  const options = useMemo(() => {
    const raw = configOptions.get(sessionId) ?? [];
    const order = ["mode", "model", "effort"];
    return [...raw].sort((a, b) => {
      const ai = order.indexOf(a.id);
      const bi = order.indexOf(b.id);
      if (ai === -1 && bi === -1) return a.id.localeCompare(b.id);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
  }, [configOptions, sessionId]);

  function submit(): void {
    if (!sendable) return;
    const blocks: ContentBlock[] = [];
    const trimmed = text.trimEnd();
    if (trimmed) blocks.push({ type: "text", text: trimmed });
    blocks.push(...attachments);
    if (blocks.length === 0) return;
    if (blocks.length === 1 && attachments.length === 0) {
      // Text-only path uses the legacy `text` field — keeps wire-compat
      // with the existing daemon path that wraps it in a single ACP Text
      // block. No functional difference; just smaller frames.
      send({ type: "prompt", session_id: sessionId, text: trimmed });
    } else {
      send({ type: "prompt", session_id: sessionId, content: blocks });
    }
    setText("");
    setAttachments([]);
  }

  async function handleFiles(files: FileList | null): Promise<void> {
    if (!files) return;
    const next: ContentBlock[] = [];
    for (const f of Array.from(files)) {
      if (!f.type.startsWith("image/")) continue;
      const data = await readAsBase64(f);
      next.push({ type: "image", mimeType: f.type, data });
    }
    if (next.length > 0) setAttachments((prev) => [...prev, ...next]);
  }

  return (
    <Box
      sx={{
        p: { xs: 1, sm: 1.5 },
        pb: { xs: "calc(env(safe-area-inset-bottom) + 8px)", sm: 1.5 },
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
      }}
    >
      {attachments.length > 0 && (
        <AttachmentPreview
          blocks={attachments}
          onRemove={(i): void =>
            setAttachments((prev) => prev.filter((_, idx) => idx !== i))
          }
        />
      )}
      <Stack direction="row" spacing={1} alignItems="flex-end">
        <TextField
          fullWidth
          multiline
          minRows={1}
          maxRows={12}
          size="small"
          placeholder={dead ? "Session ended" : "Message the agent…"}
          value={text}
          disabled={dead}
          onChange={(e): void => setText(e.target.value)}
          onKeyDown={(e): void => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              submit();
            }
          }}
          slotProps={{
            input: {
              sx: {
                fontFamily:
                  "system-ui, -apple-system, Segoe UI, Roboto, sans-serif",
                fontSize: 14,
              },
            },
          }}
        />
        {busy ? (
          <IconButton
            color="error"
            aria-label="cancel"
            sx={{ width: 44, height: 44 }}
            onClick={(): void => send({ type: "cancel", session_id: sessionId })}
          >
            <Stop />
          </IconButton>
        ) : (
          <IconButton
            color="primary"
            aria-label="send"
            disabled={!sendable}
            sx={{ width: 44, height: 44 }}
            onClick={submit}
          >
            <Send />
          </IconButton>
        )}
      </Stack>
      <Stack
        direction="row"
        spacing={0.75}
        alignItems="center"
        sx={{
          mt: 1,
          overflowX: "auto",
          // Hide scrollbar on touch viewports — the chips themselves are the
          // affordance. Keep it on desktop where pointer drag matters.
          "&::-webkit-scrollbar": {
            display: { xs: "none", sm: "block" },
            height: 6,
          },
          msOverflowStyle: "none",
          scrollbarWidth: "thin",
          minHeight: 40,
        }}
      >
        <input
          ref={fileInput}
          type="file"
          accept="image/*"
          multiple
          style={{ display: "none" }}
          onChange={(e): void => {
            void handleFiles(e.target.files);
            e.target.value = "";
          }}
        />
        <Tooltip title="Attach images">
          <span>
            <IconButton
              aria-label="attach"
              disabled={dead}
              sx={{ width: 40, height: 40, flexShrink: 0 }}
              onClick={(): void => fileInput.current?.click()}
            >
              <Add />
            </IconButton>
          </span>
        </Tooltip>
        {options.map((opt) => (
          <ConfigOptionChip
            key={opt.id}
            option={opt}
            disabled={dead}
            onSelect={(value): void =>
              send({
                type: "set_config_option",
                session_id: sessionId,
                config_id: opt.id,
                value,
              })
            }
          />
        ))}
        <Box sx={{ flex: 1 }} />
        <Typography
          variant="caption"
          color="text.disabled"
          sx={{
            display: { xs: "none", sm: "block" },
            ml: 1,
            whiteSpace: "nowrap",
            fontSize: 11,
            flexShrink: 0,
          }}
        >
          ⌘/Ctrl + Enter = send
        </Typography>
      </Stack>
    </Box>
  );
}

function ConfigOptionChip({
  option,
  disabled,
  onSelect,
}: {
  option: ConfigOption;
  disabled: boolean;
  onSelect: (value: string | boolean) => void;
}): React.JSX.Element {
  const [anchor, setAnchor] = useState<null | HTMLElement>(null);
  const current = useMemo(
    () =>
      option.options.find((o) => o.value === option.currentValue) ??
      option.options[0],
    [option.options, option.currentValue],
  );
  return (
    <>
      <Button
        size="small"
        variant="outlined"
        color="inherit"
        disabled={disabled}
        endIcon={<ExpandMore fontSize="small" />}
        onClick={(e): void => setAnchor(e.currentTarget)}
        sx={{
          textTransform: "none",
          minHeight: 36,
          px: 1.25,
          flexShrink: 0,
          fontWeight: 500,
          borderColor: "divider",
        }}
      >
        {current?.name ?? String(option.currentValue)}
      </Button>
      <Menu
        anchorEl={anchor}
        open={!!anchor}
        onClose={(): void => setAnchor(null)}
        slotProps={{ paper: { sx: { maxWidth: 360 } } }}
      >
        {option.options.map((o) => (
          <MenuItem
            key={String(o.value)}
            selected={o.value === option.currentValue}
            onClick={(): void => {
              onSelect(o.value);
              setAnchor(null);
            }}
            sx={{ alignItems: "flex-start", whiteSpace: "normal" }}
          >
            <Box>
              <Typography variant="body2" sx={{ fontWeight: 500 }}>
                {o.name}
              </Typography>
              {o.description && (
                <Typography variant="caption" color="text.secondary">
                  {o.description}
                </Typography>
              )}
            </Box>
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

function AttachmentPreview({
  blocks,
  onRemove,
}: {
  blocks: ContentBlock[];
  onRemove: (i: number) => void;
}): React.JSX.Element {
  return (
    <Stack direction="row" spacing={1} sx={{ mb: 1, overflowX: "auto" }}>
      {blocks.map((b, i) => {
        if (b.type !== "image" || typeof b.data !== "string") return null;
        const mt = typeof b.mimeType === "string" ? b.mimeType : "image/png";
        return (
          <Box
            key={i}
            sx={{
              position: "relative",
              width: 64,
              height: 64,
              borderRadius: 1,
              border: 1,
              borderColor: "divider",
              overflow: "hidden",
              flexShrink: 0,
            }}
          >
            <img
              src={`data:${mt};base64,${b.data}`}
              alt="attachment"
              style={{ width: "100%", height: "100%", objectFit: "cover" }}
            />
            <IconButton
              size="small"
              onClick={(): void => onRemove(i)}
              sx={{
                position: "absolute",
                top: 2,
                right: 2,
                width: 22,
                height: 22,
                bgcolor: "rgba(0,0,0,0.55)",
                color: "#fff",
                "&:hover": { bgcolor: "rgba(0,0,0,0.75)" },
              }}
            >
              <Close sx={{ fontSize: 14 }} />
            </IconButton>
          </Box>
        );
      })}
    </Stack>
  );
}

async function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = (): void => reject(reader.error);
    reader.onload = (): void => {
      const result = reader.result;
      if (typeof result !== "string") {
        reject(new Error("expected data URL"));
        return;
      }
      // result is "data:<mime>;base64,<XXX>"; strip the prefix.
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}
