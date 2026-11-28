import { SessionPromptCacheCard } from "./SessionPromptCacheCard";

/**
 * Session-level prompt-cache strategy, live expiry window, efficiency
 * breakdown, and turn-by-turn hit-rate sparkline.
 */
export function SessionCacheMetricsSection({
  sessionId,
  compact = false,
  refreshKey = 0,
}: {
  sessionId: string;
  compact?: boolean;
  refreshKey?: number;
}): React.JSX.Element | null {
  return (
    <SessionPromptCacheCard
      sessionId={sessionId}
      compact={compact}
      readOnly={compact}
      refreshKey={refreshKey}
    />
  );
}
