import { MenuItem, Stack, TextField, Typography } from "@mui/material";
import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { notifyLineageChanged } from "./ancestorHistory";
import { derive } from "./derive";
import type { ForkConfig } from "./ForkConfiguration";
import type { Envelope, MachineSummary, SessionMeta } from "./protocol";
import { duplicateMode, duplicateSession, forkAvailability, forkSession } from "./sessionFork";
import { requestPickSession } from "./sessionPickRequest";
import {
  closeSessionSettings,
  type ForkPointSelection,
  SESSION_NEW_FROM_EVENT,
  takeNewFromRequestDetail,
} from "./sessionSettingsOpen";
import { useStoreSelector } from "./store";
import {
  extractCompletedTurnForkPoints,
  formatTurnForkLabel,
  latestContextClearSeq,
  type TurnForkPoint,
} from "./turnForkPoints";
import { newUuid } from "./uuid";

const EMPTY_TIMELINE: Envelope[] = [];
export const LATEST_FORK_POINT_KEY = "latest";

export type PendingSettingsFork =
  | { mode: "fork"; operation: string; messageId?: string | undefined; config: ForkConfig }
  | {
      mode: "linked";
      operation: string;
      body: {
        provider: string;
        provider_account_id: string;
        preset: string | null;
        inherit_settings: boolean;
        config: ForkConfig;
        before_seq?: number | undefined;
        include_uncommitted?: boolean;
      };
    };

export function buildSettingsForkAttempt(input: {
  forkMode: "fork" | "linked";
  provider: string;
  account: string;
  preset: string | undefined;
  inheritSettings: boolean;
  overrides: ForkConfig;
  effectiveIncludeUncommitted: boolean | undefined;
  forkPoint?: Pick<TurnForkPoint, "messageId" | "beforeSeq"> | null | undefined;
}): PendingSettingsFork {
  if (input.forkMode === "fork") {
    return {
      mode: "fork",
      operation: `duplicate-${newUuid()}`,
      ...(input.forkPoint?.messageId ? { messageId: input.forkPoint.messageId } : {}),
      config: input.overrides,
    };
  }
  return {
    mode: "linked",
    operation: `duplicate-${newUuid()}`,
    body: {
      provider: input.provider,
      provider_account_id: input.account,
      preset: input.preset || null,
      inherit_settings: input.inheritSettings,
      config: input.overrides,
      ...(input.forkPoint ? { before_seq: input.forkPoint.beforeSeq } : {}),
      ...(input.effectiveIncludeUncommitted !== undefined
        ? { include_uncommitted: input.effectiveIncludeUncommitted }
        : {}),
    },
  };
}

export function submitSettingsFork(sessionId: string, attempt: PendingSettingsFork) {
  return attempt.mode === "fork"
    ? forkSession(sessionId, {
        operation: attempt.operation,
        ...(attempt.messageId ? { messageId: attempt.messageId } : {}),
        config: attempt.config,
      })
    : duplicateSession(sessionId, attempt.body, attempt.operation);
}

/** Coordinates Replace-or-fork expansion, pre-selected turn requests (from
 *  Desktop hover or session menus), and scroll reveal into view. */
export function useNewFromDisclosure(
  sessionId: string,
  hasData: boolean,
  inlineLinks: boolean,
  actionsRef: RefObject<HTMLDivElement | null>,
): {
  open: boolean;
  setOpen: React.Dispatch<React.SetStateAction<boolean>>;
  requestedForkPoint: ForkPointSelection | null;
  toggleOpen: () => void;
} {
  const [open, setOpen] = useState(false);
  const [reveal, setReveal] = useState(false);
  const [requestedForkPoint, setRequestedForkPoint] = useState<ForkPointSelection | null>(null);

  useEffect(() => {
    const take = () => {
      const detail = takeNewFromRequestDetail(sessionId);
      if (!detail.matched) return;
      setRequestedForkPoint(detail.forkPoint ?? null);
      setOpen(true);
      setReveal(true);
    };
    take();
    document.addEventListener(SESSION_NEW_FROM_EVENT, take);
    return () => document.removeEventListener(SESSION_NEW_FROM_EVENT, take);
  }, [sessionId]);

  const ready = open && hasData;
  useEffect(() => {
    if (!reveal || !ready) return undefined;
    const show = () => {
      const target = actionsRef.current;
      if (!target) return;
      if (
        inlineLinks &&
        target.parentElement?.firstElementChild === target &&
        target.parentElement.previousElementSibling === null
      ) {
        return;
      }
      target.scrollIntoView({ block: "start" });
    };
    const frame = requestAnimationFrame(show);
    const settled = setTimeout(() => {
      show();
      setReveal(false);
    }, 400);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(settled);
    };
  }, [actionsRef, inlineLinks, reveal, ready]);

  return {
    open,
    setOpen,
    requestedForkPoint,
    toggleOpen: () => {
      setRequestedForkPoint(null);
      setOpen((value) => !value);
    },
  };
}

