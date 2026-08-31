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
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  authApi,
  AuthApiError,
  PASSWORD_LOGIN_METHOD,
  type ProductMe,
  type ProductOidcProvider,
  resolveProductLoginMethodOrder,
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
import { announceProductAuthCookieChanged } from "../productAuthEvents";
import {
  type ProductAccountVerificationMethod,
  productAccountVerificationMethods,
  type ProductPrimaryReauthMethods,
  providerVerificationMethodId,
  resolvePrimaryReauthMethods,
} from "./productReauthMethods";

const PASSKEY_METHOD = "passkey";

function initialMethod(
  me: ProductMe,
  purpose: ProductVerificationPurpose,
  accountMethods: ProductAccountVerificationMethod[],
  primaryMethods: ProductPrimaryReauthMethods,
): string {
  const passkeyAvailable = (me.passkey_count ?? 0) > 0 &&
    passkeyFlowSupported();
  if (purpose === "passkey") {
    return passkeyAvailable ? PASSKEY_METHOD : "";
  }
  if (purpose === "recent" && passkeyAvailable) {
    return PASSKEY_METHOD;
  }
  const candidates = purpose === "primary"
    ? primaryMethods.methods
    : accountMethods;
  return candidates[0]?.id ?? "";
}

export type ProductVerificationPurpose = "recent" | "passkey" | "primary";

export function ProductRecentAuthSheet({
  open,
  me,
  providers,
  passwordEnabled,
  loginMethodOrder,
  requireResumeGesture = false,
  resumeLabel = "Continue",
  purpose = "recent",
  locked = false,
  onVerified,
  onCancel,
  onSignOut,
}: {
  open: boolean;
  me: ProductMe;
  providers: ProductOidcProvider[];
  passwordEnabled: boolean;
  loginMethodOrder: string[];
  requireResumeGesture?: boolean;
  resumeLabel?: string;
  purpose?: ProductVerificationPurpose;
  locked?: boolean;
  onVerified: (me: ProductMe) => void;
  onCancel: () => void;
  onSignOut?: () => void;
}): React.JSX.Element {
  const mobile = useSurfaceProfile().kind !== "desktop";
  const orderedLoginMethodIds = useMemo(
    () =>
      resolveProductLoginMethodOrder(
        loginMethodOrder,
        passwordEnabled,
        providers,
      ),
    [loginMethodOrder, passwordEnabled, providers],
  );
  const accountMethods = useMemo(
    () =>
      productAccountVerificationMethods(
        orderedLoginMethodIds,
        passwordEnabled,
        providers,
      ),
    [orderedLoginMethodIds, passwordEnabled, providers],
  );
  const primaryMethods = useMemo(
    () => resolvePrimaryReauthMethods(me.primary_auth_method, accountMethods),
    [accountMethods, me.primary_auth_method],
  );
  const [method, setMethod] = useState(() =>
    initialMethod(
      me,
      purpose,
      accountMethods,
      primaryMethods,
    )
  );
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [verifiedMe, setVerifiedMe] = useState<ProductMe | null>(null);
  const providerAbort = useRef<AbortController | null>(null);
  const passkeyAvailable = (me.passkey_count ?? 0) > 0 &&
    passkeyFlowSupported();
  const methods = purpose === "passkey"
    ? (passkeyAvailable ? [{ id: PASSKEY_METHOD, label: "Passkey" }] : [])
    : purpose === "primary"
    ? primaryMethods.methods
    : [
      ...(purpose === "recent" && passkeyAvailable
        ? [{ id: PASSKEY_METHOD, label: "Passkey" }]
        : []),
      ...accountMethods,
    ];
  const selectedProvider = providers.find((provider) =>
    providerVerificationMethodId(provider.id) === method
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
      initialMethod(
        me,
        purpose,
        accountMethods,
        primaryMethods,
      ),
    );
  }, [accountMethods, me, open, primaryMethods, purpose]);
  useEffect(() => {
    if (methods.some((candidate) => candidate.id === method)) return;
    setMethod(
      initialMethod(
        me,
        purpose,
        accountMethods,
        primaryMethods,
      ),
    );
  }, [
    accountMethods,
    me,
    method,
    methods,
    primaryMethods,
    purpose,
  ]);

  const finish = (
    request: () => Promise<ProductMe>,
    fallback: string,
  ): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void request()
      .then((next) => {
        // Password/provider login and an enabled Passkey refresh can replace
        // the cookie. Detach the old authenticated socket before it can push
        // stale deadlines back over the newly verified session.
        announceProductAuthCookieChanged();
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
    if (busy || locked) return;
    onCancel();
  };

  const title = purpose === "primary"
    ? "Sign in again"
    : purpose === "passkey"
    ? "Unlock Cowboy"
    : "Verify it’s you";
  const description = purpose === "primary"
    ? locked
      ? "Your service’s primary-login limit has been reached. Reauthenticate with the same method that started this browser session; running agents continue in the background."
      : "Your service’s primary-login deadline is approaching. Reauthenticate with the same method that started this browser session; running agents continue in the background."
    : purpose === "passkey"
    ? locked
      ? "Your scheduled Passkey check is due. Verify locally to unlock this view; running agents continue in the background."
      : "Your scheduled Passkey check is approaching. Verify now without interrupting running agents."
    : `Passkey changes require a sign-in or Passkey check from the last five minutes. Verify now, then ${
      requireResumeGesture
        ? "tap Continue once so Safari can open the Passkey prompt."
        : "Cowboy will continue your pending change automatically."
    }`;

  return (
    <ConfirmSheet
      open={open}
      onClose={cancel}
      title={title}
      actions={locked
        ? onSignOut && (
          <Button color="inherit" disabled={busy} onClick={onSignOut}>
            Sign out
          </Button>
        )
        : (
          <Button color="inherit" disabled={busy} onClick={cancel}>
            Cancel
          </Button>
        )}
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
            {description}
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          {!verifiedMe && purpose === "primary" &&
            primaryMethods.legacySession && (
            <Alert severity="info">
              This session predates sign-in-method tracking. Choose the same
              method you originally used; Cowboy will bind it to this session
              after successful verification.
            </Alert>
          )}
          {!verifiedMe && purpose === "primary" &&
            primaryMethods.unavailableMethod && (
            <Alert severity="warning">
              This session was signed in with{" "}
              {primaryMethods.unavailableMethod}, but that method is no longer
              enabled. Sign out to start a new session with another method.
            </Alert>
          )}
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
                autoFocus={!mobile && purpose === "primary"}
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
              {purpose === "primary" && primaryMethods.unavailableMethod
                ? "The original sign-in method is unavailable for this session."
                : "This Cowboy Service has no verification method available."}
            </Alert>
          )}
        </Stack>
      </Box>
    </ConfirmSheet>
  );
}
