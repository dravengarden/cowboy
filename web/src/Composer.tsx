import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  CircularProgress,
  ClickAwayListener,
  Divider,
  Drawer,
  IconButton,
  List,
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
  useMediaQuery,
  useTheme,
} from "@mui/material";
import {
  AlternateEmail,
  ExpandMore,
  Send,
  Stop,
  Tune,
} from "@mui/icons-material";
import { send, useStore } from "./store";
import { originLabel } from "./protocol";
import type {
  AcpUpdate,
  AvailableCommand,
  ConfigOption,
  Envelope,
  SessionMeta,
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
// safe-area inset. Action row (slash-command / @-reference triggers + the
// agent-advertised config chips for mode / model / effort) sits BELOW the
// textarea so the textarea always stays wide and tap-able. The row is
// horizontally scrollable on narrow viewports so any number of dropdowns
// doesn't force a wrap.
export function Composer({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const [text, setText] = useState("");
  const textFieldRef = useRef<HTMLDivElement | null>(null);
  const { configOptions, sessions, timelines } = useStore();
  // The active session's metadata, surfaced read-only inside the options
  // sheet (mobile's "session settings" popup). Desktop shows the same facts
  // in the always-visible sidebar, so the sheet — and this lookup — only
  // matters on the compact tier.
  const session = sessions.find((s) => s.id === sessionId);
  const theme = useTheme();
  // Touch tier collapses the agent config into a single Tune button — tapping
  // it opens a BottomSheet with the session info + every config option in one
  // place. Inspired by ChatGPT / DeepSeek / Gemini: chips wrap awkwardly on
  // iPad portrait (820px) and are completely unreadable on a 390px iPhone, so
  // the sheet pattern wins on every sub-desktop viewport. Desktop keeps the
  // inline chip row — there's room.
  const compact = useMediaQuery(theme.breakpoints.down("lg"));
  const [sheetOpen, setSheetOpen] = useState(false);

  const busy = status === "busy";
  const starting = status === "starting";
  const dead = status === "exited" || status === "crashed";
  const sendable = !!text.trim() && !dead;

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

  // @-file picker. Unlike slash (whole-input only), `@` triggers whenever the
  // text *ends* with an `@token` actively being typed — a trailing space or
  // selecting a file completes the reference and closes the picker. The
  // candidate list comes from the daemon (gitignore-aware fuzzy search of the
  // session cwd); `atDismissed` lets Escape suppress it until the query moves.
  const atQuery = useMemo(() => parseAtQuery(text), [text]);
  const [atDismissed, setAtDismissed] = useState(false);
  useEffect(() => {
    setAtDismissed(false);
  }, [atQuery]);
  const [atFiles, setAtFiles] = useState<string[]>([]);
  const [atIndex, setAtIndex] = useState(0);
  const atOpen = atQuery !== null && !atDismissed && atFiles.length > 0 && !dead;
  // Debounced fetch: a keystroke schedules a search 120ms out, and any newer
  // query (or unmount) aborts the in-flight one so results can't land stale.
  useEffect(() => {
    if (atQuery === null || dead) {
      setAtFiles([]);
      return undefined;
    }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      const url = `/api/sessions/${encodeURIComponent(sessionId)}/files?q=${
        encodeURIComponent(atQuery)
      }&limit=20`;
      fetch(url, { signal: controller.signal })
        .then((r) => (r.ok ? r.json() : { files: [] }))
        .then((d: { files?: string[] }) =>
          setAtFiles(Array.isArray(d.files) ? d.files : []),
        )
        .catch(() => {
          /* aborted or transient network error — keep the last list */
        });
    }, 120);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [atQuery, dead, sessionId]);
  // Clamp the selection when the candidate list shrinks.
  useEffect(() => {
    setAtIndex((i) => Math.max(0, Math.min(i, atFiles.length - 1)));
  }, [atFiles.length]);
  const insertFile = useCallback((path: string): void => {
    setText((prev) => {
      const m = AT_QUERY_RE.exec(prev);
      if (m === null) return prev;
      // The token sits at the very end (the regex is `$`-anchored), so cut from
      // the `@` and append the picked path plus a trailing space — which also
      // ends the `@token`, closing the picker on the next render.
      const at = prev.length - (m[1] ?? "").length - 1;
      const next = `${prev.slice(0, at)}@${path} `;
      queueMicrotask(() => {
        const input = textFieldRef.current?.querySelector("textarea");
        if (input) {
          input.focus();
          const end = input.value.length;
          input.setSelectionRange(end, end);
        }
      });
      return next;
    });
  }, []);

  // The `/` and `@` action buttons just type their trigger character into the
  // field and focus it: `/` from an empty field opens the slash-command picker
  // (same path as typing it); `@` starts a file reference that the agent reads
  // from the prompt text. Append + focus rather than replace so an in-progress
  // message isn't clobbered.
  const appendToken = useCallback((ch: string): void => {
    setText((t) => t + ch);
    queueMicrotask(() => {
      const input = textFieldRef.current?.querySelector("textarea");
      if (input) {
        input.focus();
        const end = input.value.length;
        input.setSelectionRange(end, end);
      }
    });
  }, []);

  function submit(): void {
    if (!sendable) return;
    const trimmed = text.trimEnd();
    if (!trimmed) return;
    send({ type: "prompt", session_id: sessionId, text: trimmed });
    setText("");
  }

  return (
    <Box
      sx={{
        // Pad every edge against the device safe area, not just the bottom.
        // The action row's far-left (slash) and far-right (send) buttons sit
        // right where iPhone's rounded screen corners curve in. In portrait
        // `env(safe-area-inset-left/right)` is 0, so floor the side padding to
        // 12px to keep those tap targets off the corner radius; in landscape the
        // non-zero side insets push them clear of the notch too. Bottom: the
        // home-indicator inset minus 16px so the action row sits tight (~18px on
        // a home-bar iPhone instead of the full ~34px) — the buttons reach a bit
        // into the indicator zone, which is fine for taps. Floored to 4px on
        // devices without a home bar.
        pt: { xs: 1, sm: 1.5 },
        pb: { xs: "max(calc(env(safe-area-inset-bottom) - 16px), 4px)", sm: 1.5 },
        pl: { xs: "max(env(safe-area-inset-left), 12px)", sm: "max(env(safe-area-inset-left), 12px)" },
        pr: { xs: "max(env(safe-area-inset-right), 12px)", sm: "max(env(safe-area-inset-right), 12px)" },
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        position: "relative", // anchor for Popper portal placement
      }}
    >
      {/* Simple composer: a plain MUI outlined input on top (default theme
          radius — no oversized pill), with one action row beneath it. */}
      <TextField
        fullWidth
        multiline
        minRows={1}
        maxRows={12}
        size="small"
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
          // @-file picker keyboard control — same precedence over send/newline.
          if (atOpen) {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setAtIndex((i) => (i + 1) % atFiles.length);
              return;
            }
            if (e.key === "ArrowUp") {
              e.preventDefault();
              setAtIndex((i) => (i - 1 + atFiles.length) % atFiles.length);
              return;
            }
            if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.ctrlKey)) {
              const file = atFiles[atIndex];
              if (file) {
                e.preventDefault();
                insertFile(file);
                return;
              }
            }
            if (e.key === "Escape") {
              e.preventDefault();
              setAtDismissed(true);
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
              fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, sans-serif",
              // 16px on touch so iOS/iPadOS Safari doesn't auto-zoom on focus
              // (it never zooms back out); 14px on desktop. See index.html.
              fontSize: 14,
              "@media (pointer: coarse)": { fontSize: 16 },
            },
          },
        }}
      />
      {/* Action row below the input: slash-command / @-reference triggers on
          the left, then the agent config (inline chips on desktop, the bottom
          sheet on touch), then the send button. Buttons are 40px on touch so
          the side safe-area floor keeps them off the iPhone corner radius. */}
      <Stack direction="row" alignItems="center" spacing={0.5} sx={{ mt: 0.75, minHeight: 40 }}>
        <Tooltip title="Slash command / skill">
          <span>
            <IconButton
              aria-label="slash command"
              disabled={dead}
              sx={{ width: { xs: 40, lg: 36 }, height: { xs: 40, lg: 36 }, flexShrink: 0 }}
              onClick={(): void => appendToken("/")}
            >
              <Box component="span" sx={{ fontSize: 18, fontWeight: 700, lineHeight: 1 }}>
                /
              </Box>
            </IconButton>
          </span>
        </Tooltip>
        <Tooltip title="Reference a file (@)">
          <span>
            <IconButton
              aria-label="reference a file"
              disabled={dead}
              sx={{ width: { xs: 40, lg: 36 }, height: { xs: 40, lg: 36 }, flexShrink: 0 }}
              onClick={(): void => appendToken("@")}
            >
              <AlternateEmail fontSize="small" />
            </IconButton>
          </span>
        </Tooltip>

        {compact ? (
          <Tooltip title="Options">
            <span>
              <IconButton
                aria-label="options"
                disabled={dead}
                sx={{ width: 40, height: 40, flexShrink: 0 }}
                onClick={(): void => setSheetOpen(true)}
              >
                <Tune fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
        ) : (
          <Stack
            direction="row"
            alignItems="center"
            spacing={0.5}
            sx={{
              minWidth: 0,
              overflowX: "auto",
              scrollbarWidth: "thin",
              "&::-webkit-scrollbar": { height: 6 },
            }}
          >
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
          </Stack>
        )}

        <Box sx={{ flex: 1 }} />

        {!compact && (
          <Typography
            variant="caption"
            color="text.disabled"
            sx={{ whiteSpace: "nowrap", fontSize: 11, flexShrink: 0, mr: 0.5 }}
          >
            ⌘/Ctrl + Enter = send
          </Typography>
        )}

        {busy ? (
          <Tooltip title="Stop">
            <IconButton
              color="error"
              aria-label="cancel"
              sx={{ width: { xs: 40, lg: 36 }, height: { xs: 40, lg: 36 }, flexShrink: 0 }}
              onClick={(): void => send({ type: "cancel", session_id: sessionId })}
            >
              <Stop />
            </IconButton>
          </Tooltip>
        ) : (
          <Tooltip title="Send (⌘/Ctrl + Enter)">
            <span>
              <IconButton
                color="primary"
                aria-label="send"
                disabled={!sendable}
                sx={{ width: { xs: 40, lg: 36 }, height: { xs: 40, lg: 36 }, flexShrink: 0 }}
                onClick={submit}
              >
                <Send />
              </IconButton>
            </span>
          </Tooltip>
        )}
      </Stack>
      {compact && (
        <ComposerSheet
          open={sheetOpen}
          onClose={(): void => setSheetOpen(false)}
          session={session}
          options={options}
          loading={showSkeleton}
          dead={dead}
          onSelectOption={(configId, value): void =>
            send({
              type: "set_config_option",
              session_id: sessionId,
              config_id: configId,
              value,
            })
          }
        />
      )}
      <SlashPicker
        open={slashOpen && filteredCommands.length > 0}
        anchorEl={textFieldRef.current}
        commands={filteredCommands}
        selectedIndex={slashIndex}
        onSelect={insertCommand}
        onHighlight={(i): void => setSlashIndex(i)}
        onClose={(): void => setText("")}
      />
      <AtPicker
        open={atOpen}
        anchorEl={textFieldRef.current}
        files={atFiles}
        selectedIndex={atIndex}
        onSelect={insertFile}
        onHighlight={(i): void => setAtIndex(i)}
        onClose={(): void => setAtDismissed(true)}
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
  // Desktop-only: ComposerSheet handles touch viewports now, so this just
  // needs the anchored Menu it always had.
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

