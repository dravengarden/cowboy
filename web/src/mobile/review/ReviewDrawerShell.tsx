import { Box } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { bindMobileSpatialDrawer } from "../../mobileSpatialDrawer";
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
    const surface = surfaceRef.current;
    if (!root || !drawerElement || !mask || !surface) return undefined;
    const binding = bindMobileSpatialDrawer({
      gestureTarget: root,
      surface,
      drawer: drawerElement,
      drawerMask: mask,
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
        "&[data-mobile-drawer-moving='true'] *": {
          animationPlayState: "paused !important",
        },
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
          bgcolor: "background.default",
          backfaceVisibility: "hidden",
          willChange: "transform, opacity",
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
          inset: 0,
          bgcolor: "background.default",
          boxShadow: open ? "-18px 0 42px rgba(0,0,0,0.16)" : "none",
          pointerEvents: "none",
          backfaceVisibility: "hidden",
          willChange: "transform",
        }}
      />
      <Box
        ref={surfaceRef}
        sx={{
          position: "absolute",
          zIndex: 1,
          inset: 0,
          overflow: "hidden",
          bgcolor: "background.default",
          backfaceVisibility: "hidden",
          transformOrigin: "left center",
          willChange: "transform",
        }}
      >
        {open && (
          <Box
            aria-label="Close worktree drawer"
            onClick={() => closeRef.current()}
            sx={{ position: "absolute", zIndex: 2, inset: 0 }}
          />
        )}
        {children}
      </Box>
    </Box>
  );
}
