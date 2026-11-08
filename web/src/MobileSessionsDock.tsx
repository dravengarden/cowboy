import { Box } from "@mui/material";
import { Add, Close as CloseIcon, InfoOutlined, Settings as SettingsIcon } from "@mui/icons-material";
import { FLOATING_CONTROLS_BOTTOM, MobileSheetActionGroup, readerBottomEdge } from "@cowboy/app-shell";
import type { SessionFoldControl } from "./SessionFoldControl";

/** The Mobile Sessions drawer's floating controls: session actions on the
 *  reading edge, app actions on the trailing edge. They share the frost and
 *  island edge of every sheet footer, so rows fade out behind them. */
export function MobileSessionsDock({ phone, fold, onNew, onClose, onOpenAbout, onOpenSettings }: {
  phone: boolean;
  fold: Pick<SessionFoldControl, "action" | "label" | "icon" | "run">;
  /** Absent when this list cannot create sessions. */
  onNew?: (() => void) | undefined;
  onClose?: (() => void) | undefined;
  onOpenAbout?: (() => void) | undefined;
  onOpenSettings?: (() => void) | undefined;
}): React.JSX.Element {
  const glyph = { fontSize: "1.25em" };
  return (
    <Box
      data-sessions-floating-controls
      sx={(theme) => ({
        ...readerBottomEdge(theme),
        position: "absolute",
        zIndex: 3,
        left: 0,
        right: 0,
        bottom: 0,
        pt: 0.5,
        pb: FLOATING_CONTROLS_BOTTOM,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        boxSizing: "border-box",
        // Each MobileSheetActionGroup defaults to width 100%, which would
        // split this row into two half-width columns instead of two islands.
        pl: 2,
        pr: phone ? 2 : 4,
        pointerEvents: "none",
        "& > [data-mobile-sheet-footer-shield]": { width: "auto", flex: "0 0 auto" },
      })}
    >
      <MobileSheetActionGroup
        actions={[
          ...(onNew ? [{ key: "new", label: "Create", onPress: onNew, icon: <Add aria-hidden sx={{ fontSize: "1.35em" }} /> }] : []),
          { key: "fold", label: fold.label, visible: fold.action !== null, onPress: fold.run, icon: fold.icon },
        ]}
      />
      <MobileSheetActionGroup
        actions={[
          {
            key: "close",
            label: "Close sessions",
            onPress: onClose ?? (() => undefined),
            icon: <CloseIcon aria-hidden sx={{ ...glyph, transform: "translate(-0.75px, -0.5px)" }} />,
          },
          ...(onOpenAbout ? [{ key: "info", label: "About", onPress: onOpenAbout, icon: <InfoOutlined aria-hidden sx={glyph} /> }] : []),
          ...(onOpenSettings
            ? [{ key: "settings", label: "Settings", onPress: onOpenSettings, icon: <SettingsIcon aria-hidden sx={glyph} /> }]
            : []),
        ]}
      />
    </Box>
  );
}