// Unified bottom sheet for touch viewports: every action lives here, none
// in a visible inline row. ChatGPT / DeepSeek / Gemini all collapse their
// composer controls behind a single `+` because chip rows wrap awkwardly
// on iPad portrait (820px) and break entirely on a 390px iPhone, while
// the bottom-sheet pattern is iOS-native muscle memory.
function ComposerSheet({
  open,
  onClose,
  session,
  options,
  loading,
  dead,
  onSelectOption,
}: {
  open: boolean;
  onClose: () => void;
  session: SessionMeta | undefined;
  options: ConfigOption[];
  loading: boolean;
  dead: boolean;
  onSelectOption: (configId: string, value: string | boolean) => void;
}): React.JSX.Element {
  return (
    <Drawer
      anchor="bottom"
      open={open}
      onClose={onClose}
      slotProps={{
        paper: {
          sx: {
            borderTopLeftRadius: 16,
            borderTopRightRadius: 16,
            maxHeight: "85vh",
            pb: "max(env(safe-area-inset-bottom), 12px)",
            // Side insets so rows clear the notch / rounded corners in
            // landscape; 0 off-device, so desktop is unchanged.
            pl: "env(safe-area-inset-left)",
            pr: "env(safe-area-inset-right)",
          },
        },
      }}
    >
      {/* iOS-style drag handle — purely visual; tapping outside closes. */}
      <Box
        sx={{
          width: 36,
          height: 4,
          borderRadius: 2,
          bgcolor: "action.disabledBackground",
          mx: "auto",
          mt: 1,
          mb: 0.5,
        }}
      />
      {session && <SessionInfoSection session={session} />}
      {(loading || options.length > 0) && (
        <>
          <Divider />
          <Box sx={{ px: 2.5, py: 1.5 }}>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
            >
              Agent
            </Typography>
            {loading ? (
              <Stack
                direction="row"
                spacing={1.5}
                alignItems="center"
                sx={{ py: 1, color: "text.secondary" }}
              >
                <CircularProgress size={16} />
                <Typography variant="body2">Loading agent options…</Typography>
              </Stack>
            ) : (
              // Selecting a value does NOT close the sheet — mode, model, and
              // effort are commonly changed together, and each <Select> already
              // closes its own menu on pick. The user dismisses the sheet by
              // tapping outside once they're done.
              <Stack spacing={2} sx={{ mt: 1.5 }}>
                {options.map((opt) => (
                  <ConfigSheetDropdown
                    key={opt.id}
                    option={opt}
                    disabled={dead}
                    onSelect={(value): void => onSelectOption(opt.id, value)}
                  />
                ))}
              </Stack>
            )}
          </Box>
        </>
      )}
    </Drawer>
  );
}

