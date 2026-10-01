import { Button, Collapse, Stack, Typography } from "@mui/material";
import { useState } from "react";

/** Browser identity is separate from the credential being replaced in Cowboy. */
export function ProviderAccountSwitch({ onCopy, disabled, requiresCode }: {
  onCopy: () => void;
  disabled: boolean;
  requiresCode: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <Stack spacing={1}>
      <Button
        disabled={disabled}
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        Use another account
      </Button>
      <Collapse in={expanded}>
        <Stack
          spacing={1.5}
          sx={{ p: 1.5, bgcolor: "action.hover", borderRadius: 2 }}
        >
          <Typography variant="body2">
            The sign-in page may remember your previous account. Use a private
            browser tab to choose a different one.
          </Typography>
          <Typography component="ol" variant="body2" sx={{ m: 0, pl: 2.5 }}>
            <li>Copy the sign-in link below.</li>
            <li>
              Open Safari Private Browsing or an Incognito window, then paste
              the link into its address bar.
            </li>
            <li>
              Sign in with the account you want to use, then return to
              Cowboy{requiresCode
                ? " and enter the authorization code below"
                : " to finish"}.
            </li>
          </Typography>
          <Button variant="outlined" disabled={disabled} onClick={onCopy}>
            Copy sign-in link
          </Button>
          <Typography variant="caption" color="text.secondary">
            Your existing Cowboy connection stays in place until the new sign-in
            succeeds. Keep this link private; if it expires, start sign-in
            again.
          </Typography>
        </Stack>
      </Collapse>
    </Stack>
  );
}
