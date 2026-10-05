import { type KeyboardEvent, type ReactNode, useRef } from "react";
import { Box, ButtonBase } from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import { COARSE_POINTER_ROOT_CLASS } from "./platform";
import { desktopKeyIntent } from "./desktop/commands/keyIntent";

// The one Cowboy segmented switcher: equal pill
// segments with no track, the selected one filled with `action.selected` and
// labelled in the brand colour, optional leading icons. Every view switcher
// (Create, sign-in, Repository, Settings sections, …) renders through this so
// they look and behave the same.
//
// The selected fill is paint-only on the segment itself: no sliding thumb, no
// transform and no shadow, because several hosts sit inside the Mobile peek
// compositor where a nested transformed layer reassembles tiles on every
// swipe frame (docs/mobile-spatial-presentation.md §2.1). iOS keeps a sticky
// :hover after a touch, and the theme clears hover paint on coarse pointers
// with a more specific rule, so the selected fill is repeated with doubled
// specificity for those states and a touch-activated segment blurs itself.

export interface SegmentedTabOption<T extends string> {
  readonly value: T;
  readonly label: ReactNode;
  readonly icon?: ReactNode;
  readonly disabled?: boolean;
  /** Accessible name when `label` is not plain text. */
  readonly ariaLabel?: string;
  /** Tab element id (tab semantics), e.g. for `aria-labelledby`. */
  readonly id?: string;
  /** Controlled tab panel id (tab semantics). */
  readonly controls?: string;
  readonly keyShortcuts?: string;
}

/** `roving` moves the keyboard cursor inside the tablist and keeps focus
 *  there; `activate` is a click, tap or Enter that commits to the segment. */
export type SegmentedTabChangeSource = "roving" | "activate";

export function SegmentedTabs<T extends string>({
  value,
  options,
  onChange,
  "aria-label": ariaLabel,
  semantics = "tabs",
  fullWidth = true,
  disabled = false,
  sx,
  rootProps,
  vimKeys = false,
}: {
  readonly value: T | null;
  readonly options: readonly SegmentedTabOption<T>[];
  readonly onChange: (value: T, source: SegmentedTabChangeSource) => void;
  readonly "aria-label": string;
  /** `tabs` switches views (tablist, arrow keys select); `toggle` chooses an
   *  option inside a form (group of pressed buttons). */
  readonly semantics?: "tabs" | "toggle";
  readonly fullWidth?: boolean;
  readonly disabled?: boolean;
  readonly sx?: SxProps<Theme>;
  /** Extra attributes for the root, typically `data-*` hooks. */
  readonly rootProps?: Readonly<Record<`data-${string}`, string | undefined>>;
  /** Desktop Vim grammar for a focused tablist: `h`/`l` beside the arrows and
   *  `1…9` for a direct segment, resolved from physical keys so an active CJK
   *  input source cannot turn them into marked text. Touch never sets it. */
  readonly vimKeys?: boolean;
}): React.JSX.Element {
  const buttons = useRef(new Map<T, HTMLButtonElement>());
  const tabs = semantics === "tabs";
  const enabled = options.filter((option) => !disabled && !option.disabled);
  const focusable = value !== null &&
      enabled.some((option) => option.value === value)
    ? value
    : enabled[0]?.value;
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, current: T) => {
    delete event.currentTarget.dataset.touchActivated;
    if (!tabs || enabled.length === 0) return;
    let key = event.key;
    if (vimKeys) {
      const intent = desktopKeyIntent(event.nativeEvent);
      if (intent.owner === "ime") return;
      if (intent.owner === "command" && !intent.modified) {
        key = intent.key === "h"
          ? "ArrowLeft"
          : intent.key === "l"
          ? "ArrowRight"
          : intent.key;
      }
    }
    const index = enabled.findIndex((option) => option.value === current);
    const slot = vimKeys && /^[1-9]$/.test(key)
      ? options[Number(key) - 1]
      : undefined;
    const next = slot
      ? enabled.find((option) => option.value === slot.value)
      : key === "ArrowRight"
      ? enabled[(index + 1) % enabled.length]
      : key === "ArrowLeft"
      ? enabled[(index - 1 + enabled.length) % enabled.length]
      : key === "Home"
      ? enabled[0]
      : key === "End"
      ? enabled.at(-1)
      : undefined;
    if (!next) return;
    event.preventDefault();
    buttons.current.get(next.value)?.focus();
    if (next.value !== value) onChange(next.value, "roving");
  };
  return (
    <Box
      {...rootProps}
      role={tabs ? "tablist" : "group"}
      aria-label={ariaLabel}
      data-segmented-tabs
      sx={[
        {
          display: fullWidth ? "grid" : "inline-grid",
          gridAutoFlow: "column",
          // Equal segments while they fit; a long label widens its segment
          // instead of truncating.
          gridAutoColumns: fullWidth ? "minmax(max-content, 1fr)" : "1fr",
          gap: 0.5,
          width: fullWidth ? "100%" : "fit-content",
          maxWidth: "100%",
          WebkitTapHighlightColor: "transparent",
        },
        ...(Array.isArray(sx) ? sx : [sx]),
      ]}
    >
      {options.map((option) => {
        const selected = option.value === value;
        const optionDisabled = disabled || option.disabled === true;
        return (
          <ButtonBase
            key={option.value}
            ref={(node: HTMLButtonElement | null) => {
              if (node) buttons.current.set(option.value, node);
              else buttons.current.delete(option.value);
            }}
            disableRipple
            disabled={optionDisabled}
            role={tabs ? "tab" : undefined}
            id={option.id}
            aria-label={option.ariaLabel}
            aria-selected={tabs ? selected : undefined}
            aria-pressed={tabs ? undefined : selected}
            aria-controls={option.controls}
            aria-keyshortcuts={option.keyShortcuts}
            tabIndex={tabs ? (option.value === focusable ? 0 : -1) : undefined}
            data-selected={selected ? "true" : "false"}
            data-segmented-tab={option.value}
            onPointerDown={(event) => {
              // The host may be a draggable sheet; a segment tap never starts
              // its drag.
              event.stopPropagation();
              if (event.pointerType === "touch") {
                event.currentTarget.dataset.touchActivated = "true";
              } else if (event.pointerType === "mouse") {
                delete event.currentTarget.dataset.touchActivated;
              }
            }}
            onPointerEnter={(event) => {
              if (event.pointerType === "mouse") {
                delete event.currentTarget.dataset.touchActivated;
              }
            }}
            onKeyDown={(event) => onKeyDown(event, option.value)}
            onClick={(event) => {
              if (!selected) onChange(option.value, "activate");
              if (event.currentTarget.dataset.touchActivated === "true") {
                event.currentTarget.blur();
              }
            }}
            sx={segmentSx}
          >
            {option.icon && (
              <Box component="span" aria-hidden className="segmented-tab-icon">
                {option.icon}
              </Box>
            )}
            <Box component="span" className="segmented-tab-label">
              {option.label}
            </Box>
          </ButtonBase>
        );
      })}
    </Box>
  );
}

