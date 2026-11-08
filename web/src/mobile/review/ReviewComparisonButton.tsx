import { ExpandMore } from "@mui/icons-material";
import { Button, Typography } from "@mui/material";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

export function ReviewComparisonButton({
  label,
  disabled,
  onClick,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
}): React.JSX.Element {
  const mobile = useSurfaceProfile().kind !== "desktop";
  const [scenario, ...reference] = label.split(" · ");
  return (
    <Button
      data-review-compare
      variant={mobile ? "contained" : "outlined"}
      disableElevation
      color="inherit"
      aria-haspopup="dialog"
      aria-label={`Compare: ${label}`}
      title={label}
      disabled={disabled}
      onClick={onClick}
      endIcon={<ExpandMore />}
      sx={{
        flex: "0 1 auto",
        maxWidth: "100%",
        minHeight: 48,
        minWidth: 64,
        px: 0.75,
        borderColor: "divider",
        color: "text.secondary",
        fontSize: "0.8125rem",
        textTransform: "none",
        ...(mobile
          ? {
              borderRadius: "999px",
              bgcolor: "action.hover",
              "&:hover": { bgcolor: "action.hover" },
            }
          : {}),
      }}
    >
      <Typography
        component="span"
        variant="caption"
        noWrap
        sx={{ minWidth: 0, fontWeight: 500, fontSize: "inherit" }}
      >
        {reference.length ? reference.join(" · ") : scenario}
      </Typography>
    </Button>
  );
}