// Read-only session metadata at the top of the options sheet. This used to
// live behind a long-press on the mobile title bar (a gesture nobody found),
// so it now rides the one popup the user already opens to change mode / model
// / effort. Desktop shows the same facts in the persistent sidebar, so this
// section only renders inside the compact-tier sheet.
function SessionInfoSection({
  session,
}: {
  session: SessionMeta;
}): React.JSX.Element {
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: "Provider", value: session.provider },
    { label: "Working dir", value: session.cwd, mono: true },
    { label: "Origin", value: originLabel(session.origin) },
    { label: "Status", value: session.status },
    { label: "Session id", value: session.id, mono: true },
  ];
  return (
    <>
      <Box sx={{ px: 2.5, pt: 0.5, pb: 0.25 }}>
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
        >
          Session
        </Typography>
      </Box>
      <List dense disablePadding>
        {rows.map((r) => (
          <SheetDetailRow
            key={r.label}
            label={r.label}
            value={r.value}
            mono={r.mono === true}
          />
        ))}
      </List>
    </>
  );
}

function SheetDetailRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        px: 2.5,
        py: 0.75,
        display: "flex",
        gap: 2,
        alignItems: "baseline",
      }}
    >
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ minWidth: 96, flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography
        variant="body2"
        sx={{
          flex: 1,
          minWidth: 0,
          wordBreak: "break-word",
          fontFamily: mono ? MONO : "inherit",
        }}
      >
        {value}
      </Typography>
    </Box>
  );
}