const selectedSegment = {
  bgcolor: "action.selected",
  color: "primary.main",
  fontWeight: 700,
};

const segmentSx: SxProps<Theme> = {
  minWidth: 0,
  minHeight: 40,
  px: 1.25,
  gap: 0.75,
  borderRadius: 999,
  bgcolor: "transparent",
  color: "text.secondary",
  fontFamily: "inherit",
  fontSize: "0.875rem",
  fontWeight: 500,
  lineHeight: 1.25,
  letterSpacing: "0.01em",
  textTransform: "none",
  transform: "none",
  transition: "background-color 160ms ease, color 160ms ease",
  "@media (prefers-reduced-motion: reduce)": { transition: "none" },
  "& .segmented-tab-icon": {
    display: "inline-flex",
    flexShrink: 0,
    "& .MuiSvgIcon-root": { fontSize: "1.125rem" },
  },
  "& .segmented-tab-label": {
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  "@media (hover: hover) and (pointer: fine)": {
    minHeight: 34,
    fontSize: "0.8125rem",
    "&:hover": { bgcolor: "action.hover", color: "text.primary" },
  },
  "&.Mui-focusVisible": {
    outline: "2px solid",
    outlineColor: "primary.main",
    outlineOffset: -2,
  },
  "&.Mui-disabled": { color: "text.disabled" },
  "&&[data-selected='true']": selectedSegment,
  "&&[data-selected='true']:hover": selectedSegment,
  [`html.${COARSE_POINTER_ROOT_CLASS} &&[data-selected='true'], html.${COARSE_POINTER_ROOT_CLASS} &&[data-selected='true']:hover, html.${COARSE_POINTER_ROOT_CLASS} &&[data-selected='true'].Mui-focusVisible`]:
    selectedSegment,
  "&[data-touch-activated='true'][data-selected='false']:hover, &[data-touch-activated='true'][data-selected='false'].Mui-focusVisible":
    { bgcolor: "transparent", color: "text.secondary" },
  "&[data-touch-activated='true']:active": { bgcolor: "action.selected" },
};
