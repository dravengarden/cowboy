import { useState } from "react";
import { alpha, Button, Snackbar, Stack } from "@mui/material";
import { DraftDestinationPicker } from "./DraftDestinationPicker";
import { moveDraft, openSession } from "./store";
import { setActiveSessionId } from "./controlPlane";
import { navigationHaptic } from "./haptic";

/** "Move draft to another session": the destination picker for the draft
 *  `moveSrcId` names, and the snackbar that offers Open and Undo after a move. */
export function DraftMoveSurfaces({
  sessionId,
  moveSrcId,
  closeMove,
}: {
  sessionId: string;
  moveSrcId: string | null;
  closeMove: () => void;
}): React.JSX.Element {
  const [moveUndo, setMoveUndo] = useState<
    { id: string; toId: string; toTitle: string } | null
  >(null);
  return (
    <>
    {moveSrcId !== null && (
      <DraftDestinationPicker
        title="Move draft to…"
        sourceId={sessionId}
        onClose={closeMove}
        onPick={(destination): void => {
          moveDraft(sessionId, moveSrcId, destination.id);
          setMoveUndo({ id: moveSrcId, toId: destination.id, toTitle: destination.title });
          closeMove();
        }}
      />
    )}
    <Snackbar
      open={moveUndo !== null}
      autoHideDuration={6000}
      onClose={(_e, reason): void => {
        // Keep it up until autohide / Undo — a stray tap (clickaway) shouldn't
        // snatch the Undo away before the user can reach it.
        if (reason !== "clickaway") setMoveUndo(null);
      }}
      anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
      // Match the app-level mobile snackbar: stay above the bottom navigation
      // and composer actions while respecting the home-indicator safe area.
      sx={{
        bottom: {
          xs: "calc(env(safe-area-inset-bottom, 0px) + 96px)",
          sm: 24,
        },
        width: { xs: "calc(100% - 16px)", sm: "auto" },
        maxWidth: { xs: "calc(100% - 16px)", sm: "100%" },
        "& .MuiSnackbarContent-root": {
          width: { xs: "100%", sm: "auto" },
          minWidth: { xs: 0, sm: 288 },
          color: "text.primary",
          bgcolor: (theme) => alpha(
            theme.palette.background.paper,
            theme.palette.mode === "dark" ? 0.96 : 0.94,
          ),
          backgroundImage: "none",
          border: 1,
          borderColor: "divider",
          boxShadow: (theme) => `0 12px 36px ${
            alpha(
              theme.palette.common.black,
              theme.palette.mode === "dark" ? 0.42 : 0.18,
            )
          }`,
          backdropFilter: "blur(24px) saturate(160%)",
          WebkitBackdropFilter: "blur(24px) saturate(160%)",
        },
      }}
      message={moveUndo ? `Moved to ${moveUndo.toTitle}` : ""}
      action={
        <Stack direction="row" spacing={0.25}>
          <Button
            color="primary"
            size="small"
            onClick={(): void => {
              if (!moveUndo) return;
              navigationHaptic();
              globalThis.dispatchEvent(
                new CustomEvent("cowboy:transcript-save-viewport"),
              );
              openSession(moveUndo.toId);
              setActiveSessionId(moveUndo.toId);
              setMoveUndo(null);
            }}
            sx={{ textTransform: "none" }}
          >
            Open
          </Button>
          <Button
            color="primary"
            size="small"
            onClick={(): void => {
              if (moveUndo) moveDraft(moveUndo.toId, moveUndo.id, sessionId);
              setMoveUndo(null);
            }}
            sx={{ textTransform: "none" }}
          >
            Undo
          </Button>
        </Stack>
      }
    />
    </>
  );
}
