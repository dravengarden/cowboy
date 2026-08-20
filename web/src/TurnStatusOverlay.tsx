import { Box, IconButton, Stack, Typography } from "@mui/material";
import type { PaletteColor, Theme } from "@mui/material";
import type { Status } from "./protocol";
import { setPaused } from "./store";
import { frostedPill } from "./frostedGlass";
import { TURN_STATUS_PILL_MIN_HEIGHT } from "./floatingOverlayPolicy";
import {
  deriveTurnStatusKind,
  type TurnStatusKind,
} from "./turnStatusPolicy";

type PaletteKey = "warning";

const KIND_META: Record<TurnStatusKind, { color: PaletteKey; label: string }> = {
  interrupted: { color: "warning", label: "Turn interrupted" },
  paused: { color: "warning", label: "Queue paused" },
};

export function TurnStatusOverlay({
  sessionId,
  status,
  working,
  paused,
  onFocusComposer,
}: {
  sessionId: string;
  status: Status;
  working: boolean;
  paused: boolean;
  onFocusComposer: () => void;
}): React.JSX.Element | null {
  const kind = deriveTurnStatusKind({ status, working, paused });
  if (kind === null) return null;

  const meta = KIND_META[kind];
  const tone = (theme: Theme): PaletteColor => theme.palette[meta.color];
  const action = kind === "paused"
    ? { label: "Resume", onClick: () => setPaused(sessionId, false) }
    : undefined;

  return (
    <Box
      data-turn-status-overlay
      data-composer-stack-slot="status"
      sx={{
        position: "relative",
        display: "flex",
        justifyContent: "center",
        px: 2,
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        pointerEvents: "none",
        zIndex: 3,
      }}
    >
      <Stack
        role="status"
        data-turn-status-pill
        direction="row"
        alignItems="center"
        spacing={1}
        onClick={onFocusComposer}
        sx={(theme) => ({
          cursor: "text",
          maxWidth: "100%",
          userSelect: "none",
          WebkitUserSelect: "none",
          WebkitTouchCallout: "none",
          pointerEvents: "auto",
          ...(action ? { pl: 2, pr: 0.5 } : { px: 2 }),
          py: 0.5,
          minHeight: TURN_STATUS_PILL_MIN_HEIGHT,
          borderRadius: 999,
          ...frostedPill(theme, tone(theme).main),
        })}
      >
        <Typography
          variant="body2"
          sx={{
            fontWeight: 600,
            color: (theme) => tone(theme).main,
            whiteSpace: "nowrap",
          }}
        >
          {meta.label}
        </Typography>
        {action && (
          <IconButton
            size="small"
            aria-label={action.label}
            onClick={(event): void => {
              event.stopPropagation();
              action.onClick();
            }}
            sx={{
              color: (theme) => tone(theme).contrastText,
              bgcolor: (theme) => tone(theme).main,
              "&:hover": { bgcolor: (theme) => tone(theme).dark },
              px: 1.25,
              width: "auto",
              height: 28,
              borderRadius: 999,
              fontSize: 12,
              fontWeight: 600,
            }}
          >
            {action.label}
          </IconButton>
        )}
      </Stack>
    </Box>
  );
}
