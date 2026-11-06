import { ExpandMore } from "@mui/icons-material";
import { Box, ButtonBase, Typography } from "@mui/material";

export function ReviewComparisonButton({
  label,
  disabled,
  onClick,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
}): React.JSX.Element {
  const [scenario, ...reference] = label.split(" · ");
  return (
    <ButtonBase
      data-review-compare
      aria-haspopup="dialog"
      aria-label={`Compare: ${label}`}
      title={label}
      disabled={disabled}
      onClick={onClick}
      sx={{
        flex: "0 1 auto",
        maxWidth: "100%",
        minWidth: 44,
        minHeight: 44,
        borderRadius: "0.5rem",
        justifyContent: "stretch",
        textAlign: "left",
        "&:focus-visible": { outline: "2px solid", outlineColor: "primary.main" },
      }}
    >
      <Box
        component="span"
        data-review-comparison-surface
        sx={{
          display: "flex",
          alignItems: "center",
          gap: 0.25,
          width: "100%",
          minWidth: 0,
          minHeight: "2rem",
          pl: 0.75,
          pr: 0.5,
          py: 0.25,
          bgcolor: "action.hover",
          borderRadius: "0.5rem",
        }}
      >
        <Typography
          component="span"
          variant="caption"
          noWrap
          sx={{ flex: 1, minWidth: 0, fontWeight: 500, fontSize: "0.8125rem" }}
        >
          {reference.length ? reference.join(" · ") : scenario}
        </Typography>
        <ExpandMore sx={{ fontSize: "1rem", color: "text.secondary", flexShrink: 0 }} />
      </Box>
    </ButtonBase>
  );
}
