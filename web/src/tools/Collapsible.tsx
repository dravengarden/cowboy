import { type ReactNode, useEffect, useRef, useState } from "react";
import { alpha, Box, ButtonBase, IconButton, useTheme } from "@mui/material";
import ExpandLessRounded from "@mui/icons-material/ExpandLessRounded";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";

// A reusable fold for any potentially-long tool content (command output, a read
// file, a diff, raw JSON). It clips to `maxHeight` when collapsed, with a bottom
// fade into the card surface so it reads as "there's more below", and a single
// edge toggle. The toggle + fade only appear when the content actually overflows
// (measured) — short output stays plain with no chrome. This is the "不拥挤"
// mechanism: every block defaults compact, the user opens only what they want.

export function Collapsible({
  children,
  collapsedChildren,
  forceOverflow = false,
  maxHeight = 280,
  // The surface the fade blends into — the tool body is `background.paper`, so
  // the gradient ends opaque on exactly that colour (no visible seam).
  startOpen = false,
}: {
  children: ReactNode;
  collapsedChildren?: ReactNode;
  forceOverflow?: boolean;
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
    const check = (): void => setOverflows(forceOverflow || el.scrollHeight > maxHeight + 4);
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [forceOverflow, maxHeight]);

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

  const expandButton = (): React.JSX.Element => (
    <ButtonBase
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
    <Box ref={rootRef} sx={{ position: "relative" }}>
      {overflows && open && (
        // Keep collapse reachable without laying a large labelled pill over the
        // code. The target stays 44px; only quiet circular chrome is visible.
        <Box
          sx={{
            position: "sticky",
            top: 6,
            zIndex: 3,
            height: 0,
            display: "flex",
            justifyContent: "flex-end",
            pointerEvents: "none",
            pr: 0.5,
          }}
        >
          <IconButton
            aria-label="Collapse content"
            aria-expanded={true}
            onClick={toggle}
            sx={{
              pointerEvents: "auto",
              width: 44,
              height: 44,
              color: "text.secondary",
              backgroundColor: "transparent",
              "&::before": {
                content: '""',
                position: "absolute",
                inset: 6,
                borderRadius: "50%",
                backgroundColor: alpha(paper, 0.88),
                backdropFilter: "blur(10px)",
                border: `1px solid ${alpha(theme.palette.text.secondary, 0.14)}`,
                boxShadow: `0 1px 4px ${alpha(theme.palette.common.black, 0.09)}`,
              },
              "&:hover": { backgroundColor: "transparent", color: "primary.main" },
            }}
          >
            <ExpandLessRounded sx={{ position: "relative", fontSize: "1.125rem" }} />
          </IconButton>
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
