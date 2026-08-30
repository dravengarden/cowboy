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
import { useEffect, useMemo, useRef, useState } from "react";
import { ConfirmSheet } from "../Sheet";
import {
  authApi,
  AuthApiError,
  PASSWORD_LOGIN_METHOD,
  resolveProductLoginMethodOrder,
  type ProductMe,
  type ProductOidcProvider,
} from "./authApi";
import {
  browserOidcFlowSupported,
  nativeOidcFlowSupported,
  runBrowserOidc,
  runNativeOidc,
} from "./nativeOidcFlow";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  verifyPasskey,
} from "./passkeyFlow";

const PASSKEY_METHOD = "passkey";
const PROVIDER_PREFIX = "provider:";

function providerMethod(provider: ProductOidcProvider): string {
  return `${PROVIDER_PREFIX}${provider.id}`;
}

function initialMethod(
  me: ProductMe,
  passwordEnabled: boolean,
  providers: ProductOidcProvider[],
  loginMethodOrder: string[],
): string {
  if ((me.passkey_count ?? 0) > 0 && passkeyFlowSupported()) {
    return PASSKEY_METHOD;
  }
  const first = resolveProductLoginMethodOrder(
    loginMethodOrder,
    passwordEnabled,
    providers,
  )[0];
  if (first === PASSWORD_LOGIN_METHOD) return PASSWORD_LOGIN_METHOD;
  const provider = providers.find((candidate) => candidate.id === first);
  return provider ? providerMethod(provider) : "";
}

export function ProductRecentAuthSheet({
  open,
  me,
  providers,
  passwordEnabled,
  loginMethodOrder,
  requireResumeGesture = false,
  resumeLabel = "Continue",
  onVerified,
  onCancel,
}: {
  open: boolean;
  me: ProductMe;
  providers: ProductOidcProvider[];
  passwordEnabled: boolean;
  loginMethodOrder: string[];
  requireResumeGesture?: boolean;
  resumeLabel?: string;
  onVerified: (me: ProductMe) => void;
  onCancel: () => void;
}): React.JSX.Element {
  const orderedLoginMethodIds = useMemo(
    () =>
      resolveProductLoginMethodOrder(
        loginMethodOrder,
        passwordEnabled,
        providers,
      ),
    [loginMethodOrder, passwordEnabled, providers],
  );
  const [method, setMethod] = useState(() =>
    initialMethod(me, passwordEnabled, providers, orderedLoginMethodIds)
  );
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [verifiedMe, setVerifiedMe] = useState<ProductMe | null>(null);
  const providerAbort = useRef<AbortController | null>(null);
  const passkeyAvailable = (me.passkey_count ?? 0) > 0 &&
    passkeyFlowSupported();
  const methods = [
    ...(passkeyAvailable ? [{ id: PASSKEY_METHOD, label: "Passkey" }] : []),
    ...orderedLoginMethodIds.flatMap((id) => {
      if (id === PASSWORD_LOGIN_METHOD) {
        return [{ id: PASSWORD_LOGIN_METHOD, label: "Password" }];
      }
      const provider = providers.find((candidate) => candidate.id === id);
      return provider
        ? [{ id: providerMethod(provider), label: provider.display_name }]
        : [];
    }),
  ];
  const selectedProvider = providers.find((provider) =>
    providerMethod(provider) === method
  );
  const useNativeProviderFlow = selectedProvider !== undefined &&
    nativeOidcFlowSupported();
  const useBrowserProviderFlow = selectedProvider !== undefined &&
    !useNativeProviderFlow && browserOidcFlowSupported();
  const useProviderHandoff = useNativeProviderFlow || useBrowserProviderFlow;

  useEffect(() => () => providerAbort.current?.abort(), []);
  useEffect(() => {
    if (!open) return;
    setPassword("");
    setError(null);
    setVerifiedMe(null);
    setMethod(
      initialMethod(me, passwordEnabled, providers, orderedLoginMethodIds),
    );
  }, [me, open, orderedLoginMethodIds, passwordEnabled, providers]);
  useEffect(() => {
    if (methods.some((candidate) => candidate.id === method)) return;
    setMethod(
      initialMethod(me, passwordEnabled, providers, orderedLoginMethodIds),
    );
  }, [me, method, methods, orderedLoginMethodIds, passwordEnabled, providers]);

  const finish = (
    request: () => Promise<ProductMe>,
    fallback: string,
  ): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void request()
      .then((next) => {
        if (requireResumeGesture) setVerifiedMe(next);
        else onVerified(next);
      })
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
    if (!selectedProvider || !useProviderHandoff) return;
    const abort = new AbortController();
    providerAbort.current = abort;
    finish(
      () =>
        (useNativeProviderFlow
          ? runNativeOidc(selectedProvider, abort.signal)
          : runBrowserOidc(selectedProvider, abort.signal)).finally(() => {
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
          if (method !== PASSWORD_LOGIN_METHOD || password === "") return;
          finish(
            () => authApi.login(me.account, password),
            "Could not verify your password",
          );
        }}
      >
        <Stack spacing={2}>
          <Typography color="text.secondary">
            Passkey changes require a sign-in or Passkey check from the last
            five minutes. Verify now, then {requireResumeGesture
              ? "tap Continue once so Safari can open the Passkey prompt."
              : "Cowboy will continue your pending change automatically."}
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          {verifiedMe && (
            <>
              <Alert severity="success">
                Identity verified. Your pending change is still here.
              </Alert>
              <Button
                type="button"
                variant="contained"
                size="large"
                onClick={() => onVerified(verifiedMe)}
              >
                {resumeLabel}
              </Button>
            </>
          )}
          {!verifiedMe && methods.length > 1 && (
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
          {!verifiedMe && method === PASSKEY_METHOD && (
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
          {!verifiedMe && method === PASSWORD_LOGIN_METHOD && (
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
          {!verifiedMe && selectedProvider && (
            <>
              <Button
                type="button"
                href={useProviderHandoff
                  ? undefined
                  : selectedProvider.start_url}
                onClick={useProviderHandoff ? verifyProvider : undefined}
                variant="contained"
                size="large"
                disabled={useProviderHandoff && busy}
              >
                {useProviderHandoff && busy
                  ? "Waiting for approval…"
                  : selectedProvider.button_label}
              </Button>
              {!useProviderHandoff && (
                <Typography variant="body2" color="text.secondary">
                  After the secure redirect returns, repeat the Passkey change.
                </Typography>
              )}
              {useProviderHandoff && busy && (
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
          {!verifiedMe && methods.length === 0 && (
            <Alert severity="error">
              This Cowboy Service has no verification method available.
            </Alert>
          )}
        </Stack>
      </Box>
    </ConfirmSheet>
  );
}
