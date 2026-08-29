import { Visibility, VisibilityOff } from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Divider,
  IconButton,
  InputAdornment,
  Stack,
  Tab,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { PasswordStrength } from "../admin/PasswordStrength";
import { assessAdminPassword } from "../admin/passwordStrength";
import {
  authApi,
  AuthApiError,
  type AuthStatus,
  type ProductMe,
  type ProductOidcProvider,
} from "./authApi";
import { nativeOidcFlowSupported, runNativeOidc } from "./nativeOidcFlow";

export function ProductLoginPage({
  setupRequired,
  setupPending,
  providers,
  passwordEnabled,
  onAuthed,
  onStatus,
}: {
  setupRequired: boolean;
  setupPending: boolean;
  providers: ProductOidcProvider[];
  passwordEnabled: boolean;
  onAuthed: (me: ProductMe) => void;
  onStatus?: (status: AuthStatus) => void;
}): React.JSX.Element {
  const creating = setupRequired && setupPending;
  const needsCode = setupRequired && !setupPending;
  const [account, setAccount] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [setupToken, setSetupToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [passwordVisible, setPasswordVisible] = useState(false);
  const providerAbort = useRef<AbortController | null>(null);
  const [method, setMethod] = useState(() =>
    passwordEnabled ? "password" : providers[0]?.id ?? ""
  );
  useEffect(() => {
    if (creating) setPasswordVisible(true);
  }, [creating]);
  useEffect(() => () => {
    const pending = providerAbort.current;
    providerAbort.current = null;
    pending?.abort();
  }, []);
  useEffect(() => {
    if (setupRequired) return;
    const available = passwordEnabled && method === "password" ||
      providers.some((provider) => provider.id === method);
    if (!available) {
      setMethod(passwordEnabled ? "password" : providers[0]?.id ?? "");
    }
  }, [method, passwordEnabled, providers, setupRequired]);
  const passwordScore = assessAdminPassword(password, account);
  const canCreate = account.trim() !== "" && passwordScore.acceptable &&
    password === confirm;
  const confirmMismatch = confirm.length > 0 && password !== confirm;
  const canLogin = account.trim() !== "" && password !== "";
  const loginMethods = [
    ...(passwordEnabled ? [{ id: "password", label: "Password" }] : []),
    ...providers.map((provider) => ({
      id: provider.id,
      label: provider.display_name,
    })),
  ];
  const selectedProvider = providers.find((provider) => provider.id === method);
  const useNativeProviderFlow = selectedProvider !== undefined &&
    nativeOidcFlowSupported();

  const submit = (): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    const request = needsCode
      ? authApi.setup(setupToken.trim()).then((status) => {
        onStatus?.(status);
      })
      : creating
      ? authApi.register(account, password).then(onAuthed)
      : authApi.login(account, password).then(onAuthed);
    void request
      .catch((err: unknown) => {
        setError(
          err instanceof AuthApiError ? err.message : "Could not reach Cowboy",
        );
      })
      .finally(() => setBusy(false));
  };

  const submitProvider = (): void => {
    if (busy || !selectedProvider || !useNativeProviderFlow) return;
    const abort = new AbortController();
    providerAbort.current = abort;
    setBusy(true);
    setError(null);
    void runNativeOidc(selectedProvider, abort.signal)
      .then(onAuthed)
      .catch((err: unknown) => {
        if (err instanceof DOMException && err.name === "AbortError") return;
        setError(
          err instanceof AuthApiError
            ? err.message
            : "Could not complete external sign-in",
        );
      })
      .finally(() => {
        if (providerAbort.current === abort) {
          providerAbort.current = null;
          setBusy(false);
        }
      });
  };

  return (
    <Box
      component="form"
      method="post"
      action="#"
      autoComplete="on"
      onSubmit={(event) => {
        event.preventDefault();
        if (needsCode && setupToken.trim() === "") return;
        if (creating && !canCreate) return;
        if (!needsCode && !creating && !canLogin) return;
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
            sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}
          >
            cowboy
          </Typography>
          <Typography
            component="h1"
            variant="h5"
            sx={{ fontWeight: 700, mt: 1, letterSpacing: -0.4 }}
          >
            {needsCode
              ? "Enter setup code"
              : creating
              ? "Create account"
              : "Sign in"}
          </Typography>
          <Typography color="text.secondary" sx={{ mt: 0.75 }}>
            {needsCode
              ? "This instance has no user yet. Enter the setup code from the host journal or data directory."
              : creating
              ? "Create the only user on this Cowboy instance."
              : "This instance requires a product account."}
          </Typography>
        </Box>
        {creating && (
          <>
            <Alert
              severity={passwordScore.acceptable ? "success" : "warning"}
              aria-live="polite"
            >
              {passwordScore.acceptable
                ? "Good. This password is strong enough to protect this public agent control plane."
                : "This password protects a public agent control plane. A weak password lets anyone who reaches this origin run agents on enrolled machines."}
            </Alert>
            {!passwordScore.acceptable && (
              <Alert severity="info">
                Prefer a password generated by Google Chrome or the macOS
                Passwords app. Those random secrets are accepted. Hand-chosen
                passwords need 15+ characters with uppercase, lowercase, and a
                digit.
              </Alert>
            )}
          </>
        )}
        {error && <Alert severity="error">{error}</Alert>}
        {!setupRequired && loginMethods.length > 1 && (
          <Tabs
            value={method}
            onChange={(_event, value: string) => setMethod(value)}
            variant="scrollable"
            scrollButtons="auto"
            aria-label="Sign-in method"
          >
            {loginMethods.map((loginMethod) => (
              <Tab
                key={loginMethod.id}
                value={loginMethod.id}
                label={loginMethod.label}
                disabled={busy}
              />
            ))}
          </Tabs>
        )}
        {!setupRequired && selectedProvider && (
          <Button
            type="button"
            href={useNativeProviderFlow ? undefined : selectedProvider.start_url}
            onClick={useNativeProviderFlow ? submitProvider : undefined}
            variant="contained"
            size="large"
            fullWidth
            disabled={useNativeProviderFlow && busy}
          >
            {useNativeProviderFlow && busy
              ? "Waiting for approval…"
              : selectedProvider.button_label}
          </Button>
        )}
        {!setupRequired && useNativeProviderFlow && busy && (
          <Button
            type="button"
            variant="text"
            onClick={() => providerAbort.current?.abort()}
          >
            Cancel
          </Button>
        )}
        {!setupRequired && selectedProvider && (
          <Divider>secure redirect</Divider>
        )}
        {needsCode
          ? (
            <TextField
              label="Setup code"
              value={setupToken}
              onChange={(event) => setSetupToken(event.target.value)}
              autoComplete="one-time-code"
              fullWidth
            />
          )
          : method === "password" || setupRequired
          ? (
            <>
              <TextField
                label="Account"
                name="username"
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
                name={creating ? "new-password" : "password"}
                type={passwordVisible ? "text" : "password"}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete={creating ? "new-password" : "current-password"}
                error={creating && password.length > 0 &&
                  !passwordScore.acceptable}
                fullWidth
                slotProps={{
                  input: {
                    endAdornment: creating
                      ? (
                        <InputAdornment position="end">
                          <IconButton
                            aria-label={passwordVisible
                              ? "Hide password"
                              : "Show password"}
                            edge="end"
                            onClick={() =>
                              setPasswordVisible((visible) => !visible)}
                          >
                            {passwordVisible
                              ? <VisibilityOff />
                              : <Visibility />}
                          </IconButton>
                        </InputAdornment>
                      )
                      : undefined,
                  },
                  ...(creating
                    ? {
                      htmlInput: {
                        passwordrules:
                          "minlength: 15; maxlength: 128; required: lower, upper, digit; allowed: [-];",
                      },
                    }
                    : {}),
                }}
              />
              {creating && (
                <PasswordStrength password={password} account={account} />
              )}
              {creating && (
                <TextField
                  label="Confirm password"
                  type={passwordVisible ? "text" : "password"}
                  value={confirm}
                  onChange={(event) => setConfirm(event.target.value)}
                  autoComplete="new-password"
                  error={confirmMismatch}
                  helperText={confirmMismatch
                    ? "Passwords do not match"
                    : undefined}
                  fullWidth
                />
              )}
            </>
          )
          : null}
        {(setupRequired || method === "password") && (
          <Button
            type="submit"
            variant="contained"
            size="large"
            disabled={busy ||
              (needsCode
                ? setupToken.trim() === ""
                : creating
                ? !canCreate
                : !canLogin)}
          >
            {needsCode ? "Continue" : creating ? "Create account" : "Sign in"}
          </Button>
        )}
      </Stack>
    </Box>
  );
}
