import { useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  CircularProgress,
  Divider,
  IconButton,
  List,
  Menu,
  MenuItem,
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
import { ComposerEditor, type ComposerEditorHandle } from "./ComposerEditor";
import { useVimSetting } from "./vimSetting";
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
import { BottomSheet } from "./_shell";

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
  const editorRef = useRef<ComposerEditorHandle>(null);
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

  // Slash skills + `@` file references are handled inside the editor now, via
  // CodeMirror autocomplete (see ComposerEditor + composerCompletions): no more
  // Popper pickers or caret/regex bookkeeping here. The editor reads the
  // agent-advertised `/` commands through a thunk; `@` files come from the
  // daemon's `/api/sessions/{id}/files` search.
  const availableCommands = useMemo(
    () => latestAvailableCommands(timelines.get(sessionId) ?? []),
    [timelines, sessionId],
  );

  // Vim is opt-in and desktop-only — ComposerEditor gates the actual
  // `@replit/codemirror-vim` load on a precise-pointer device, so touch never
  // pays for it. The reactive setting is flipped by the Settings toggle.
  const vim = useVimSetting();

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
        // home-indicator inset minus 20px so the action row sits tight (~14px on
        // a home-bar iPhone instead of the full ~34px) — the buttons reach a bit
        // into the indicator zone, which is fine for taps. Floored to 2px on
        // devices without a home bar.
        pt: { xs: 1, sm: 1.5 },
        pb: { xs: "max(calc(env(safe-area-inset-bottom) - 20px), 2px)", sm: 1.5 },
        pl: { xs: "max(env(safe-area-inset-left), 20px)", sm: "max(env(safe-area-inset-left), 20px)" },
        pr: { xs: "max(env(safe-area-inset-right), 20px)", sm: "max(env(safe-area-inset-right), 20px)" },
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        position: "relative", // anchor for Popper portal placement
      }}
    >
      {/* CodeMirror-6 editor styled as a MUI outlined field — replaces the
          <textarea> (which forced the iOS keyboard bar) and folds the `@`/`/`
          pickers into CM autocomplete. */}
      <ComposerEditor
        ref={editorRef}
        value={text}
        onChange={setText}
        onSubmit={submit}
        sessionId={sessionId}
        commands={(): AvailableCommand[] => availableCommands}
        placeholder={dead ? "Session ended" : "Message the agent…"}
        disabled={dead}
        vim={vim}
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
              onClick={(): void => editorRef.current?.insertTrigger("/")}
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
              onClick={(): void => editorRef.current?.insertTrigger("@")}
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
    <BottomSheet open={open} onClose={onClose}>
      {session && <SessionInfoSection session={session} />}
      {(loading || options.length > 0) && (
        <>
          <Divider />
          <Box sx={{ py: 1.5 }}>
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
    </BottomSheet>
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
      <Box sx={{ pt: 0.5, pb: 0.25 }}>
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

