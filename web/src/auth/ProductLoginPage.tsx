import {
  Alert,
  Box,
  Button,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import ArrowForwardRounded from "@mui/icons-material/ArrowForwardRounded";
import { useEffect, useMemo, useRef, useState } from "react";
import { assessAdminPassword } from "../admin/passwordStrength";
import {
  authApi,
  type AuthHostPlugin,
  type AuthStatus,
  PASSWORD_LOGIN_METHOD,
  type ProductMe,
  type ProductOidcProvider,
  resolveProductLoginMethodOrder,
} from "./authApi";
import { corePasswordMode, passwordLoginFields } from "./coreSecurity";
import { PluginSlot } from "../pluginHost/PluginSlot";
import { nativeOidcFlowSupported, runNativeOidc } from "./nativeOidcFlow";
import { loginMethodLabel } from "./productReauthMethods";
import { SegmentedPill } from "../SegmentedPill";

export type OidcLoginContext = {
  kind: "oidc";
  buttonLabel: string;
  startUrl: string;
  native: boolean;
  busy: boolean;
  onStart: () => void;
  onCancel: () => void;
};

type PasswordLoginContext = {
  kind: "password";
  mode: "setup" | "register" | "login";
  fieldLabels: {
    account: string;
    secret: string;
    confirm: string;
    setup: string;
  };
  account: string;
  password: string;
  confirm: string;
  setupToken: string;
  busy: boolean;
  passwordVisible: boolean;
  passwordAcceptable: boolean;
  confirmMismatch: boolean;
  canSubmit: boolean;
  submitLabel: string;
  onSubmit: () => void;
  onAuthed: (me: ProductMe) => void;
  onStatus?: (status: AuthStatus) => void;
  onError: (error: string | null) => void;
  onBusy: (busy: boolean) => void;
  onAccount: (value: string) => void;
  onPassword: (value: string) => void;
  onConfirm: (value: string) => void;
  onSetupToken: (value: string) => void;
  onTogglePasswordVisible: () => void;
};

export type LoginMethodContext = OidcLoginContext | PasswordLoginContext;

export function ProductLoginPage({
  setupRequired,
  setupPending,
  providers,
  hostPlugins = [],
  passwordEnabled,
  loginMethodOrder,
  onAuthed,
  onStatus,
}: {
  setupRequired: boolean;
  setupPending: boolean;
  providers: ProductOidcProvider[];
  hostPlugins?: AuthHostPlugin[];
  passwordEnabled: boolean;
  loginMethodOrder: string[];
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
  const orderedMethodIds = useMemo(
    () =>
      resolveProductLoginMethodOrder(
        loginMethodOrder,
        passwordEnabled,
        providers,
      ),
    [loginMethodOrder, passwordEnabled, providers],
  );
  const [method, setMethod] = useState(() => orderedMethodIds[0] ?? "");
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
    const available = orderedMethodIds.includes(method);
    if (!available) {
      setMethod(orderedMethodIds[0] ?? "");
    }
  }, [method, orderedMethodIds, setupRequired]);
  const passwordScore = assessAdminPassword(password, account);
  const canCreate = account.trim() !== "" && passwordScore.acceptable &&
    password === confirm;
  const confirmMismatch = confirm.length > 0 && password !== confirm;
  const canLogin = account.trim() !== "" && password !== "";
  const loginMethods = orderedMethodIds.flatMap((id) => {
    const label = loginMethodLabel(id, hostPlugins, providers);
    if (!label) return [];
    if (
      id !== PASSWORD_LOGIN_METHOD &&
      !providers.some((candidate) => candidate.id === id)
    ) {
      return [];
    }
    return [{ id, label }];
  });
  const passwordMode = corePasswordMode({
    setupRequired,
    setupPending,
    passwordEnabled,
    selectedMethod: method,
  });
  const selectedProvider = !setupRequired && method !== PASSWORD_LOGIN_METHOD
    ? providers.find((provider) => provider.id === method)
    : undefined;
  const useNativeProviderFlow = selectedProvider !== undefined &&
    nativeOidcFlowSupported();
  const submit = (): void => {
    // A stale selection during a policy change cannot submit a disabled local
    // method, nor can Enter on an OIDC surface submit retained password fields.
    if (busy || passwordMode === null) return;
    setBusy(true);
    setError(null);
    const request = needsCode
      ? authApi.request("/api/auth/setup", {
        method: "POST",
        body: { token: setupToken.trim() },
      }).then((status) => {
        onStatus?.(status as AuthStatus);
      })
      : creating
      ? authApi.request("/api/auth/register", {
        method: "POST",
        body: { account, password },
      }).then((me) => onAuthed(me as ProductMe))
      : authApi.request("/api/auth/login", {
        method: "POST",
        body: { account, password },
      }).then((me) => onAuthed(me as ProductMe));
    void request
      .catch((err: unknown) => {
        setError(
          err instanceof Error ? err.message : "Could not reach Cowboy",
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
          err instanceof Error
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

  const loginContext: LoginMethodContext | null = selectedProvider
    ? {
      kind: "oidc" as const,
      buttonLabel: selectedProvider.button_label,
      startUrl: selectedProvider.start_url,
      native: useNativeProviderFlow,
      busy,
      onStart: submitProvider,
      onCancel: () => providerAbort.current?.abort(),
    }
    : passwordMode !== null
    ? {
      kind: "password" as const,
      mode: passwordMode,
      account,
      password,
      confirm,
      fieldLabels: passwordLoginFields(),
      setupToken,
      busy,
      passwordVisible,
      passwordAcceptable: passwordScore.acceptable,
      confirmMismatch,
      canSubmit: needsCode
        ? setupToken.trim() !== ""
        : creating
        ? canCreate
        : canLogin,
      submitLabel: needsCode
        ? "Continue"
        : creating
        ? "Create account"
        : "Sign in",
      onSubmit: submit,
      onAuthed,
      ...(onStatus ? { onStatus } : {}),
      onError: setError,
      onBusy: setBusy,
      onAccount: setAccount,
      onPassword: setPassword,
      onConfirm: setConfirm,
      onSetupToken: setSetupToken,
      onTogglePasswordVisible: () => setPasswordVisible((visible) => !visible),
    }
    : null;

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
        height: "100%",
        overflowY: "auto",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "flex-start",
        pl: "max(20px, env(safe-area-inset-left, 0px))",
        pr: "max(20px, env(safe-area-inset-right, 0px))",
        pt:
          "max(32px, var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px)))",
        pb: "max(32px, env(safe-area-inset-bottom, 0px))",
        bgcolor: "background.default",
        color: "text.primary",
      }}
    >
      <Stack
        spacing={3}
        useFlexGap
        sx={{
          width: "100%",
          maxWidth: 420,
          flexShrink: 0,
          my: "auto",
          p: { xs: 2.5, sm: 4.5 },
          bgcolor: "background.paper",
          border: "1px solid",
          borderColor: (theme) => alpha(theme.palette.text.primary, 0.08),
          borderRadius: "20px",
          boxShadow: (theme) =>
            `0 8px 32px ${alpha(theme.palette.common.black, 0.035)}`,
          // The workspace's default text scale is 65%; keep sign-in legible
          // at that scale while still honoring larger accessibility sizes.
          "& .MuiAlert-message": {
            fontSize: "max(0.875rem, 14px)",
            lineHeight: 1.5,
          },
        }}
      >
        <Stack direction="row" spacing={1.25} sx={{ alignItems: "center" }}>
          <Box
            component="img"
            src="/cowboy-app-icon-192-v6.png"
            alt=""
            width={36}
            height={36}
            sx={{
              width: 36,
              height: 36,
              borderRadius: "10px",
              display: "block",
            }}
          />
          <Typography
            component="p"
            sx={{
              fontSize: "max(1.125rem, 18px)",
              fontWeight: 650,
              letterSpacing: "-0.025em",
            }}
          >
            Cowboy
          </Typography>
        </Stack>
        <Box>
          <Typography
            component="h1"
            variant="h4"
            sx={{
              fontWeight: 650,
              letterSpacing: "-0.035em",
              fontSize: "max(1.75rem, 28px)",
              lineHeight: 1.25,
            }}
          >
            {needsCode
              ? "Enter setup code"
              : creating
              ? "Create account"
              : "Sign in"}
          </Typography>
          <Typography
            color="text.secondary"
            sx={{ mt: 1, fontSize: "max(0.9375rem, 14px)", lineHeight: 1.6 }}
          >
            {needsCode
              ? "This instance has no user yet. Enter the setup code from the host journal or data directory."
              : creating
              ? "Create the only user on this Cowboy instance."
              : "Sign in to your workspace."}
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
          <SegmentedPill
            value={method}
            options={loginMethods.map((loginMethod) => ({
              value: loginMethod.id,
              label: loginMethod.label,
            }))}
            onChange={setMethod}
            disabled={busy}
            fullWidth
            aria-label="Sign-in method"
            sx={{
              borderRadius: "12px",
              bgcolor: "action.hover",
              backdropFilter: "none",
              WebkitBackdropFilter: "none",
              "& > [aria-hidden]": {
                borderRadius: "8px",
                bgcolor: (theme) =>
                  theme.palette.mode === "dark"
                    ? alpha(theme.palette.common.white, 0.12)
                    : theme.palette.background.paper,
                boxShadow: (theme) =>
                  `0 1px 3px ${alpha(theme.palette.common.black, 0.08)}`,
                "@media (prefers-reduced-motion: reduce)": {
                  transition: "none",
                },
              },
              "& .MuiButtonBase-root": {
                minHeight: 44,
                borderRadius: "8px",
                px: 1,
                fontSize: "max(0.875rem, 14px)",
                fontWeight: 550,
                "&.Mui-focusVisible": {
                  outline: "2px solid",
                  outlineColor: "primary.main",
                  outlineOffset: 2,
                },
              },
            }}
          />
        )}
        {loginContext?.kind === "password"
          ? <LoginMethodFallback context={loginContext} />
          : loginContext?.kind === "oidc" && selectedProvider
          ? (
            <PluginSlot
              pluginId={selectedProvider.id}
              slot="login.method"
              context={loginContext}
              placeholder={null}
              render={() => <LoginMethodFallback context={loginContext} />}
            >
              <LoginMethodFallback context={loginContext} />
            </PluginSlot>
          )
          : (
            <Alert severity="warning">
              No configured sign-in method is available.
            </Alert>
          )}
      </Stack>
    </Box>
  );
}

