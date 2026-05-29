import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  ClickAwayListener,
  IconButton,
  Menu,
  MenuItem,
  MenuList,
  Paper,
  Popper,
  Skeleton,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import { Add, Close, ExpandMore, Send, Stop } from "@mui/icons-material";
import { send, useStore } from "./store";
import type {
  AcpUpdate,
  AvailableCommand,
  ConfigOption,
  ContentBlock,
  Envelope,
  Status,
} from "./protocol";

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
  const textFieldRef = useRef<HTMLDivElement | null>(null);
  const { configOptions, timelines } = useStore();

  const busy = status === "busy";
  const starting = status === "starting";
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
  // `starting` is the obvious case; we also keep the skeleton on for the
  // brief window after status flips to `running` but before the agent's
  // first `config_option_update` arrives (otherwise the action row pops
  // empty for ~1 frame and then re-flows when the chips appear).
  const showSkeleton = !dead && options.length === 0 && (starting || status === "running");

  // Slash-command picker: claude-agent-acp streams its `/help`, `/clear` etc
  // via `available_commands_update` SessionUpdate. Read the latest one from
  // the timeline rather than mirror it in store state — the list is small
  // and only this component renders it.
  const availableCommands = useMemo(
    () => latestAvailableCommands(timelines.get(sessionId) ?? []),
    [timelines, sessionId],
  );
  const slashQuery = useMemo(() => parseSlashQuery(text), [text]);
  const slashOpen = slashQuery !== null && availableCommands.length > 0 && !dead;
  const filteredCommands = useMemo(() => {
    if (!slashOpen || slashQuery === null) return [];
    const q = slashQuery.toLowerCase();
    return availableCommands.filter((c) => c.name.toLowerCase().includes(q));
  }, [slashOpen, slashQuery, availableCommands]);
  const [slashIndex, setSlashIndex] = useState(0);
  // Clamp the selected index back to range whenever the filter shrinks.
  useEffect(() => {
    setSlashIndex((i) => Math.max(0, Math.min(i, filteredCommands.length - 1)));
  }, [filteredCommands.length]);
  const insertCommand = useCallback(
    (name: string): void => {
      setText(`/${name} `);
      setSlashIndex(0);
      // Defer focus to the next tick so React's controlled-value update has
      // applied before we put the caret at the end.
      queueMicrotask(() => {
        const input = textFieldRef.current?.querySelector("textarea");
        if (input) {
          input.focus();
          const end = input.value.length;
          input.setSelectionRange(end, end);
        }
      });
    },
    [],
  );

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
        position: "relative", // anchor for Popper portal placement
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
          inputRef={(): void => {
            // ref to the underlying textarea is used for caret placement; the
            // wrapping div ref (textFieldRef) is what Popper anchors to.
          }}
          ref={textFieldRef}
          placeholder={dead ? "Session ended" : "Message the agent…"}
          value={text}
          disabled={dead}
          onChange={(e): void => setText(e.target.value)}
          onKeyDown={(e): void => {
            // Slash picker keyboard control wins over send/newline so the
            // user can pick a command without surprises.
            if (slashOpen && filteredCommands.length > 0) {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSlashIndex((i) => (i + 1) % filteredCommands.length);
                return;
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setSlashIndex(
                  (i) => (i - 1 + filteredCommands.length) % filteredCommands.length,
                );
                return;
              }
              if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.ctrlKey)) {
                const cmd = filteredCommands[slashIndex];
                if (cmd) {
                  e.preventDefault();
                  insertCommand(cmd.name);
                  return;
                }
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setText("");
                return;
              }
            }
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
          // Hide scrollbar on touch viewports (anything below the desktop
          // tier; same threshold as the sidebar drawer). On desktop the
          // bar is a useful drag affordance when chips overflow.
          "&::-webkit-scrollbar": {
            display: { xs: "none", lg: "block" },
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
        {showSkeleton ? (
          <ConfigChipSkeletons />
        ) : (
          options.map((opt) => (
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
          ))
        )}
        <Box sx={{ flex: 1 }} />
        {/* Keyboard hint is meaningless on touch — hide unless we're on a
            pointer-first viewport (sidebar persistent, lg+). */}
        <Typography
          variant="caption"
          color="text.disabled"
          sx={{
            display: { xs: "none", lg: "block" },
            ml: 1,
            whiteSpace: "nowrap",
            fontSize: 11,
            flexShrink: 0,
          }}
        >
          ⌘/Ctrl + Enter = send
        </Typography>
      </Stack>
      <SlashPicker
        open={slashOpen && filteredCommands.length > 0}
        anchorEl={textFieldRef.current}
        commands={filteredCommands}
        selectedIndex={slashIndex}
        onSelect={insertCommand}
        onClose={(): void => setText("")}
      />
    </Box>
  );
}

