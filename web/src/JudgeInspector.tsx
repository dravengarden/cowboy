import { useEffect, useState, useSyncExternalStore } from "react";
import {
  alpha,
  Box,
  Button,
  Chip,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import type { PaletteColor, Theme } from "@mui/material";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";
import DeleteOutlineRounded from "@mui/icons-material/DeleteOutlineRounded";
import FactCheckOutlined from "@mui/icons-material/FactCheckOutlined";
import type { JudgeRun } from "./protocol";
import { clearJudgeRuns, removeJudgeRun, useJudgeRuns } from "./store";
import { Sheet } from "./Sheet";
import { Kbd } from "./Kbd";
import { MOD_LABEL } from "./platform";

// The judge-run inspector: long-press the turn-status pill opens this. It lists
// the confirm-detect judge runs for a session (server-authoritative history,
// newest first) so the user can audit WHY each turn-end held / drained / was
// marked done. Each row shows the verdict; tap to expand its raw LLM input
// (the agent's final text) + output (the model's raw JSON). Delete one run or
// clear the whole history — both are durable, cross-terminal mutations.

type PaletteKey = "primary" | "success" | "info";

// awaiting (a question) = purple, done = green, neither (continued / cut off) =
// neutral info. Mirrors the overlay pill's palette so the two surfaces agree.
function verdictTone(r: JudgeRun): { key: PaletteKey; label: string } {
  if (r.awaiting_user) return { key: "primary", label: "Awaiting reply" };
  if (r.done) return { key: "success", label: "Task done" };
  return { key: "info", label: "No hold" };
}

function relTime(at: number): string {
  const s = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (s < 60) return `${String(s)}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${String(m)}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${String(h)}h ago`;
  return `${String(Math.round(h / 24))}d ago`;
}

function RawBlock({ label, text }: { label: string; text: string }): React.JSX.Element {
  return (
    <Box>
      <Typography variant="caption" sx={{ fontWeight: 600, color: "text.secondary" }}>
        {label}
      </Typography>
      <Box
        component="pre"
        sx={{
          m: 0,
          mt: 0.25,
          p: 1,
          borderRadius: 1.5,
          bgcolor: (t) => alpha(t.palette.text.primary, t.palette.mode === "dark" ? 0.06 : 0.04),
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
          fontSize: 11,
          lineHeight: 1.5,
          maxHeight: 220,
          overflow: "auto",
        }}
      >
        {text || "—"}
      </Box>
    </Box>
  );
}

function RunRow({
  run,
  expanded,
  onToggle,
  onDelete,
}: {
  run: JudgeRun;
  expanded: boolean;
  onToggle: () => void;
  onDelete: () => void;
}): React.JSX.Element {
  const tone = verdictTone(run);
  const color = (t: Theme): PaletteColor => t.palette[tone.key];
  return (
    <Box
      sx={{
        borderRadius: 2,
        overflow: "hidden",
        border: (t) => `1px solid ${alpha(t.palette.divider, 0.6)}`,
        bgcolor: (t) => alpha(color(t).main, t.palette.mode === "dark" ? 0.08 : 0.06),
      }}
    >
      {/* Summary — tap to expand. The delete button stops propagation. */}
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        onClick={onToggle}
        sx={{ px: 1.25, py: 1, cursor: "pointer", minWidth: 0 }}
      >
        <Box
          sx={{
            width: 8,
            height: 8,
            borderRadius: "50%",
            flexShrink: 0,
            bgcolor: (t) => color(t).main,
          }}
        />
        <Stack sx={{ minWidth: 0, flex: 1 }}>
          <Stack direction="row" alignItems="center" spacing={0.75}>
            <Typography
              variant="body2"
              sx={{ fontWeight: 600, color: (t) => color(t).main, whiteSpace: "nowrap" }}
            >
              {tone.label}
            </Typography>
            <Chip
              label={run.layer}
              size="small"
              sx={{ height: 16, fontSize: 10, "& .MuiChip-label": { px: 0.5 } }}
            />
            <Typography variant="caption" sx={{ color: "text.disabled", whiteSpace: "nowrap" }}>
              {relTime(run.at)}
            </Typography>
          </Stack>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {run.reason || "—"}
          </Typography>
        </Stack>
        <IconButton
          size="small"
          aria-label="delete run"
          onClick={(e): void => {
            e.stopPropagation();
            onDelete();
          }}
          sx={{ color: "text.disabled", "&:hover": { color: "error.main" } }}
        >
          <DeleteOutlineRounded sx={{ fontSize: 18 }} />
        </IconButton>
        <ExpandMoreRounded
          sx={{
            fontSize: 20,
            color: "text.disabled",
            transform: expanded ? "rotate(180deg)" : "none",
            transition: "transform .15s",
          }}
        />
      </Stack>
      {expanded && (
        <Stack spacing={1} sx={{ px: 1.25, pb: 1.25, pt: 0.5 }}>
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            {run.model || "no model"} · {run.latency_ms}ms · cache {run.cache_hit}/{run.cache_miss}
            {" "}· conf {run.confidence.toFixed(2)}
          </Typography>
          <RawBlock label="Input" text={run.input} />
          <RawBlock label="Output" text={run.output} />
        </Stack>
      )}
    </Box>
  );
}

function JudgeInspector({
  sessionId,
  open,
  onClose,
  forceSheet,
}: {
  sessionId: string;
  open: boolean;
  onClose: () => void;
  forceSheet: boolean;
}): React.JSX.Element {
  const runs = useJudgeRuns(sessionId);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    if (!open || runs.length === 0) return undefined;
    const onKey = (event: KeyboardEvent): void => {
      if (
        event.key !== "Backspace" ||
        (!event.metaKey && !event.ctrlKey) ||
        event.repeat ||
        event.isComposing
      ) return;
      event.preventDefault();
      event.stopPropagation();
      clearJudgeRuns(sessionId);
    };
    globalThis.addEventListener("keydown", onKey, true);
    return (): void => globalThis.removeEventListener("keydown", onKey, true);
  }, [open, runs.length, sessionId]);

  return (
    <Sheet
      forceSheet={forceSheet}
      open={open}
      onClose={onClose}
      title="Judge runs"
      actions={
        <>
          {runs.length > 0 && (
            <Button
              onClick={(): void => clearJudgeRuns(sessionId)}
              color="error"
              sx={{ mr: "auto" }}
            >
              Clear all
              <Kbd keys={`${MOD_LABEL}⌫`} />
            </Button>
          )}
          <Button onClick={onClose} color="inherit">
            Close
            <Kbd keys="Esc" />
          </Button>
        </>
      }
    >
      {runs.length === 0
        ? (
          <Stack alignItems="center" spacing={1.5} sx={{ py: 5, color: "text.disabled" }}>
            <FactCheckOutlined sx={{ fontSize: 40, opacity: 0.6 }} />
            <Typography variant="body2">No judge runs yet.</Typography>
            <Typography variant="caption" sx={{ textAlign: "center", maxWidth: 260 }}>
              The confirm-detect judge records each turn-end here — its verdict and
              the raw input/output it saw.
            </Typography>
          </Stack>
        )
        : (
          <Stack spacing={1} sx={{ mt: 1 }}>
            {runs.map((r) => (
              <RunRow
                key={r.id}
                run={r}
                expanded={expandedId === r.id}
                onToggle={(): void => setExpandedId((cur) => (cur === r.id ? null : r.id))}
                onDelete={(): void => removeJudgeRun(sessionId, r.id)}
              />
            ))}
          </Stack>
        )}
    </Sheet>
  );
}