/** Cowboy-owned renderer for the closed login-method context union. */
export function LoginMethodFallback(
  { context }: { context: LoginMethodContext },
): React.JSX.Element {
  if (context.kind === "oidc") {
    return (
      <Stack spacing={1.25}>
        <Button
          type="button"
          href={context.native ? undefined : context.startUrl}
          onClick={context.native ? context.onStart : undefined}
          variant="contained"
          size="large"
          fullWidth
          disableElevation
          disabled={context.native && context.busy}
          endIcon={context.native && context.busy
            ? undefined
            : <ArrowForwardRounded />}
          sx={loginActionSx}
        >
          {context.native && context.busy
            ? "Waiting for approval…"
            : context.buttonLabel}
        </Button>
        {context.native && context.busy && (
          <Button
            type="button"
            variant="text"
            onClick={context.onCancel}
            sx={{
              textTransform: "none",
              fontWeight: 600,
              fontSize: "max(0.875rem, 14px)",
            }}
          >
            Cancel
          </Button>
        )}
      </Stack>
    );
  }
  return (
    <Stack
      spacing={2}
      sx={{
        "& .MuiInputBase-root, & .MuiInputLabel-root": {
          fontSize: "max(1rem, 16px)",
        },
        "& .MuiFormHelperText-root": {
          fontSize: "max(0.75rem, 12px)",
        },
      }}
    >
      {context.mode === "setup"
        ? (
          <TextField
            label={context.fieldLabels.setup}
            value={context.setupToken}
            onChange={(event) => context.onSetupToken(event.target.value)}
            autoComplete="one-time-code"
            fullWidth
          />
        )
        : (
          <>
            <TextField
              label={context.fieldLabels.account}
              name="username"
              value={context.account}
              onChange={(event) => context.onAccount(event.target.value)}
              autoComplete="username"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              fullWidth
            />
            <TextField
              label={context.fieldLabels.secret}
              name={context.mode === "register" ? "new-password" : "password"}
              type="password"
              value={context.password}
              onChange={(event) => context.onPassword(event.target.value)}
              autoComplete={context.mode === "register"
                ? "new-password"
                : "current-password"}
              fullWidth
            />
            {context.mode === "register" && (
              <TextField
                label={context.fieldLabels.confirm}
                type="password"
                value={context.confirm}
                onChange={(event) => context.onConfirm(event.target.value)}
                autoComplete="new-password"
                error={context.confirmMismatch}
                helperText={context.confirmMismatch
                  ? "Passwords do not match"
                  : undefined}
                fullWidth
              />
            )}
          </>
        )}
      <Button
        type="submit"
        variant="contained"
        size="large"
        fullWidth
        disableElevation
        disabled={context.busy || !context.canSubmit}
        sx={loginActionSx}
      >
        {context.submitLabel}
      </Button>
    </Stack>
  );
}

const loginActionSx = {
  textTransform: "none" as const,
  fontWeight: 600,
  letterSpacing: 0,
  minHeight: 48,
  px: 1.5,
  textAlign: "center" as const,
  borderRadius: "10px",
  fontSize: "max(0.95rem, 15px)",
  "& .MuiButton-endIcon.MuiButton-icon > :nth-of-type(1)": {
    fontSize: "1.2em",
  },
  "&.Mui-focusVisible": {
    outline: "2px solid",
    outlineColor: "primary.main",
    outlineOffset: 3,
  },
};
