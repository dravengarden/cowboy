import { type ReactNode, useEffect, useRef, useState } from "react";
import { alpha, Box, ButtonBase, IconButton, useTheme } from "@mui/material";
import ExpandLessRounded from "@mui/icons-material/ExpandLessRounded";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";

// A reusable fold for any potentially-long tool content (command output, a read
// file, a diff, raw JSON). It clips to `maxHeight` when collapsed, with a bottom
// fade into the card surface so it reads as "there's more below", and a single
// edge toggle. Expanded controls share a sticky row above the content, so they
// never compete with copy controls or cover the first line. The toggle + fade
// only appear when the content actually overflows
// (measured) — short output stays plain with no chrome. This is the "不拥挤"
// mechanism: every block defaults compact, the user opens only what they want.

export function Collapsible({
  children,
  collapsedChildren,
  forceOverflow = false,
  maxHeight = 280,
  toolbar,
  // The surface the fade blends into — the tool body is `background.paper`, so
  // the gradient ends opaque on exactly that colour (no visible seam).
  startOpen = false,
}: {
  children: ReactNode;
  collapsedChildren?: ReactNode;
  forceOverflow?: boolean;
  maxHeight?: number;
  toolbar?: ReactNode;
  startOpen?: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const [open, setOpen] = useState(startOpen);
  const [overflows, setOverflows] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const innerRef = useRef<HTMLDivElement | null>(null);
  const toggleRef = useRef<HTMLButtonElement | null>(null);

  // Measure whether the content exceeds the clamp, so the toggle/fade only show
  // when there's genuinely hidden content. Re-measure on resize (highlighter
  // async-loads + reflows, the viewport rotates, fonts settle).
  useEffect(() => {
    const el = innerRef.current;
    if (!el) return undefined;
    const check = (): void => setOverflows(forceOverflow || el.scrollHeight > maxHeight + 4);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [forceOverflow, maxHeight]);

  const clamped = overflows && !open;
  const paper = theme.palette.background.paper;

  const toggle = (): void => {
    const keepFocus = document.activeElement === toggleRef.current;
    setOpen(!open);
    // A sticky collapse control can be activated halfway through a very long
    // block. Once the block contracts, return its beginning to the viewport so
    // the user keeps their place instead of landing below the collapsed card.
    requestAnimationFrame(() => {
      if (keepFocus) toggleRef.current?.focus({ preventScroll: true });
      if (open) rootRef.current?.scrollIntoView({ block: "nearest" });
    });
  };

  const expandButton = (): React.JSX.Element => (
    <ButtonBase
      ref={toggleRef}
      aria-expanded={false}
      onClick={toggle}
      sx={{
        minHeight: 44,
        width: "100%",
        gap: 0.375,
        color: "primary.main",
        fontSize: "0.8125rem",
        fontWeight: 650,
        whiteSpace: "nowrap",
        "&:hover": { backgroundColor: alpha(theme.palette.primary.main, 0.035) },
        "&:focus-visible": { outline: `2px solid ${theme.palette.primary.main}`, outlineOffset: 2 },
      }}
    >
      Expand
      <ExpandMoreRounded sx={{ fontSize: "1.0625rem" }} />
    </ButtonBase>
  );

  return (
    <Box ref={rootRef} sx={{ position: "relative", isolation: "isolate" }}>
      {(toolbar || (overflows && open)) && (
        <Box
          data-fold-toolbar
          sx={{
            position: overflows && open ? "sticky" : "relative",
            top: 0,
            zIndex: 3,
            display: "flex",
            alignItems: "center",
            justifyContent: "flex-end",
            flexWrap: "wrap",
            gap: 0.5,
            py: 0.25,
            minWidth: 0,
            borderRadius: 1,
            backgroundColor: paper,
          }}
        >
          {toolbar}
          {overflows && open && (
            <IconButton
              ref={toggleRef}
              aria-label="Collapse content"
              aria-expanded={true}
              onClick={toggle}
              sx={{
                flexShrink: 0,
                width: 44,
                height: 44,
                color: "text.secondary",
                "&:hover": { color: "primary.main" },
              }}
            >
              <ExpandLessRounded sx={{ fontSize: "1.125rem" }} />
            </IconButton>
          )}
        </Box>
      )}
      <Box
        ref={innerRef}
        sx={{
          maxHeight: clamped ? maxHeight : "none",
          overflow: "hidden",
        }}
      >
        {!open && collapsedChildren !== undefined ? collapsedChildren : children}
      </Box>
      {clamped && (
        // Fade only the clipped edge; the compact affordance floats inside the
        // content instead of adding a visually empty row below the code card.
        <Box
          aria-hidden
          sx={{
            position: "absolute",
            left: 0,
            right: 0,
            bottom: 0,
            height: 68,
            pointerEvents: "none",
            background: `linear-gradient(to bottom, ${alpha(paper, 0)}, ${alpha(paper, 0.92)} 68%, ${paper})`,
          }}
        />
      )}
      {overflows && !open && (
        <Box
          sx={{
            position: "absolute",
            left: 0,
            right: 0,
            bottom: 0,
            zIndex: 1,
            pointerEvents: "none",
            display: "flex",
            justifyContent: "center",
            pb: 0.5,
          }}
        >
          <Box sx={{ pointerEvents: "auto", width: "100%" }}>{expandButton()}</Box>
        </Box>
      )}
    </Box>
  );
}
