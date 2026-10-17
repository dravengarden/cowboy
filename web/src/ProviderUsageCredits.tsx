import { Stack, Typography } from "@mui/material";
import type { UsageCreditRow } from "./usageCreditDetails";

export function ProviderUsageCredits(
  { rows }: { rows: UsageCreditRow[] | undefined },
): React.JSX.Element {
  return (
    <>
      {rows?.map((row, index) => (
        <Stack
          key={`${row.label}-${index}`}
          direction="row"
          spacing={2}
          sx={{ justifyContent: "space-between", alignItems: "baseline" }}
        >
          <Typography
            variant="body2"
            color="text.secondary"
            sx={{ minWidth: 0 }}
          >
            {row.label}
          </Typography>
          <Typography
            variant="body2"
            sx={{
              minWidth: 0,
              textAlign: "right",
              fontVariantNumeric: "tabular-nums",
              overflowWrap: "anywhere",
            }}
          >
            {row.value}
          </Typography>
        </Stack>
      ))}
    </>
  );
}
