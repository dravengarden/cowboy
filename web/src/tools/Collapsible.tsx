import { type ReactNode, useEffect, useRef, useState } from "react";
import { alpha, Box, ButtonBase, useTheme } from "@mui/material";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";

// A reusable fold for any potentially-long tool content (command output, a read
// file, a diff, raw JSON). It clips to `maxHeight` when collapsed, with a bottom
// fade into the card surface so it reads as "there's more below", and a single
// pill toggle. The toggle + fade only appear when the content actually overflows
// (measured) — short output stays plain with no chrome. This is the "不拥挤"
// mechanism: every block defaults compact, the user opens only what they want.

export function Collapsible({
  children,
  maxHeight = 280,
  // The surface the fade blends into — the tool body is `background.paper`, so
  // the gradient ends opaque on exactly that colour (no visible seam).
  startOpen = false,
}: {
  children: ReactNode;
  maxHeight?: number;
  startOpen?: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const [open, setOpen] = useState(startOpen);
  const [overflows, setOverflows] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const innerRef = useRef<HTMLDivElement | null>(null);

  // Measure whether the content exceeds the clamp, so the toggle/fade only show
  // when there's genuinely hidden content. Re-measure on resize (highlighter
  // async-loads + reflows, the viewport rotates, fonts settle).
  useEffect(() => {
    const el = innerRef.current;
    if (!el) return undefined;
    const check = (): void => setOverflows(el.scrollHeight > maxHeight + 4);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [maxHeight]);

  const clamped = overflows && !open;
  const paper = theme.palette.background.paper;

  const toggle = (): void => {
    if (!open) {
      setOpen(true);
      return;
    }

    setOpen(false);
    // A sticky collapse control can be activated halfway through a very long
    // block. Once the block contracts, return its beginning to the viewport so
    // the user keeps their place instead of landing below the collapsed card.
    requestAnimationFrame(() => rootRef.current?.scrollIntoView({ block: "nearest" }));
  };

  const toggleButton = (expanded: boolean): React.JSX.Element => (
    <ButtonBase
      aria-expanded={expanded}
      onClick={toggle}
      sx={{
        minHeight: 44,
        px: 1,
        borderRadius: 999,
        gap: 0.25,
        color: "primary.main",
        fontSize: 12,
        fontWeight: 600,
        whiteSpace: "nowrap",
        "&:hover": { textDecoration: "underline" },
        "&:focus-visible": { outline: `2px solid ${theme.palette.primary.main}`, outlineOffset: 2 },
      }}
    >
      {expanded ? "Show less" : "Show more"}
      <ExpandMoreRounded
        sx={{ fontSize: 16, transform: expanded ? "rotate(180deg)" : "none", transition: "transform .15s" }}
      />
    </ButtonBase>
  );

  return (
    <Box ref={rootRef} sx={{ position: "relative" }}>
      {overflows && open && (
        // Keep collapse reachable while the surrounding Sheet/Dialog scrolls.
        // The sticky row is bounded by this block, so consecutive expanded
        // outputs hand the control off naturally without creating nested scroll.
        <Box
          sx={{
            position: "sticky",
            top: 0,
            zIndex: 2,
            height: 44,
            display: "flex",
            justifyContent: "flex-end",
            pointerEvents: "none",
          }}
        >
          <Box
            sx={{
              pointerEvents: "auto",
              borderRadius: 999,
              backgroundColor: alpha(paper, 0.92),
              backdropFilter: "blur(8px)",
              boxShadow: `0 1px 4px ${alpha(theme.palette.common.black, 0.16)}`,
            }}
          >
            {toggleButton(true)}
          </Box>
        </Box>
      )}
      <Box
        ref={innerRef}
        sx={{
          maxHeight: clamped ? maxHeight : "none",
          overflow: "hidden",
        }}
      >
        {children}
      </Box>
      {clamped && (
        // Fade the clipped edge into the card surface (transparent → paper).
        <Box
          aria-hidden
          sx={{
            position: "absolute",
            left: 0,
            right: 0,
            bottom: 0,
            height: 48,
            pointerEvents: "none",
            background: `linear-gradient(to bottom, ${alpha(paper, 0)}, ${paper})`,
          }}
        />
      )}
      {overflows && !open && <Box sx={{ mt: 0.5, display: "inline-flex" }}>{toggleButton(false)}</Box>}
    </Box>
  );
}