function ConfigChipSkeletons(): React.JSX.Element {
  // Three skeletons sized to the typical chip widths (Bypass Permissions ≈
  // 160px, Default (recommended) ≈ 170px, High ≈ 80px). Keeps the row's
  // visual rhythm stable when the real chips replace them.
  const widths = [148, 168, 76];
  return (
    <>
      {widths.map((w, i) => (
        <Skeleton
          key={i}
          variant="rounded"
          width={w}
          height={36}
          animation="wave"
          sx={{ flexShrink: 0, borderRadius: 1 }}
        />
      ))}
    </>
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

function SlashPicker({
  open,
  anchorEl,
  commands,
  selectedIndex,
  onSelect,
  onClose,
}: {
  open: boolean;
  anchorEl: HTMLElement | null;
  commands: AvailableCommand[];
  selectedIndex: number;
  onSelect: (name: string) => void;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Popper
      open={open}
      anchorEl={anchorEl}
      placement="top-start"
      // Cover the textarea width up to a reasonable max so long command
      // descriptions don't overflow the viewport on narrow phones.
      modifiers={[{ name: "offset", options: { offset: [0, 8] } }]}
      sx={{ zIndex: (theme): number => theme.zIndex.modal + 1 }}
    >
      <ClickAwayListener onClickAway={onClose}>
        <Paper
          elevation={6}
          sx={{
            width: anchorEl ? anchorEl.clientWidth : "auto",
            maxWidth: "min(560px, 92vw)",
            maxHeight: 320,
            overflowY: "auto",
            borderRadius: 1.5,
          }}
        >
          <MenuList dense disablePadding>
            {commands.map((c, i) => (
              <MenuItem
                key={c.name}
                selected={i === selectedIndex}
                onClick={(): void => onSelect(c.name)}
                sx={{ alignItems: "flex-start", whiteSpace: "normal", py: 0.75 }}
              >
                <Stack sx={{ width: "100%", minWidth: 0 }}>
                  <Stack direction="row" spacing={1} alignItems="baseline">
                    <Typography
                      variant="body2"
                      sx={{
                        fontWeight: 600,
                        fontFamily:
                          "ui-monospace, SFMono-Regular, Menlo, monospace",
                      }}
                    >
                      /{c.name}
                    </Typography>
                  </Stack>
                  {c.description && (
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ whiteSpace: "normal" }}
                    >
                      {c.description}
                    </Typography>
                  )}
                </Stack>
              </MenuItem>
            ))}
          </MenuList>
        </Paper>
      </ClickAwayListener>
    </Popper>
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

// Find the most recent `available_commands_update` payload in the session's
// event log. Walks in reverse so the cost is at most one event when the
// agent already advertised, and the empty-array baseline is cheap on first
// connect.
function latestAvailableCommands(timeline: Envelope[]): AvailableCommand[] {
  for (let i = timeline.length - 1; i >= 0; i -= 1) {
    const env = timeline[i];
    if (env && env.kind === "update") {
      const u = env.update as AcpUpdate;
      if (u.sessionUpdate === "available_commands_update" && Array.isArray(u.availableCommands)) {
        return u.availableCommands;
      }
    }
  }
  return [];
}

// Parse a leading `/<word>?` from the current text input. Returns the word
// after the slash (possibly empty) when the input is *exactly* one slash-
// prefixed token with no spaces or newlines, otherwise null (= picker
// stays closed). This matches Slack's first-position-only behavior; the
// user gets the picker only when starting a fresh slash command.
function parseSlashQuery(text: string): string | null {
  const m = /^\/(\S*)$/.exec(text);
  return m ? (m[1] ?? "") : null;
}