// One labelled dropdown per agent option (mode / model / effort). Collapses
// what used to be an always-expanded radio list — the sheet stays short even
// when an agent advertises a dozen models. ACP option values may be string OR
// boolean, so the <Select> is keyed on String(value) and mapped back to the
// original type on change.
function ConfigSheetDropdown({
  option,
  disabled,
  onSelect,
}: {
  option: ConfigOption;
  disabled: boolean;
  onSelect: (value: string | boolean) => void;
}): React.JSX.Element {
  const currentKey = String(option.currentValue);
  return (
    <TextField
      select
      fullWidth
      size="small"
      disabled={disabled}
      label={option.name}
      value={currentKey}
      onChange={(e): void => {
        const picked = option.options.find(
          (o) => String(o.value) === e.target.value,
        );
        if (picked) onSelect(picked.value);
      }}
      {...(option.description ? { helperText: option.description } : {})}
    >
      {option.options.map((o) => (
        <MenuItem key={String(o.value)} value={String(o.value)}>
          {o.name}
        </MenuItem>
      ))}
    </TextField>
  );
}

const MONO = "ui-monospace, SFMono-Regular, Menlo, monospace";

function SlashPicker({
  open,
  anchorEl,
  commands,
  selectedIndex,
  onSelect,
  onHighlight,
  onClose,
}: {
  open: boolean;
  anchorEl: HTMLElement | null;
  commands: AvailableCommand[];
  selectedIndex: number;
  onSelect: (name: string) => void;
  onHighlight: (i: number) => void;
  onClose: () => void;
}): React.JSX.Element {
  // Each row is a single ellipsized line (name + dimmed description) so the
  // list stays scannable — skill blurbs are 1-3 sentences and used to wrap to
  // 3-5 lines each, leaving only ~2 rows visible. Full text moves to an
  // on-demand detail surface that differs by input model:
  //
  //   Desktop (pointer: fine) — a footer pinned under the list always mirrors
  //   the highlighted row's full name + description. Arrow keys AND hover move
  //   the highlight, so full info is one glance away with no gesture (the
  //   editor-autocomplete "docs pane" idiom). A hover tooltip can't do this:
  //   the palette is driven by the keyboard, where there's no hovered element.
  //
  //   Touch (pointer: coarse) — no hover, so a long-press peeks a row's full
  //   text into the same footer; a plain tap still inserts. The footer is
  //   absent until a peek, so it doesn't steal vertical space from the list
  //   while the soft keyboard is already squeezing the viewport.
  const coarse = useMediaQuery("(pointer: coarse)");
  const [peekIndex, setPeekIndex] = useState<number | null>(null);
  // Drop a stale peek whenever the filtered set or open-state changes — the
  // index could otherwise point past the new list.
  useEffect(() => {
    setPeekIndex(null);
  }, [commands, open]);

  const detailIndex = coarse ? peekIndex : selectedIndex;
  const detail =
    detailIndex !== null && detailIndex >= 0 ? commands[detailIndex] : undefined;

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
            borderRadius: 1.5,
            overflow: "hidden",
          }}
        >
          <MenuList dense disablePadding sx={{ maxHeight: 280, overflowY: "auto" }}>
            {commands.map((c, i) => (
              <SlashRow
                key={c.name}
                command={c}
                selected={i === selectedIndex}
                coarse={coarse}
                onSelect={(): void => onSelect(c.name)}
                onHighlight={(): void => onHighlight(i)}
                onPeek={(): void => setPeekIndex(i)}
              />
            ))}
          </MenuList>
          {detail && (
            <Box
              sx={{
                borderTop: 1,
                borderColor: "divider",
                bgcolor: "action.hover",
                px: 1.5,
                py: 1,
              }}
            >
              <Typography
                variant="body2"
                sx={{ fontWeight: 700, fontFamily: MONO }}
              >
                /{detail.name}
              </Typography>
              {detail.description && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block", whiteSpace: "normal", mt: 0.25 }}
                >
                  {detail.description}
                </Typography>
              )}
            </Box>
          )}
        </Paper>
      </ClickAwayListener>
    </Popper>
  );
}

