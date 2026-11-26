import { Typography } from "@mui/material";
import { useStoreSelector } from "./store";

/** Discloses how Clear interacts with existing child branches and inherited
 *  ancestor history. Uses only live store metadata (zero extra fetches). */
export function ClearBranchNotice({ sessionId }: { sessionId: string }): React.JSX.Element | null {
  const { childCount, isChild } = useStoreSelector((state) => {
    const self = state.sessions.find((entry) => entry.id === sessionId);
    let children = 0;
    for (const entry of state.sessions) {
      if (entry.forked_from === sessionId || entry.continued_from === sessionId) {
        children += 1;
      }
    }
    return {
      childCount: children,
      isChild: Boolean(self?.forked_from || self?.continued_from),
    };
  }, (a, b) => a.childCount === b.childCount && a.isChild === b.isChild);

  if (childCount === 0 && !isChild) return null;
  return (
    <Typography
      variant="caption"
      color="text.secondary"
      data-clear-branch-notice
      sx={{ display: "block", mt: 1 }}
    >
      {childCount > 0 &&
        `Existing ${childCount === 1 ? "fork or continuation" : `${childCount} forks/continuations`} will keep their inherited copy of this conversation.`}
      {childCount > 0 && isChild ? " " : ""}
      {isChild && "Clearing also detaches the inherited conversation shown above this session."}
    </Typography>
  );
}
