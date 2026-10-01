import { Button, Collapse, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useReliableTouchTap } from "./useReliableTouchTap";

/** Browser identity is separate from the credential being replaced in Cowboy. */
export function ProviderAccountSwitch(
  { onCopy, onOpen, notice, disabled, requiresCode }: {
    onCopy: () => void;
    onOpen: () => void;
    notice: string;
    disabled: boolean;
    requiresCode: boolean;
  },
) {
  const [expanded, setExpanded] = useState(false);
  const [fallback, setFallback] = useState(false);
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(() => {
    if (!disabled) setExpanded(!expanded);
  });
  const openTap = useReliableTouchTap<HTMLButtonElement>(() => {
    if (!disabled) onOpen();
  });
  const fallbackTap = useReliableTouchTap<HTMLButtonElement>(() => {
    if (!disabled) setFallback(!fallback);
  });
  const copyTap = useReliableTouchTap<HTMLButtonElement>(() => {
    if (!disabled) onCopy();
  });
  return (
    <Stack spacing={1}>
      <Button
        disabled={disabled}
        aria-expanded={expanded}
        {...toggleTap}
      >
        Use another account
      </Button>
      <Collapse in={expanded}>
        <Stack
          spacing={1.5}
          sx={{ p: 1.5, bgcolor: "action.hover", borderRadius: 2 }}
        >
          <Typography variant="body2">
            If the Provider page offers Switch account or Sign out, use it to
            sign in with your other account, then continue authorization there.
            You do not need to copy a link.
          </Typography>
          <Button variant="outlined" disabled={disabled} {...openTap}>
            Return to sign-in page
          </Button>
          <Button disabled={disabled} aria-expanded={fallback} {...fallbackTap}>
            No account switch on that page?
          </Button>
          <Collapse in={fallback}>
            <Stack spacing={1.5}>
              <Typography variant="body2">
                Use a private browser tab as a fallback:
              </Typography>
              <Typography component="ol" variant="body2" sx={{ m: 0, pl: 2.5 }}>
                <li>Copy the sign-in link below.</li>
                <li>
                  Open Safari Private Browsing or an Incognito window, then
                  paste the link into its address bar.
                </li>
                <li>
                  Sign in with the account you want to use, then return to
                  Cowboy{requiresCode
                    ? " and enter the authorization code below"
                    : " to finish"}.
                </li>
              </Typography>
              <Button variant="outlined" disabled={disabled} {...copyTap}>
                {notice.startsWith("Sign-in link copied")
                  ? "Link copied — copy again"
                  : notice === "Copying sign-in link…"
                  ? "Copying…"
                  : "Copy sign-in link"}
              </Button>
            </Stack>
          </Collapse>
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
