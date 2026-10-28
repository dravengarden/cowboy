import { alpha, Box, CircularProgress, Tooltip } from "@mui/material";
import type { SessionMeta } from "./protocol";
import { managedChildParent } from "./managedCalls";
import { useStoreSelector } from "./store";

const byParentCache = new WeakMap<readonly SessionMeta[], Map<string, number>>();

/**
 * Running managed calls per parent session, read from the child sessions the
 * list already has (no per-row request). One pass per `sessions` snapshot, so
 * a row selects a number and an unchanged row never re-renders.
 */
export function runningCallsByParent(
  sessions: readonly SessionMeta[],
): Map<string, number> {
  const cached = byParentCache.get(sessions);
  if (cached) return cached;
  const counts = new Map<string, number>();
  for (const session of sessions) {
    if (session.status !== "busy" && session.status !== "starting") continue;
    const parent = managedChildParent(session);
    if (parent) counts.set(parent, (counts.get(parent) ?? 0) + 1);
  }
  byParentCache.set(sessions, counts);
  return counts;
}

/** A session waiting on Agents it called: a compact live pill (spinner and
 * count) beside the title, in the same tinted style as the Calls chips. */
export function SessionCallsBadge(
  { sessionId }: { sessionId: string },
): React.JSX.Element | null {
  const running = useStoreSelector((snapshot) =>
    runningCallsByParent(snapshot.sessions).get(sessionId) ?? 0
  );
  if (running === 0) return null;
  const label = `${running} managed call${running === 1 ? "" : "s"}`;
  return (
    <Tooltip
      title={`${label} running · open the session and press ␣G to watch`}
      enterDelay={400}
    >
      <Box
        component="span"
        role="status"
        aria-label={`${label} running`}
        data-session-calls={running}
        sx={(theme) => ({
          display: "inline-flex",
          alignItems: "center",
          gap: "0.35em",
          flexShrink: 0,
          height: "1.5em",
          px: "0.45em",
          // Same geometry as the outlined chips beside it; no visible edge.
          border: "1px solid transparent",
          borderRadius: 999,
          fontSize: "0.6875rem",
          fontWeight: 600,
          lineHeight: 1,
          color: "primary.main",
          bgcolor: alpha(
            theme.palette.primary.main,
            theme.palette.mode === "dark" ? 0.22 : 0.12,
          ),
        })}
      >
        <CircularProgress size="0.8em" thickness={6} color="inherit" />
        {running}
      </Box>
    </Tooltip>
  );
}
