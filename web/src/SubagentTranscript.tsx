// A subagent's observed full transcript in the Background tasks inspector,
// rendered by the same Transcript as a session or a Call's conversation
// (docs/subagents.md#observed-transcripts). Read-only: it never opens, wakes
// or prompts the child. A subagent nothing was observed for keeps the bounded
// snapshot (SubagentDetail).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Box, Button, Typography } from "@mui/material";
import { Transcript } from "./Transcript";
import { useStoreSelector } from "./store";
import { type Subagent, subagentStatus } from "./subagents";
import {
  afterCursor,
  EMPTY_TRANSCRIPT,
  fetchObservedPage,
  mergeObserved,
  type ObservedTranscript,
  observedTimeline,
} from "./observedTranscript";

/** The same floating-dock clearance as a Call's conversation on a phone. */
const MOBILE_FOOTER_CLEARANCE =
  "var(--cowboy-sheet-dock-clearance, calc(76px + env(safe-area-inset-bottom, 0px)))";
/** Poll interval while the subagent works. */
const LIVE_POLL_MS = 1500;
/** Transcripts kept for an instant reopen. */
const CACHED = 16;
const cache = new Map<string, ObservedTranscript>();

function remember(key: string, transcript: ObservedTranscript): void {
  cache.delete(key);
  cache.set(key, transcript);
  while (cache.size > CACHED) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

export interface SubagentTranscriptState {
  transcript: ObservedTranscript;
  /** The first page arrived (or failed). */
  loaded: boolean;
  /** Something was observed: show the transcript, not the snapshot. */
  observed: boolean;
  loadEarlier: () => void;
}

export function useSubagentTranscript(
  sessionId: string,
  subagent: Subagent | null,
): SubagentTranscriptState {
  const childId = subagent?.id ?? null;
  const key = childId ? `${sessionId}\u0000${childId}` : "";
  const [state, setState] = useState<{
    key: string;
    transcript: ObservedTranscript;
    loaded: boolean;
  }>(() => ({ key, transcript: cache.get(key) ?? EMPTY_TRANSCRIPT, loaded: cache.has(key) }));
  const current =
    state.key === key
      ? state
      : { key, transcript: cache.get(key) ?? EMPTY_TRANSCRIPT, loaded: cache.has(key) };
  const latest = useRef(current);
  latest.current = current;
  const tone = subagent ? subagentStatus(subagent).tone : "unknown";
  const working = tone === "working" || tone === "waiting";

  const fetchPage = useCallback(
    async (direction: "newest" | "after" | "before"): Promise<void> => {
      if (!childId) return;
      const shown = latest.current.transcript;
      const cursor =
        direction === "after"
          ? { after: afterCursor(shown) ?? 0 }
          : direction === "before"
            ? { before: shown.events[0]?.seq ?? Number.MAX_SAFE_INTEGER }
            : {};
      try {
        const page = await fetchObservedPage(sessionId, childId, cursor);
        if (latest.current.key !== key) return;
        const transcript = mergeObserved(latest.current.transcript, page, direction);
        remember(key, transcript);
        setState({ key, transcript, loaded: true });
      } catch {
        if (latest.current.key === key && !latest.current.loaded) {
          setState({ key, transcript: latest.current.transcript, loaded: true });
        }
      }
    },
    [childId, key, sessionId],
  );

  useEffect(() => {
    if (childId) void fetchPage(latest.current.transcript.events.length > 0 ? "after" : "newest");
  }, [childId, fetchPage]);

  // Live while it works, and once more after it stops for its final rows.
  useEffect(() => {
    if (!childId) return undefined;
    if (!working) {
      const settle = setTimeout(() => void fetchPage("after"), 500);
      return () => clearTimeout(settle);
    }
    const timer = setInterval(() => void fetchPage("after"), LIVE_POLL_MS);
    return () => clearInterval(timer);
  }, [childId, working, fetchPage]);

  const loadEarlier = useCallback(() => void fetchPage("before"), [fetchPage]);
  return {
    transcript: current.transcript,
    loaded: current.loaded,
    observed: current.transcript.events.length > 0,
    loadEarlier,
  };
}

/** Whether the selected task reads as a conversation: a Call always, a
 *  subagent once its observed transcript has rows. A phone that opened the
 *  subagent's details moves to its transcript as soon as it is known. */
export function useTaskConversation(
  sessionId: string,
  task: { source: string; subagent?: Subagent | undefined } | null,
  desktop: boolean,
  level: string,
  setLevel: (level: "conversation") => void,
  depth: { current: number },
): { child: SubagentTranscriptState; conversation: boolean } {
  const child = useSubagentTranscript(
    sessionId,
    task?.source === "subagent" ? (task.subagent ?? null) : null,
  );
  const conversation = task?.source === "call" || (task?.source === "subagent" && child.observed);
  useEffect(() => {
    if (!desktop && conversation && level === "detail" && depth.current === 2)
      setLevel("conversation");
  }, [desktop, conversation, level, setLevel, depth]);
  return { child, conversation };
}

export function SubagentConversation({
  sessionId,
  subagent,
  state,
  desktop,
}: {
  sessionId: string;
  subagent: Subagent;
  state: SubagentTranscriptState;
  desktop: boolean;
}): React.JSX.Element {
  const timelineId = `subagent:${sessionId}:${subagent.id}`;
  const timeline = useMemo(
    () => observedTimeline(timelineId, state.transcript, subagent.prompt),
    [timelineId, state.transcript, subagent.prompt],
  );
  const parent = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId),
  );
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const tone = subagentStatus(subagent).tone;
  const { transcript, loadEarlier } = state;
  return (
    <Box
      data-subagent-transcript={subagent.id}
      sx={{
        flex: 1,
        minWidth: 0,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        position: "relative",
      }}
    >
      {(transcript.moreBefore || transcript.gaps > 0) && (
        <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: desktop ? 0 : 2, py: 0.5 }}>
          {transcript.moreBefore && (
            <Button
              size="small"
              data-subagent-transcript-earlier
              onClick={loadEarlier}
              sx={{ textTransform: "none" }}
            >
              Show earlier activity
            </Button>
          )}
          {transcript.gaps > 0 && (
            <Typography variant="caption" color="text.secondary" data-subagent-transcript-gap>
              Some activity was not recorded.
            </Typography>
          )}
        </Box>
      )}
      <Transcript
        sessionId={timelineId}
        timeline={timeline}
        status={tone === "working" || tone === "waiting" ? "busy" : "running"}
        provider={parent?.provider ?? ""}
        providerVersion={parent?.provider_version}
        providerDigest={parent?.provider_generation_digest}
        cwd={parent?.cwd ?? ""}
        loading={!state.loaded}
        connected={connected}
        topInset="0px"
        bottomInset={desktop ? "0px" : MOBILE_FOOTER_CLEARANCE}
        shortContentAtTop={desktop}
        desktopNavigation={desktop}
        statusBar={false}
        magneticHaptics={false}
      />
    </Box>
  );
}
