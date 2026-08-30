import LogoutRounded from "@mui/icons-material/LogoutRounded";
import { Box, Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useProductAuth } from "./ProductAuthGate";

/** Minimal product sign-out. Lives in auth/ so Desktop/Mobile can call it
 *  without importing store.ts. */
export function ProductAccountMenu(): React.JSX.Element {
  const { me, signOut } = useProductAuth();
  const [busy, setBusy] = useState(false);
  if (me.auth_enabled === false) return <></>;
  return (
    <Box
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 3,
        p: 2,
      }}
    >
      <Stack spacing={1.5}>
        <Box>
          <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
            Sign out on this device
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.4 }}>
            This removes the Cowboy login from this device. Running agents keep
            working in the background.
          </Typography>
        </Box>
        <Button
          color="error"
          variant="outlined"
          size="large"
          fullWidth
          startIcon={<LogoutRounded />}
          disabled={busy}
          onClick={() => {
            if (busy) return;
            setBusy(true);
            void signOut().finally(() => setBusy(false));
          }}
        >
          {busy ? "Signing out…" : `Sign out ${me.account}`}
        </Button>
      </Stack>
    </Box>
  );
}
