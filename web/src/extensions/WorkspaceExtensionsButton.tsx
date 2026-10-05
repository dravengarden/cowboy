import { desktopSize } from "../surface/desktopSize";
import { ExtensionOutlined } from "@mui/icons-material";
import { Box, CircularProgress, IconButton } from "@mui/material";
import { lazy, Suspense, useState } from "react";
import { Sheet } from "../Sheet";
import { useSurfaceProfile } from "../surface/SurfaceProfile";

const WorkspaceExtensions = lazy(() => import("./WorkspaceExtensions"));

const DesktopExtensionsSurface = lazy(() =>
  import("./DesktopExtensionsSurface")
);

export function WorkspaceExtensionsButton(
  { context, machineId }: {
    context?: string | undefined;
    machineId?: string | undefined;
  },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const [open, setOpen] = useState(false);
  const content = open && context
    ? (
      <Box
        sx={{
          height: desktop ? "min(660px, 70vh)" : "calc(100dvh - 130px)",
          minHeight: 260,
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        }}
      >
        <Suspense fallback={<CircularProgress sx={{ m: 3 }} size={desktopSize(24)} />}>
          <WorkspaceExtensions
            key={context}
            context={context}
            machineId={machineId}
          />
        </Suspense>
      </Box>
    )
    : null;
  return (
    <>
      <IconButton
        disabled={!context}
        onClick={() => setOpen(true)}
        aria-label="Open extensions"
        title="Extensions"
        data-desktop-item="topbar-extensions"
      >
        <ExtensionOutlined />
      </IconButton>
      {desktop
        ? (context && (
          <Suspense fallback={null}>
            <DesktopExtensionsSurface
              open={open && !!context}
              onClose={() => setOpen(false)}
              onOpen={() => setOpen(true)}
            >
              {content}
            </DesktopExtensionsSurface>
          </Suspense>
        ))
        : (
          <Sheet
            open={open && !!context}
            onClose={() => setOpen(false)}
            title="Extensions"
            forceSheet
            cover
            portal
          >
            {content}
          </Sheet>
        )}
    </>
  );
}