// One picker row: a single ellipsized line. On desktop, hovering raises the
// keyboard highlight (so the detail footer follows the mouse too). On touch, a
// ~450ms long-press peeks the full text without inserting; the `pressed` ref
// then swallows the click that fires on pointer-up so the peek doesn't also
// submit the command.
function SlashRow({
  command,
  selected,
  coarse,
  onSelect,
  onHighlight,
  onPeek,
}: {
  command: AvailableCommand;
  selected: boolean;
  coarse: boolean;
  onSelect: () => void;
  onHighlight: () => void;
  onPeek: () => void;
}): React.JSX.Element {
  const pressed = useRef(false);
  const timer = useRef<number | null>(null);
  const start = useRef({ x: 0, y: 0 });
  const cancel = useCallback((): void => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  // Touch gets long-press peek; desktop gets hover-to-highlight. Keeping the
  // two prop sets disjoint avoids a phantom hover firing on tap (some mobile
  // browsers synthesize mouseenter on touch).
  const interaction = coarse
    ? {
        onPointerDown: (e: React.PointerEvent): void => {
          pressed.current = false;
          start.current = { x: e.clientX, y: e.clientY };
          timer.current = window.setTimeout((): void => {
            pressed.current = true;
            onPeek();
          }, 450);
        },
        onPointerUp: cancel,
        onPointerCancel: cancel,
        onPointerLeave: cancel,
        onPointerMove: (e: React.PointerEvent): void => {
          // A scroll start (noticeable move) cancels the press so dragging the
          // list doesn't peek.
          if (
            Math.abs(e.clientX - start.current.x) > 8 ||
            Math.abs(e.clientY - start.current.y) > 8
          ) {
            cancel();
          }
        },
        // Swallow the OS long-press context menu so the peek lands on us.
        onContextMenu: (e: React.MouseEvent): void => e.preventDefault(),
      }
    : { onMouseEnter: onHighlight };

  return (
    <MenuItem
      selected={selected}
      onClick={(): void => {
        if (pressed.current) {
          pressed.current = false;
          return;
        }
        onSelect();
      }}
      sx={{ py: 0.5 }}
      {...interaction}
    >
      <Box
        sx={{
          display: "flex",
          alignItems: "baseline",
          gap: 1,
          width: "100%",
          minWidth: 0,
        }}
      >
        <Typography
          variant="body2"
          sx={{
            fontWeight: 600,
            fontFamily: MONO,
            flexShrink: 0,
            maxWidth: "60%",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          /{command.name}
        </Typography>
        {command.description && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{
              flex: 1,
              minWidth: 0,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {command.description}
          </Typography>
        )}
      </Box>
    </MenuItem>
  );
}

// File picker for the `@` reference. Mirrors SlashPicker's Popper + keyboard
// model, but rows are file paths: the basename leads (monospace) with the
// parent directory trailing, dimmed, so a long path stays scannable. No detail
// footer — a path is its own description.
function AtPicker({
  open,
  anchorEl,
  files,
  selectedIndex,
  onSelect,
  onHighlight,
  onClose,
}: {
  open: boolean;
  anchorEl: HTMLElement | null;
  files: string[];
  selectedIndex: number;
  onSelect: (path: string) => void;
  onHighlight: (i: number) => void;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Popper
      open={open}
      anchorEl={anchorEl}
      placement="top-start"
      modifiers={[{ name: "offset", options: { offset: [0, 8] } }]}
      sx={{ zIndex: (theme): number => theme.zIndex.modal + 1 }}
    >
      <ClickAwayListener onClickAway={onClose}>
        <Paper
          elevation={6}
          sx={{
            width: anchorEl ? anchorEl.clientWidth : "auto",
            maxWidth: "min(560px, 92vw)",
            borderRadius: 1.5,
            overflow: "hidden",
          }}
        >
          <MenuList dense disablePadding sx={{ maxHeight: 280, overflowY: "auto" }}>
            {files.map((path, i) => (
              <AtRow
                key={path}
                path={path}
                selected={i === selectedIndex}
                onSelect={(): void => onSelect(path)}
                onHighlight={(): void => onHighlight(i)}
              />
            ))}
          </MenuList>
        </Paper>
      </ClickAwayListener>
    </Popper>
  );
}

function AtRow({
  path,
  selected,
  onSelect,
  onHighlight,
}: {
  path: string;
  selected: boolean;
  onSelect: () => void;
  onHighlight: () => void;
}): React.JSX.Element {
  const slash = path.lastIndexOf("/");
  const name = slash >= 0 ? path.slice(slash + 1) : path;
  const dir = slash >= 0 ? path.slice(0, slash) : "";
  return (
    <MenuItem
      selected={selected}
      onClick={onSelect}
      onMouseEnter={onHighlight}
      sx={{ py: 0.5 }}
    >
      <Box
        sx={{
          display: "flex",
          alignItems: "baseline",
          gap: 1,
          width: "100%",
          minWidth: 0,
        }}
      >
        <Typography
          variant="body2"
          sx={{
            fontFamily: MONO,
            flexShrink: 0,
            maxWidth: "60%",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {name}
        </Typography>
        {dir && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{
              flex: 1,
              minWidth: 0,
              fontFamily: MONO,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {dir}
          </Typography>
        )}
      </Box>
    </MenuItem>
  );
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

// Trailing `@token` matcher: an `@` at the start of input or after whitespace,
// followed by the run of non-space chars up to the end of the input. Anchored
// to `$` so it only fires while the reference is the thing being actively
// typed; a space (or a newline) after the token ends the match and closes the
// picker. Reused by `insertFile` to locate the slice to replace.
const AT_QUERY_RE = /(?:^|\s)@(\S*)$/;

function parseAtQuery(text: string): string | null {
  const m = AT_QUERY_RE.exec(text);
  return m ? (m[1] ?? "") : null;
}
