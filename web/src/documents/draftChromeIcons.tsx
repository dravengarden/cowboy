import { SvgIcon, type SvgIconProps } from "@mui/material";
import type { ReactNode } from "react";

// The mobile Draft page follows Obsidian, whose chrome uses Lucide's thin
// stroke icons (ISC license). Material's filled glyphs read heavy beside that
// writing surface, so the handful the page chrome needs are drawn here in
// Lucide's 24px grid instead of adding an icon dependency.
function strokeIcon(name: string, paths: ReactNode) {
  function Icon(props: SvgIconProps): React.JSX.Element {
    return (
      <SvgIcon
        {...props}
        sx={[
          {
            fill: "none",
            stroke: "currentColor",
            strokeWidth: 1.75,
            strokeLinecap: "round",
            strokeLinejoin: "round",
          },
          ...(Array.isArray(props.sx) ? props.sx : [props.sx]),
        ]}
      >
        {paths}
      </SvgIcon>
    );
  }
  Icon.displayName = name;
  return Icon;
}

export const PanelLeftIcon = strokeIcon(
  "PanelLeftIcon",
  <>
    <rect width="18" height="18" x="3" y="3" rx="2" />
    <path d="M9 3v18" />
  </>,
);

export const HistoryIcon = strokeIcon(
  "HistoryIcon",
  <>
    <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
    <path d="M3 3v5h5" />
    <path d="M12 7v5l4 2" />
  </>,
);

export const DownloadIcon = strokeIcon(
  "DownloadIcon",
  <>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <path d="m7 10 5 5 5-5" />
    <path d="M12 15V3" />
  </>,
);

export const EllipsisIcon = strokeIcon(
  "EllipsisIcon",
  <>
    <circle cx="12" cy="12" r="1" />
    <circle cx="19" cy="12" r="1" />
    <circle cx="5" cy="12" r="1" />
  </>,
);

export const KeyboardHideIcon = strokeIcon(
  "KeyboardHideIcon",
  <>
    <rect width="20" height="12" x="2" y="3" rx="2" />
    <path d="M6 7h.01M10 7h.01M14 7h.01M18 7h.01M8 11h8" />
    <path d="m9 18 3 3 3-3" />
  </>,
);
