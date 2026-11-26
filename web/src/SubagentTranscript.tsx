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

  // One request at a time, in order: an older response can never replace
  // newer rows, and every cursor follows the rows already merged.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const busy = useRef(0);
  // Each selection is a generation: a request queued or answered for an
  // earlier one (the same child closed and reopened included) does nothing.
  const generation = useRef(0);
  const fetchPage = useCallback(
    (direction: "sync" | "after" | "before"): Promise<void> => {
      if (!childId) return Promise.resolve();
      const own = generation.current;
      const current = (): boolean => generation.current === own && latest.current.key === key;
      busy.current += 1;
      const run = async (): Promise<void> => {
        // Chosen when it runs: an empty view starts from the newest page and
        // a shown one continues after its newest row, so no hole opens.
        const mode =
          direction === "sync"
            ? latest.current.transcript.events.length > 0
              ? "after"
              : "newest"
            : direction;
        // A forward read drains every page the Controller has, in bounded
        // runs that queue their own continuation.
        for (let page = 0; page < (mode === "after" ? 50 : 1); page += 1) {
          if (!current()) return;
          const shown = latest.current.transcript;
          const cursor =
            mode === "after"
              ? { after: afterCursor(shown) ?? 0 }
              : mode === "before"
                ? { before: shown.events[0]?.seq ?? Number.MAX_SAFE_INTEGER }
                : {};
          const result = await fetchObservedPage(sessionId, childId, cursor);
          if (!current()) return;
          const transcript = mergeObserved(latest.current.transcript, result, mode);
          remember(key, transcript);
          latest.current = { key, transcript, loaded: true };
          setState(latest.current);
          if (!result.more_after) return;
          if (page === 49) setTimeout(() => void fetchPage("after"), 0);
        }
      };
      const next = queue.current
        .then(run)
        .catch(() => {
          if (current() && !latest.current.loaded) {
            latest.current = { ...latest.current, loaded: true };
            setState(latest.current);
          }
        })
        .finally(() => {
          busy.current -= 1;
        });
      queue.current = next;
      return next;
    },
    [childId, key, sessionId],
  );

  useEffect(() => {
    if (!childId) return undefined;
    void fetchPage("sync");
    return () => {
      generation.current += 1;
    };
  }, [childId, fetchPage]);

  // Live while it works, once the first page is in; after it stops, twice
  // more for rows still on their way through the Controller's writer.
  const loaded = current.loaded;
  useEffect(() => {
    if (!childId || !loaded) return undefined;
    if (!working) {
      const timers = [600, 3000].map((delay) => setTimeout(() => void fetchPage("after"), delay));
      return () => timers.forEach(clearTimeout);
    }
    const timer = setInterval(() => {
      if (busy.current === 0) void fetchPage("after");
    }, LIVE_POLL_MS);
    return () => clearInterval(timer);
  }, [childId, working, loaded, fetchPage]);

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
        <Box sx={{ display: "flex", alignItems: "center", gap: 1, px: 2, py: 0.75 }}>
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
