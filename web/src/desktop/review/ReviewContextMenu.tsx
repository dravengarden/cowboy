import { ExpandMore } from "@mui/icons-material";
import { Box, Button, ListItemText, ListSubheader, Menu, MenuItem, Typography } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { useActiveSessionId } from "../../controlPlane";
import { useAnnouncedSessionContext } from "../../SessionContextCue";
import { useStoreSelector } from "../../store";
import { reviewLinkedSessions } from "../../mobile/review/reviewContextModel";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { REVIEW_CONTEXT_GROUPS, reviewContextOptions } from "./reviewContextOptions";
import type { DesktopReviewContext } from "./useDesktopReviewContext";

/** What Review reads (`␣VX`): the Session's default or another context.
 *  Choosing one never moves the Agent. */
export function ReviewContextMenu({
  context,
  openRequest,
  onChoose,
}: {
  context: DesktopReviewContext;
  openRequest: number;
  /** A review-directory value; the Session's own one means its default. */
  onChoose: (value: string) => void;
}): React.JSX.Element {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  // A request opens the menu once: remounting (a new context, a tab
  // switch) must not replay the last one.
  const handled = useRef(openRequest);
  useEffect(() => {
    if (openRequest === handled.current) return;
    handled.current = openRequest;
    setOpen(true);
  }, [openRequest]);
  const agentId = useActiveSessionId();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const session = sessions.find((item) => item.id === agentId);
  const linkedContext = useAnnouncedSessionContext(
    session?.id ?? "",
    !!(session?.continued_from || session?.forked_from),
  );
  const options = useMemo(() => session
    ? reviewContextOptions(
      session,
      sessions,
      machines,
      reviewLinkedSessions(session, linkedContext, sessions),
      context.agentSynced.bindings ?? [],
    )
    : [], [context.agentSynced.bindings, linkedContext, machines, session, sessions]);
  const choose = (value: string | null): void => {
    setOpen(false);
    if (session) onChoose(value ?? `session::${session.id}`);
  };
  const target = context.target;
  let group: string | undefined;
  return (
    <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, px: 1, pt: 0.5, minWidth: 0 }}>
      <Button
        ref={anchor}
        size="small"
        color="inherit"
        data-review-context
        onClick={() => setOpen(true)}
        endIcon={<ExpandMore />}
        title={target?.cwd}
        sx={{ minWidth: 0, flex: 1, justifyContent: "flex-start", textTransform: "none", overflow: "hidden" }}
      >
        <Box component="span" sx={{ display: "flex", flexDirection: "column", alignItems: "flex-start", minWidth: 0 }}>
          <Typography component="span" noWrap sx={{ fontSize: "0.78rem", fontWeight: 650, maxWidth: "100%" }}>
            {target?.label ?? "No code"}
          </Typography>
          <Typography component="span" noWrap sx={{ fontSize: "0.66rem", color: "text.secondary", maxWidth: "100%" }}>
            {context.choice ? `${target?.kind ?? "Context"} · chosen here` : target?.kind ?? context.error ?? ""}
          </Typography>
        </Box>
      </Button>
      <LeaderKeycap leaderKey="VX" />
      <Menu anchorEl={anchor.current} open={open} onClose={() => setOpen(false)}>
        {options.flatMap((option) => {
          const header = option.group !== group ? REVIEW_CONTEXT_GROUPS[option.group] : undefined;
          group = option.group;
          const key = option.value ?? "default";
          const item = (
            <MenuItem
              key={key}
              selected={option.value === (context.choice ?? null)}
              data-review-context-option={key}
              onClick={() => choose(option.value)}
            >
              <ListItemText primary={option.title} secondary={option.detail} />
            </MenuItem>
          );
          return header
            ? [<ListSubheader key={`group-${option.group}`} sx={{ lineHeight: "28px", fontSize: "0.68rem" }}>{header}</ListSubheader>, item]
            : [item];
        })}
      </Menu>
    </Box>
  );
}
