import { AccountTreeOutlined, DifferenceOutlined } from "@mui/icons-material";
import { Box, ButtonBase } from "@mui/material";
import { alpha, type SxProps, type Theme } from "@mui/material/styles";
import { COARSE_POINTER_ROOT_CLASS } from "../../platform";
import type { ReviewMode } from "./reviewMode";

export function ReviewModeSwitcher({
  mode,
  onChange,
}: {
  mode: ReviewMode;
  onChange: (mode: ReviewMode) => void;
}): React.JSX.Element {
  // Paint-only selected fill. A sliding thumb with its own transform and
  // drop shadow sits inside the Review peek's compositor layer, so iOS
  // reassembles that nested tile on every swipe frame
  // (docs/mobile-spatial-presentation.md).
  const markTouchActivation = (
    event: React.PointerEvent<HTMLButtonElement>,
  ): void => {
    if (event.pointerType === "touch") {
      event.currentTarget.dataset.touchActivated = "true";
    } else if (event.pointerType === "mouse") {
      delete event.currentTarget.dataset.touchActivated;
    }
  };
  const clearMouseActivation = (
    event: React.PointerEvent<HTMLButtonElement>,
  ): void => {
    if (event.pointerType === "mouse") {
      delete event.currentTarget.dataset.touchActivated;
    }
  };
  const selectedFill = (theme: Theme): string =>
    alpha(
      theme.palette.primary.main,
      theme.palette.mode === "dark" ? 0.24 : 0.13,
    );
  const selectedChip = {
    bgcolor: selectedFill,
    color: "primary.main",
  };
  const modeButtonSx: SxProps<Theme> = {
    width: 44,
    height: 44,
    borderRadius: "16px",
    bgcolor: "transparent",
    color: "text.secondary",
    transform: "none",
    // Theme ButtonBase kills hover paint on coarse pointers
    // (`backgroundColor: transparent`). That rule is more specific than a
    // plain `[aria-pressed='true']` fill, so iOS sticky hover erased the
    // selected chip and left a stale circle on the previous mode.
    "&&[aria-pressed='true']": selectedChip,
    [`html.${COARSE_POINTER_ROOT_CLASS} &&[aria-pressed='true']`]: selectedChip,
    [`html.${COARSE_POINTER_ROOT_CLASS} &&[aria-pressed='true']:hover`]:
      selectedChip,
    [`html.${COARSE_POINTER_ROOT_CLASS} &&[aria-pressed='true'].Mui-focusVisible`]:
      selectedChip,
    "@media (hover: none), (pointer: coarse), (any-pointer: coarse)": {
      "&&[aria-pressed='true'], &&[aria-pressed='true']:hover, &&[aria-pressed='true'].Mui-focusVisible":
        selectedChip,
    },
    "&[data-touch-activated='true'][aria-pressed='false']:hover, &[data-touch-activated='true'][aria-pressed='false'].Mui-focusVisible":
      {
        bgcolor: "transparent",
        color: "text.secondary",
      },
    "&[data-touch-activated='true'][aria-pressed='true']:hover, &[data-touch-activated='true'][aria-pressed='true'].Mui-focusVisible":
      selectedChip,
    "&[data-touch-activated='true']:active": {
      bgcolor: "action.selected",
    },
  };
  return (
    <Box
      data-mobile-review-mode-switcher
      role="group"
      aria-label="Switch between Git changes and Worktree files"
      sx={{
        display: "flex",
        alignItems: "center",
        flex: "0 0 auto",
        boxSizing: "border-box",
        height: 48,
        p: "1px",
        gap: "2px",
        borderRadius: "20px",
        border: 1,
        borderColor: (theme) => alpha(theme.palette.text.primary, 0.1),
        bgcolor: (theme) => alpha(theme.palette.text.primary, 0.04),
        transform: "none",
        willChange: "auto",
        WebkitTapHighlightColor: "transparent",
        "& .MuiSvgIcon-root": {
          // Root-relative geometry keeps both mode glyphs in step with the
          // global font-size setting instead of freezing them at 17px.
          fontSize: "1.125rem",
        },
      }}
    >
      <ButtonBase
        disableRipple
        aria-label="Git changes"
        aria-pressed={mode === "git"}
        onPointerDown={markTouchActivation}
        onPointerEnter={clearMouseActivation}
        onKeyDown={(event): void => {
          delete event.currentTarget.dataset.touchActivated;
        }}
        onClick={(event): void => {
          onChange("git");
          if (event.currentTarget.dataset.touchActivated === "true") {
            event.currentTarget.blur();
          }
        }}
        sx={modeButtonSx}
      >
        <DifferenceOutlined />
      </ButtonBase>
      <ButtonBase
        disableRipple
        aria-label="Worktree files"
        aria-pressed={mode === "files"}
        onPointerDown={markTouchActivation}
        onPointerEnter={clearMouseActivation}
        onKeyDown={(event): void => {
          delete event.currentTarget.dataset.touchActivated;
        }}
        onClick={(event): void => {
          onChange("files");
          if (event.currentTarget.dataset.touchActivated === "true") {
            event.currentTarget.blur();
          }
        }}
        sx={modeButtonSx}
      >
        <AccountTreeOutlined />
      </ButtonBase>
    </Box>
  );
}
