import { Alert, Box, Button, Stack, TextField, Typography } from "@mui/material";
import { useState } from "react";
import { AuthApiError, authApi, type ProductMe, type RegistrationPublicStatus } from "./authApi";
import { showRegistration, showRegistrationToken } from "./authStatus";

export function ProductLoginPage({
  registration,
  onAuthed,
}: {
  registration: RegistrationPublicStatus;
  onAuthed: (me: ProductMe) => void;
}): React.JSX.Element {
  const canRegister = showRegistration(registration);
  const needsToken = showRegistrationToken(registration);
  const [mode, setMode] = useState<"login" | "register">("login");
  const registering = canRegister && mode === "register";
  const [account, setAccount] = useState("");
  const [password, setPassword] = useState("");
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const canSubmit = account.trim() !== "" && password !== "" &&
    !(registering && needsToken && token.trim() === "");

  const submit = (): void => {
    if (busy || !canSubmit) return;
    setBusy(true);
    setError(null);
    const request = registering
      ? authApi.register(account, password, needsToken ? token : undefined)
      : authApi.login(account, password);
    void request
      .then(onAuthed)
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not reach Cowboy");
      })
      .finally(() => setBusy(false));
  };

  return (
    <Box
      component="form"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
      sx={{
        minHeight: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        px: 3,
        py: 6,
        bgcolor: "background.default",
        color: "text.primary",
      }}
    >
      <Stack spacing={3} sx={{ width: "100%", maxWidth: 360 }}>
        <Box>
          <Typography
            component="p"
            sx={{
              fontSize: 14,
              letterSpacing: "0.06em",
              opacity: 0.75,
            }}
          >
            cowboy
          </Typography>
          <Typography
            component="h1"
            variant="h5"
            sx={{ fontWeight: 700, mt: 1, letterSpacing: -0.4 }}
          >
            {registering ? "Create account" : "Sign in"}
          </Typography>
          <Typography color="text.secondary" sx={{ mt: 0.75 }}>
            {registering
              ? needsToken
                ? "This instance accepts invite tokens."
                : "Create a product account on this Cowboy instance."
              : "This instance requires a product account."}
          </Typography>
        </Box>
        {error && <Alert severity="error">{error}</Alert>}
        <TextField
          label="Account"
          value={account}
          onChange={(event) => setAccount(event.target.value)}
          autoComplete="username"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          fullWidth
        />
        <TextField
          label="Password"
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete={registering ? "new-password" : "current-password"}
          fullWidth
        />
        {registering && needsToken && (
          <TextField
            label="Invite token"
            value={token}
            onChange={(event) => setToken(event.target.value)}
            autoComplete="one-time-code"
            fullWidth
          />
        )}
        <Button
          type="submit"
          variant="contained"
          size="large"
          disabled={busy || !canSubmit}
        >
          {registering ? "Create account" : "Sign in"}
        </Button>
        {canRegister && (
          <Button
            type="button"
            color="inherit"
            onClick={() => {
              setMode(registering ? "login" : "register");
              setError(null);
            }}
          >
            {registering ? "Already have an account? Sign in" : "Create an account"}
          </Button>
        )}
      </Stack>
    </Box>
  );
}
