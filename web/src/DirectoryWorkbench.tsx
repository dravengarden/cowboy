import type { ReactNode } from "react";
import { Box } from "@mui/material";
import { DesktopModal } from "./desktop/DesktopModal";
import { Sheet } from "./Sheet";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

/** One level-changing workbench, with a fixed action rail and one inner scroller. */
export function DirectoryWorkbench({ title, onClose, actions, children }: {
  title: string;
  onClose: () => void;
  actions: ReactNode;
  children: ReactNode;
}): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  if (desktop) {
    return (
      <DesktopModal
        open
        title={title}
        onClose={onClose}
        width={640}
        footer={
          <Box
            sx={{
              px: 2,
              py: 1.25,
              display: "flex",
              justifyContent: "flex-end",
            }}
          >
            {actions}
          </Box>
        }
      >
        <Box sx={{ p: 2 }}>{children}</Box>
      </DesktopModal>
    );
  }
  return (
    <Sheet
      open
      cover
      fill
      forceSheet
      onClose={onClose}
      title={title}
      mobileDismiss="none"
      floatingActions={false}
      actions={actions}
    >
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          px: 2,
          pb: 1,
        }}
      >
        {children}
      </Box>
    </Sheet>
  );
}
