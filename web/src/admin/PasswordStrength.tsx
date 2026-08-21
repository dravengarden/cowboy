import { Box, LinearProgress, Stack, Typography } from "@mui/material";
import {
  assessAdminPassword,
  type AdminPasswordAssessment,
  type AdminPasswordLevel,
} from "./passwordStrength";

const LEVEL_COLOR: Record<AdminPasswordLevel, "inherit" | "error" | "success"> = {
  empty: "inherit",
  weak: "error",
  strong: "success",
};

const LEVEL_VALUE: Record<AdminPasswordLevel, number> = {
  empty: 0,
  weak: 28,
  strong: 100,
};

function CheckLine({ ok, label }: { ok: boolean; label: string }) {
  return (
    <Typography
      sx={{
        fontSize: 13,
        color: ok ? "success.main" : "text.secondary",
      }}
    >
      {ok ? "✓" : "•"} {label}
    </Typography>
  );
}

export function PasswordStrength({
  password,
  account,
}: {
  password: string;
  account: string;
}) {
  const assessed: AdminPasswordAssessment = assessAdminPassword(password, account);
  const color = LEVEL_COLOR[assessed.level];
  return (
    <Stack spacing={0.75} aria-live="polite">
      <Stack direction="row" justifyContent="space-between" alignItems="baseline">
        <Typography sx={{ fontSize: 13, color: "text.secondary" }}>
          Password strength
        </Typography>
        <Typography
          sx={{
            fontSize: 13,
            fontWeight: 600,
            color: assessed.level === "empty" ? "text.secondary" : `${color}.main`,
          }}
        >
          {assessed.label}
        </Typography>
      </Stack>
      <LinearProgress
        variant="determinate"
        value={LEVEL_VALUE[assessed.level]}
        color={color === "inherit" ? "inherit" : color}
        sx={{ height: 6, borderRadius: 999, bgcolor: "action.hover" }}
      />
      {assessed.checks.generated ? (
        <Typography sx={{ fontSize: 13, color: "success.main" }}>
          ✓ Looks like a Chrome or Apple generated password
        </Typography>
      ) : (
        <Box>
          <CheckLine ok={assessed.checks.length} label="At least 15 characters" />
          <CheckLine ok={assessed.checks.upper} label="An uppercase letter" />
          <CheckLine ok={assessed.checks.lower} label="A lowercase letter" />
          <CheckLine ok={assessed.checks.digit} label="A number" />
        </Box>
      )}
    </Stack>
  );
}