/** Tracks completed turn-final fork points for a session and keeps any
 *  pre-selected turn synchronized while clearing it on context reset. */
export function useSessionForkPoints(
  session: Pick<SessionMeta, "id" | "status">,
  initialForkPoint: ForkPointSelection | null | undefined,
): {
  turns: TurnForkPoint[];
  selectedKey: string;
  selectKey: (key: string) => void;
  selectedTurn: TurnForkPoint | null;
} {
  const timeline = useStoreSelector((state) => state.timelines.get(session.id) ?? EMPTY_TIMELINE);
  const sourceBusy = session.status === "busy" || session.status === "starting";
  const items = useMemo(() => derive(timeline), [timeline]);
  const turns = useMemo(
    () => extractCompletedTurnForkPoints(items, sourceBusy),
    [items, sourceBusy],
  );
  const clearSeq = useMemo(() => latestContextClearSeq(items), [items]);
  const [selectedKey, setSelectedKey] = useState<string>(
    initialForkPoint?.itemKey ?? LATEST_FORK_POINT_KEY,
  );

  useEffect(() => {
    setSelectedKey(initialForkPoint?.itemKey ?? LATEST_FORK_POINT_KEY);
  }, [initialForkPoint, session.id]);

  const syntheticSelected = useMemo((): TurnForkPoint | null => {
    if (!initialForkPoint) return null;
    if (clearSeq !== null && clearSeq >= initialForkPoint.beforeSeq) return null;
    return {
      turnNumber: initialForkPoint.turnNumber ?? Math.max(1, turns.length),
      itemKey: initialForkPoint.itemKey,
      messageId: initialForkPoint.messageId,
      beforeSeq: initialForkPoint.beforeSeq,
      promptPreview: initialForkPoint.promptPreview || "Selected response",
      answerPreview: "",
    };
  }, [clearSeq, initialForkPoint, turns.length]);

  const selectedTurn = useMemo((): TurnForkPoint | null => {
    if (selectedKey === LATEST_FORK_POINT_KEY) return null;
    const live = turns.find((turn) => turn.itemKey === selectedKey);
    if (live) return live;
    if (syntheticSelected && syntheticSelected.itemKey === selectedKey) return syntheticSelected;
    return null;
  }, [selectedKey, syntheticSelected, turns]);

  useEffect(() => {
    if (selectedKey !== LATEST_FORK_POINT_KEY && selectedTurn === null) {
      setSelectedKey(LATEST_FORK_POINT_KEY);
    }
  }, [selectedKey, selectedTurn]);

  return {
    turns,
    selectedKey: selectedTurn ? selectedKey : LATEST_FORK_POINT_KEY,
    selectKey: setSelectedKey,
    selectedTurn,
  };
}

/** Computes effective fork mode, busy/queue waiting rules, and idempotent fork
 *  execution for `NewSessionForm`. */
