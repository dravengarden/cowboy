import { KeyboardArrowDown } from "@mui/icons-material";
import { Box, ButtonBase, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

export function ReviewContextButton({
  label,
  title,
  subtitle,
  statusColor,
  onOpen,
  branch,
  changes,
}: {
  label: string;
  title: string;
  subtitle: ReactNode;
  statusColor: string;
  onOpen: () => void;
  branch?: string | undefined;
  changes?: number | undefined;
}): React.JSX.Element {
  const mobile = useSurfaceProfile().kind !== "desktop";
  return (
    <Box
      sx={{
        flex: 1,
        minWidth: 0,
        ...(mobile ? { borderRadius: "8px", bgcolor: "background.paper" } : {}),
      }}
    >
      <ButtonBase
        data-review-context-button
        aria-label={label}
        onClick={onOpen}
        sx={{
          width: "100%",
          minWidth: 0,
          minHeight: 44,
          flexDirection: "column",
          alignItems: "stretch",
          textAlign: "left",
          ...(mobile ? { px: 1, py: 0.5, borderRadius: "8px" } : {}),
        }}
      >
        <Typography variant="body2" noWrap sx={{ fontSize: "0.875rem", fontWeight: 500 }}>
          {title}
        </Typography>
        <Box
          sx={{
            display: "flex",
            alignItems: "center",
            gap: 0.625,
            minWidth: 0,
            color: "text.secondary",
          }}
        >
          <Box
            aria-hidden
            sx={{
              width: 6,
              height: 6,
              flex: "0 0 auto",
              borderRadius: "50%",
              bgcolor: statusColor,
            }}
          />
          <Typography variant="caption" noWrap>
            {subtitle}
          </Typography>
          <KeyboardArrowDown aria-hidden sx={{ fontSize: "0.9rem", flex: "0 0 auto" }} />
        </Box>
        {mobile && branch && (
          <Typography
            data-review-context-branch
            variant="caption"
            noWrap
            title={branch}
            sx={{ color: "text.secondary", fontSize: "0.75rem", lineHeight: 1.4 }}
          >
            {branch}
            {changes ? ` · ${changes} uncommitted` : ""}
          </Typography>
        )}
      </ButtonBase>
    </Box>
  );
}
