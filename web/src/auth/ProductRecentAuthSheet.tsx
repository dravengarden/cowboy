import {
  Alert,
  Box,
  Button,
  Stack,
  Tab,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { ConfirmSheet } from "../Sheet";
import {
  authApi,
  AuthApiError,
  type ProductMe,
  type ProductOidcProvider,
} from "./authApi";
import { nativeOidcFlowSupported, runNativeOidc } from "./nativeOidcFlow";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  verifyPasskey,
} from "./passkeyFlow";

const PASSKEY_METHOD = "passkey";
const PASSWORD_METHOD = "password";
const PROVIDER_PREFIX = "provider:";

function providerMethod(provider: ProductOidcProvider): string {
  return `${PROVIDER_PREFIX}${provider.id}`;
}

function initialMethod(
  me: ProductMe,
  passwordEnabled: boolean,
  providers: ProductOidcProvider[],
): string {
  if ((me.passkey_count ?? 0) > 0 && passkeyFlowSupported()) {
    return PASSKEY_METHOD;
  }
  if (passwordEnabled) return PASSWORD_METHOD;
  return providers[0] ? providerMethod(providers[0]) : "";
}

export function ProductRecentAuthSheet({
  open,
  me,
  providers,
  passwordEnabled,
  onVerified,
  onCancel,
}: {
  open: boolean;
  me: ProductMe;
  providers: ProductOidcProvider[];
  passwordEnabled: boolean;
  onVerified: (me: ProductMe) => void;
  onCancel: () => void;
}): React.JSX.Element {
  const [method, setMethod] = useState(() =>
    initialMethod(me, passwordEnabled, providers)
  );
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const providerAbort = useRef<AbortController | null>(null);
  const passkeyAvailable = (me.passkey_count ?? 0) > 0 &&
    passkeyFlowSupported();
  const methods = [
    ...(passkeyAvailable ? [{ id: PASSKEY_METHOD, label: "Passkey" }] : []),
    ...(passwordEnabled ? [{ id: PASSWORD_METHOD, label: "Password" }] : []),
    ...providers.map((provider) => ({
      id: providerMethod(provider),
      label: provider.display_name,
    })),
  ];
  const selectedProvider = providers.find((provider) =>
    providerMethod(provider) === method
  );
  const useNativeProviderFlow = selectedProvider !== undefined &&
    nativeOidcFlowSupported();

  useEffect(() => () => providerAbort.current?.abort(), []);
  useEffect(() => {
    if (!open) return;
    setPassword("");
    setError(null);
    setMethod(initialMethod(me, passwordEnabled, providers));
  }, [me, open, passwordEnabled, providers]);
  useEffect(() => {
    if (methods.some((candidate) => candidate.id === method)) return;
    setMethod(initialMethod(me, passwordEnabled, providers));
  }, [me, method, methods, passwordEnabled, providers]);

  const finish = (
    request: () => Promise<ProductMe>,
    fallback: string,
  ): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void request()
      .then(onVerified)
      .catch((reason: unknown) => {
        if (passkeyFlowCancelled(reason)) return;
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : passkeyErrorMessage(reason, fallback),
        );
      })
      .finally(() => setBusy(false));
  };

  const verifyProvider = (): void => {
    if (!selectedProvider || !useNativeProviderFlow) return;
    const abort = new AbortController();
    providerAbort.current = abort;
    finish(
      () =>
        runNativeOidc(selectedProvider, abort.signal).finally(() => {
          if (providerAbort.current === abort) providerAbort.current = null;
        }),
      "Could not complete external verification",
    );
  };

  const cancel = (): void => {
    if (busy) return;
    onCancel();
  };

  return (
    <ConfirmSheet
      open={open}
      onClose={cancel}
      title="Verify it’s you"
      actions={
        <Button color="inherit" disabled={busy} onClick={cancel}>Cancel</Button>
      }
    >
      <Box
        component="form"
        onSubmit={(event) => {
          event.preventDefault();
          if (method !== PASSWORD_METHOD || password === "") return;
          finish(
            () => authApi.login(me.account, password),
            "Could not verify your password",
          );
        }}
      >
        <Stack spacing={2}>
          <Typography color="text.secondary">
            Passkey changes require a sign-in or Passkey check from the last
            five minutes. Verify now, then Cowboy will continue your pending
            change automatically.
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          {methods.length > 1 && (
            <Tabs
              value={method}
              onChange={(_event, value: string) => setMethod(value)}
              variant="scrollable"
              scrollButtons="auto"
              aria-label="Verification method"
            >
              {methods.map((candidate) => (
                <Tab
                  key={candidate.id}
                  value={candidate.id}
                  label={candidate.label}
                  disabled={busy}
                />
              ))}
            </Tabs>
          )}
          {method === PASSKEY_METHOD && (
            <Button
              type="button"
              variant="contained"
              size="large"
              disabled={busy}
              onClick={() =>
                finish(verifyPasskey, "Could not verify your Passkey")}
            >
              Verify with Passkey
            </Button>
          )}
          {method === PASSWORD_METHOD && (
            <>
              <TextField
                label="Account"
                name="username"
                value={me.account}
                autoComplete="username"
                fullWidth
                slotProps={{ htmlInput: { readOnly: true } }}
              />
              <TextField
                label="Password"
                name="password"
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="current-password"
                fullWidth
              />
              <Button
                type="submit"
                variant="contained"
                size="large"
                disabled={busy || password === ""}
              >
                Verify password
              </Button>
            </>
          )}
          {selectedProvider && (
            <>
              <Button
                type="button"
                href={useNativeProviderFlow
                  ? undefined
                  : selectedProvider.start_url}
                onClick={useNativeProviderFlow ? verifyProvider : undefined}
                variant="contained"
                size="large"
                disabled={useNativeProviderFlow && busy}
              >
                {useNativeProviderFlow && busy
                  ? "Waiting for approval…"
                  : selectedProvider.button_label}
              </Button>
              {!useNativeProviderFlow && (
                <Typography variant="body2" color="text.secondary">
                  After the secure redirect returns, repeat the Passkey change.
                </Typography>
              )}
              {useNativeProviderFlow && busy && (
                <Button
                  type="button"
                  variant="text"
                  onClick={() => providerAbort.current?.abort()}
                >
                  Cancel external verification
                </Button>
              )}
            </>
          )}
          {methods.length === 0 && (
            <Alert severity="error">
              This Cowboy Service has no verification method available.
            </Alert>
          )}
        </Stack>
      </Box>
    </ConfirmSheet>
  );
}
