import { UnfoldMore } from "@mui/icons-material";
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
      disabled={disabled}
      onClick={onClick}
      sx={{
        flex: 1,
        minWidth: 0,
        minHeight: 44,
        gap: 0.5,
        borderRadius: 1,
        justifyContent: "flex-start",
        textAlign: "left",
      }}
    >
      <Box sx={{ minWidth: 0, flex: 1 }}>
        <Typography
          variant={reference.length ? "caption" : "body2"}
          noWrap
          color={reference.length ? "text.secondary" : "text.primary"}
        >
          {scenario}
        </Typography>
        {reference.length > 0 && (
          <Typography variant="body2" fontWeight={600} noWrap>
            {reference.join(" · ")}
          </Typography>
        )}
      </Box>
      <UnfoldMore fontSize="small" sx={{ color: "text.secondary", flexShrink: 0 }} />
    </ButtonBase>
  );
}