export function useSettingsForkPlan(input: {
  session: SessionMeta;
  machines: readonly MachineSummary[];
  allowFork: boolean;
  samePlacement: boolean;
  supportsWorkspaceMode: boolean;
  sourceAccount: string;
  provider: string;
  account: string;
  preset: string | undefined;
  inheritSettings: boolean;
  overrides: ForkConfig;
  copyUncommitted: boolean;
  queued: number;
  initialForkPoint: ForkPointSelection | null;
  onForked: () => void;
}) {
  const { session, machines, allowFork, samePlacement, supportsWorkspaceMode } = input;
  const forkPoints = useSessionForkPoints(session, input.initialForkPoint);
  const selectedTurn = allowFork && samePlacement ? forkPoints.selectedTurn : null;
  const fork = forkAvailability(session, machines);
  const baseForkMode = duplicateMode({
    sourceProvider: session.provider,
    sourceAccount: input.sourceAccount,
    provider: input.provider,
    account: input.account,
    forkAvailable: fork !== null && !input.preset && input.inheritSettings,
  });
  const forkMode: "fork" | "linked" =
    baseForkMode === "fork" && (!selectedTurn || Boolean(selectedTurn.messageId))
      ? "fork"
      : "linked";
  const sourceBusy = session.status === "busy" || session.status === "starting";
  const waitForTurn = selectedTurn === null && forkMode === "fork" && fork === "busy";
  const forkWaits = waitForTurn || (selectedTurn === null && forkMode === "fork" && input.queued > 0);
  const effectiveIncludeUncommitted = supportsWorkspaceMode
    ? sourceBusy
      ? false
      : input.copyUncommitted
    : undefined;
  const choiceSignature = JSON.stringify([
    input.provider,
    input.account,
    input.preset,
    input.inheritSettings,
    input.overrides,
    input.copyUncommitted,
    selectedTurn?.itemKey ?? LATEST_FORK_POINT_KEY,
  ]);
  const pendingForkRef = useRef<{ signature: string; attempt: PendingSettingsFork } | null>(null);

  const executeFork = async (): Promise<void> => {
    if (pendingForkRef.current?.signature !== choiceSignature) {
      pendingForkRef.current = {
        signature: choiceSignature,
        attempt: buildSettingsForkAttempt({
          forkMode,
          provider: input.provider,
          account: input.account,
          preset: input.preset,
          inheritSettings: input.inheritSettings,
          overrides: input.overrides,
          effectiveIncludeUncommitted,
          forkPoint: selectedTurn,
        }),
      };
    }
    const receipt = await submitSettingsFork(session.id, pendingForkRef.current.attempt);
    pendingForkRef.current = null;
    notifyLineageChanged(session.id);
    notifyLineageChanged(receipt.session_id);
    input.onForked();
    closeSessionSettings(session.id);
    requestPickSession(receipt.session_id, receipt.session ?? undefined);
  };

  return {
    forkPoints,
    selectedTurn,
    forkMode,
    sourceBusy,
    waitForTurn,
    forkWaits,
    executeFork,
  };
}

/** Compact turn selector inside Replace or fork so both Mobile and Desktop can
 *  branch from any completed turn without putting noisy controls in the transcript. */
export function ForkPointPicker({
  turns,
  selectedKey,
  selectedTurn,
  sourceBusy,
  disabled,
  desktop,
  onChange,
}: {
  turns: readonly TurnForkPoint[];
  selectedKey: string;
  selectedTurn: TurnForkPoint | null;
  sourceBusy: boolean;
  disabled: boolean;
  desktop: boolean;
  onChange: (key: string) => void;
}): React.JSX.Element | null {
  if (turns.length === 0 && !selectedTurn) return null;
  const newestFirst = [...turns].reverse();
  const hasSynthetic =
    selectedTurn !== null && !turns.some((turn) => turn.itemKey === selectedTurn.itemKey);
  const newestCompletedKey = turns.at(-1)?.itemKey;

  const renderLabel = (): string => {
    if (!selectedTurn) return "Whole conversation (latest)";
    return formatTurnForkLabel(selectedTurn);
  };

  return (
    <TextField
      select
      fullWidth
      size={desktop ? "small" : "medium"}
      label="Fork from"
      value={selectedKey}
      disabled={disabled}
      data-fork-point-picker
      slotProps={{
        select: { renderValue: renderLabel },
        inputLabel: { shrink: true },
      }}
      helperText={
        selectedTurn
          ? "Applies to Fork. Choose Whole conversation (latest) to replace this session."
          : sourceBusy
            ? "Source is running — choose a completed turn above to fork right now."
            : undefined
      }
      onChange={(event): void => onChange(event.target.value)}
    >
      <MenuItem value={LATEST_FORK_POINT_KEY}>
        <Stack sx={{ minWidth: 0, whiteSpace: "normal" }}>
          <span>Whole conversation (latest)</span>
          {sourceBusy && (
            <Typography variant="caption" color="text.secondary">
              Native fork waits for the current turn
            </Typography>
          )}
        </Stack>
      </MenuItem>
      {hasSynthetic && selectedTurn && (
        <MenuItem value={selectedTurn.itemKey}>
          <Stack sx={{ minWidth: 0, whiteSpace: "normal" }}>
            <span>{formatTurnForkLabel(selectedTurn)}</span>
          </Stack>
        </MenuItem>
      )}
      {newestFirst.map((turn) => {
        const isLatestCompleted = sourceBusy && turn.itemKey === newestCompletedKey;
        const subtitle = isLatestCompleted
          ? `Latest completed · fork now${turn.answerPreview ? ` · ${turn.answerPreview}` : ""}`
          : turn.answerPreview;
        return (
          <MenuItem key={turn.itemKey} value={turn.itemKey}>
            <Stack sx={{ minWidth: 0, whiteSpace: "normal" }}>
              <span>{formatTurnForkLabel(turn)}</span>
              {subtitle && (
                <Typography variant="caption" color="text.secondary" noWrap>
                  {subtitle}
                </Typography>
              )}
            </Stack>
          </MenuItem>
        );
      })}
    </TextField>
  );
}
