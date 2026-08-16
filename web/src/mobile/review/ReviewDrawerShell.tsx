import { Box } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { mobileSpatialDrawerShadow } from "../../mobileDrawerDepth";
import { bindMobileSpatialDrawer } from "../../mobileSpatialDrawer";
import { mobilePresentationMovingRootSx } from "../../mobilePresentationMotion";
import { holdStorePresentation } from "../../store";

export function ReviewDrawerShell({
  drawer,
  children,
  onOpenChange,
  closeRequest = 0,
  toggleRequest = 0,
}: {
  drawer: React.ReactNode;
  children: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  closeRequest?: number;
  toggleRequest?: number;
}): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null);
  const drawerRef = useRef<HTMLDivElement>(null);
  const maskRef = useRef<HTMLDivElement>(null);
  const dimRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<() => void>(() => undefined);
  const toggleRef = useRef<() => void>(() => undefined);
  const openRef = useRef(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (closeRequest > 0) closeRef.current();
  }, [closeRequest]);

  useEffect(() => {
    if (toggleRequest > 0) toggleRef.current();
  }, [toggleRequest]);

  useEffect(() => {
    const root = rootRef.current;
    const drawerElement = drawerRef.current;
    const mask = maskRef.current;
    const dim = dimRef.current;
    const surface = surfaceRef.current;
    if (!root || !drawerElement || !mask || !dim || !surface) return undefined;
    const binding = bindMobileSpatialDrawer({
      gestureTarget: root,
      surface,
      drawer: drawerElement,
      drawerMask: mask,
      dim,
      side: "right",
      phone: root.clientWidth < 768,
      getOpen: () => openRef.current,
      setOpen: (nextOpen) => {
        openRef.current = nextOpen;
        setOpen(nextOpen);
        onOpenChange(nextOpen);
      },
      holdPresentation: holdStorePresentation,
    });
    closeRef.current = () => binding.settle(false);
    toggleRef.current = () => binding.settle(!openRef.current);
    return () => {
      binding.dispose();
      closeRef.current = () => undefined;
      toggleRef.current = () => undefined;
    };
  }, [onOpenChange]);

  return (
    <Box
      ref={rootRef}
      data-mobile-drawer-presented={open ? "true" : undefined}
      sx={{
        position: "relative",
        width: 1,
        height: 1,
        overflow: "hidden",
        bgcolor: (theme) =>
          theme.palette.mode === "dark"
            ? theme.palette.grey[900]
            : theme.palette.grey[200],
        ...mobilePresentationMovingRootSx("data-mobile-drawer-moving"),
      }}
    >
      <Box
        ref={drawerRef}
        aria-hidden={!open}
        sx={{
          position: "absolute",
          zIndex: 0,
          inset: 0,
          pl: "calc(100% - min(84%, 360px))",
          bgcolor: "background.paper",
          backfaceVisibility: "hidden",
          "@media (min-width: 768px)": {
            pl: "calc(100% - min(52%, 440px))",
          },
        }}
      >
        {drawer}
      </Box>
      <Box
        ref={maskRef}
        aria-hidden="true"
        sx={{
          position: "absolute",
          zIndex: 0,
          top: 0,
          bottom: 0,
          right: 0,
          width: 28,
          bgcolor: (theme) =>
            theme.palette.mode === "dark"
              ? theme.palette.grey[900]
              : theme.palette.grey[200],
          boxShadow: open ? mobileSpatialDrawerShadow("right") : "none",
          pointerEvents: "none",
          backfaceVisibility: "hidden",
          overflow: "visible",
        }}
      />
      <Box
        ref={surfaceRef}
        data-mobile-drawer-surface="true"
        sx={{
          position: "absolute",
          zIndex: 1,
          inset: 0,
          overflow: "hidden",
          bgcolor: "background.paper",
          backfaceVisibility: "hidden",
          transformOrigin: "right center",
        }}
      >
        <Box
          ref={dimRef}
          aria-label="Close worktree drawer"
          onClick={() => {
            if (open) closeRef.current();
          }}
          sx={{
            position: "absolute",
            zIndex: 2,
            inset: 0,
            bgcolor: "common.black",
            opacity: 0,
            pointerEvents: open ? "auto" : "none",
          }}
        />
        {children}
      </Box>
    </Box>
  );
}
