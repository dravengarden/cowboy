// ACP 0.101 sends `/compact` on its own shortly before an idle session's
// one-hour prompt cache expires. Native Claude Code arms that only for attended
// interactive sessions, never for SDK ones. In Cowboy it would be a paid model
// request and a lossy rewrite that no Cowboy turn, queue or quota decision
// owns, repeated for every large session left open. Keep the native SDK
// behavior: every model request starts from a Cowboy turn. The CLI's own
// automatic compaction at the context limit is unchanged.
export const IDLE_COMPACTION_POLICY = "Cowboy turn ownership";

export function disableIdleCompaction(IdleCompaction) {
  const disabledBy = IdleCompaction?.prototype?.disabledBy;
  if (typeof disabledBy !== "function") {
    throw new Error("The pinned Claude adapter changed its idle compaction gate");
  }
  IdleCompaction.prototype.disabledBy = function (...args) {
    return disabledBy.apply(this, args) ?? IDLE_COMPACTION_POLICY;
  };
}
