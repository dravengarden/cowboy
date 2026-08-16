import { Box } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { mobileSpatialDrawerShadow } from "../../mobileDrawerDepth";
import { bindMobileSpatialDrawer } from "../../mobileSpatialDrawer";
import {
  mobileCompositorFlattenSx,
  mobileDrawerRailHitSx,
  mobilePresentationMovingRootSx,
} from "../../mobilePresentationMotion";
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
        display: "flex",
        flexDirection: "column",
        width: 1,
        height: 1,
        overflow: "hidden",
        bgcolor: "background.paper",
        ...mobilePresentationMovingRootSx("data-mobile-drawer-moving"),
        ...mobileDrawerRailHitSx,
        "&[data-mobile-drawer-open='true']": mobileCompositorFlattenSx,
        "&[data-mobile-drawer-presented='true']": mobileCompositorFlattenSx,
      }}
    >
      <Box
        ref={drawerRef}
        aria-hidden={!open}
        sx={{
          position: "absolute",
          zIndex: 0,
          inset: 0,
          pl: "calc(100% - var(--mobile-drawer-width, min(84%, 360px)))",
          bgcolor: "background.paper",
          backfaceVisibility: "hidden",
          isolation: "isolate",
          transform: "translate3d(0, 0, 0)",
          "@media (min-width: 768px)": {
            pl: "calc(100% - var(--mobile-drawer-width, min(52%, 440px)))",
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
          bgcolor: "background.default",
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
          // In-flow fill. A transformed position:absolute;inset:0 surface
          // lets iOS pin the Review footer to the visual viewport.
          position: "relative",
          zIndex: 1,
          flex: 1,
          minHeight: 0,
          width: "100%",
          overflow: "hidden",
          bgcolor: "background.default",
          backfaceVisibility: "hidden",
          transformOrigin: "right center",
        }}
      >
        <Box
          ref={dimRef}
          data-mobile-drawer-dim="right"
          aria-hidden
          sx={{
            position: "absolute",
            zIndex: 2,
            inset: 0,
            bgcolor: "transparent",
            backgroundImage: (t) =>
              t.palette.mode === "dark"
                ? "linear-gradient(to left, rgba(0,0,0,0.46), rgba(0,0,0,0.20) 42%, rgba(0,0,0,0.05))"
                : "linear-gradient(to left, rgba(0,0,0,0.24), rgba(0,0,0,0.10) 46%, rgba(0,0,0,0.03))",
            opacity: 0,
            pointerEvents: "none",
          }}
        />
        <Box
          data-mobile-drawer-close="right"
          aria-label="Close worktree drawer"
          onClick={() => {
            if (open) closeRef.current();
          }}
          sx={{
            position: "absolute",
            zIndex: 2,
            inset: 0,
            pointerEvents: "none",
          }}
        />
        {children}
      </Box>
    </Box>
  );
}
