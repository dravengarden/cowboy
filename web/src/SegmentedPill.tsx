import type { ReactNode } from "react";
import type { SxProps, Theme } from "@mui/material";
import { Box, ButtonBase } from "@mui/material";
import { alpha } from "@mui/material/styles";

// A frosted-glass segmented "pill" switch — the 微信读书 / iOS segmented-control
// shape, MUI-themed: a translucent rounded track with a single filled active pill
// that SLIDES between segments. Generic + dependency-light; tapping a segment
// stops pointer propagation so it never starts the host sheet's drag.
export function SegmentedPill<T extends string>({
  value,
  options,
  onChange,
  sx,
}: {
  value: T;
  options: readonly { value: T; label: ReactNode }[];
  onChange: (v: T) => void;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const n = options.length;
  const idx = Math.max(0, options.findIndex((o) => o.value === value));
  return (
    <Box
      sx={{
        position: "relative",
        display: "inline-flex",
        p: 0.5,
        borderRadius: 999,
        // Translucent track on the frosted sheet — carries its own light blur so
        // the page still diffuses through.
        backgroundColor: (t) => alpha(t.palette.text.primary, t.palette.mode === "dark" ? 0.1 : 0.06),
        backdropFilter: "blur(16px) saturate(180%)",
        WebkitBackdropFilter: "blur(16px) saturate(180%)",
        ...sx,
      }}
    >
      {/* The sliding active pill. */}
      <Box
        aria-hidden
        sx={{
          position: "absolute",
          top: 4,
          bottom: 4,
          left: 4,
          width: `calc((100% - 8px) / ${String(n)})`,
          transform: `translateX(${String(idx * 100)}%)`,
          transition: "transform .26s cubic-bezier(0.22, 1, 0.36, 1)",
          borderRadius: 999,
          backgroundColor: (t) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.92 : 1),
          boxShadow: (t) => `0 2px 8px ${alpha(t.palette.primary.main, 0.35)}`,
        }}
      />
      {options.map((o) => {
        const active = o.value === value;
        return (
          <ButtonBase
            key={o.value}
            onClick={(): void => onChange(o.value)}
            onPointerDown={(e): void => e.stopPropagation()}
            sx={{
              position: "relative",
              zIndex: 1,
              minWidth: 80,
              px: 2,
              py: 0.5,
              borderRadius: 999,
              fontSize: 14,
              fontWeight: 600,
              color: active ? "primary.contrastText" : "text.secondary",
              transition: "color .2s",
            }}
          >
            {o.label}
          </ButtonBase>
        );
      })}
    </Box>
  );
}
