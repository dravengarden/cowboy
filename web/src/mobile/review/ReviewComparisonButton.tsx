import { ExpandMore } from "@mui/icons-material";
import { ButtonBase, Typography } from "@mui/material";

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
        flex: 1,
        minWidth: 44,
        minHeight: 44,
        gap: 0.25,
        px: 0.25,
        borderRadius: 1,
        justifyContent: "flex-end",
        textAlign: "left",
      }}
    >
      <Typography
        variant="caption"
        noWrap
        sx={{ minWidth: 0, fontWeight: 500, fontSize: "0.8125rem" }}
      >
        {reference.length ? reference.join(" · ") : scenario}
      </Typography>
      <ExpandMore sx={{ fontSize: "1rem", color: "text.secondary", flexShrink: 0 }} />
    </ButtonBase>
  );
}