// A single module-level store drives ONE inspector mounted at the app root, so
// the long-press handler anywhere (the turn-status pill) opens it by id without
// prop-drilling open-state + `forceSheet` (the navbar-at-bottom flag, known only
// at the root) down through Composer → TurnStatusOverlay.
let openSid: string | null = null;
let lastSid = ""; // retained during the close transition so content doesn't blank
const inspectorSubs = new Set<() => void>();

/** Open the judge inspector for a session (called from the pill's long-press). */
export function openJudgeInspector(sessionId: string): void {
  openSid = sessionId;
  lastSid = sessionId;
  for (const s of inspectorSubs) s();
}

function closeInspector(): void {
  openSid = null;
  for (const s of inspectorSubs) s();
}

/** Mount once at the app root. `forceSheet` = the navbar-at-bottom flag. */
export function JudgeInspectorHost({ forceSheet }: { forceSheet: boolean }): React.JSX.Element | null {
  const sid = useSyncExternalStore(
    (cb) => {
      inspectorSubs.add(cb);
      return () => {
        inspectorSubs.delete(cb);
      };
    },
    () => openSid,
    () => null,
  );
  if (lastSid === "") return null; // never opened this session
  return (
    <JudgeInspector
      sessionId={lastSid}
      open={sid !== null}
      onClose={closeInspector}
      forceSheet={forceSheet}
    />
  );
}
