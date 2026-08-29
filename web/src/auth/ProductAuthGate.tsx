import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  authApi,
  type ProductMe,
  type ProductOidcProvider,
  type ProductPasskeyServerPolicy,
} from "./authApi";
import {
  announceProductSessionEnd,
  type AuthGateDecision,
  type AuthGateView,
  classifyAuthStatus,
  deleteProductHistoryCache,
  nextAuthStatusBackoffMs,
  nextReadyStatusAction,
  PRODUCT_AUTH_LOST_EVENT,
  shouldMountProductApp,
} from "./authStatus";
import { PasskeyReauthLock } from "./PasskeyReauthLock";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  registerPasskey,
} from "./passkeyFlow";
import { ProductLoginPage } from "./ProductLoginPage";
import { ConfirmSheet } from "../Sheet";

export interface ProductAuthValue {
  me: ProductMe;
  passkeys: ProductPasskeyServerPolicy | undefined;
  updateMe: (me: ProductMe) => void;
  signOut: () => Promise<void>;
}

const ProductAuthContext = createContext<ProductAuthValue | null>(null);

export function useProductAuth(): ProductAuthValue {
  const value = useContext(ProductAuthContext);
  if (!value) {
    throw new Error("useProductAuth must be used inside ProductAuthGate");
  }
  return value;
}

export async function signOutProductSession(): Promise<void> {
  try {
    await authApi.logout();
  } catch {
    // Logout is best-effort: still drop the socket graph and local history.
  }
  await deleteProductHistoryCache();
  announceProductSessionEnd();
  globalThis.location.reload();
}

function ProductAuthSplash({ label }: { label: string }): React.JSX.Element {
  return (
    <Box
      sx={{
        minHeight: "100%",
        display: "grid",
        placeItems: "center",
        bgcolor: "background.default",
        color: "text.secondary",
      }}
    >
      <Stack spacing={2} alignItems="center">
        <CircularProgress size={28} color="inherit" />
        <Typography
          sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}
        >
          {label}
        </Typography>
      </Stack>
    </Box>
  );
}

function ProductControllerUnavailablePage({
  onRetry,
}: {
  onRetry: () => void;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        minHeight: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        px: 3,
        bgcolor: "background.default",
      }}
    >
      <Stack spacing={2} sx={{ width: "100%", maxWidth: 420 }}>
        <Typography
          sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}
        >
          cowboy
        </Typography>
        <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: -0.4 }}>
          Controller too old or activating
        </Typography>
        <Typography color="text.secondary">
          This web build needs GET /api/auth/status. The controller is still
          activating or older than this PWA. /admin remains the break-glass.
        </Typography>
        <Button variant="contained" onClick={onRetry}>Retry</Button>
      </Stack>
    </Box>
  );
}

function ProductAuthRetryPage({
  onRetry,
}: {
  onRetry: () => void;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        minHeight: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        px: 3,
        bgcolor: "background.default",
      }}
    >
      <Stack spacing={2} sx={{ width: "100%", maxWidth: 420 }}>
        <Typography
          sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}
        >
          cowboy
        </Typography>
        <Alert severity="warning">
          Can&apos;t reach Cowboy. Retrying — this is not a sign-in problem.
        </Alert>
        <Button variant="contained" onClick={onRetry}>Retry now</Button>
      </Stack>
    </Box>
  );
}

function PasskeySetupPrompt({
  me,
  policy,
  onCreated,
}: {
  me: ProductMe;
  policy: ProductPasskeyServerPolicy | undefined;
  onCreated: (me: ProductMe) => void;
}): React.JSX.Element {
  const dismissalKey = `cowboy-passkey-setup-dismissed:${me.account}`;
  const [dismissed, setDismissed] = useState(() => {
    try {
      return globalThis.localStorage.getItem(dismissalKey) === "1";
    } catch {
      return false;
    }
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nickname, setNickname] = useState("");
  const open = policy?.enabled === true && policy.prompt_after_login &&
    (me.passkey_count ?? 0) === 0 && !dismissed;
  const dismiss = (): void => {
    try {
      globalThis.localStorage.setItem(dismissalKey, "1");
    } catch {
      // A private browser may reject storage; dismissal still lasts this mount.
    }
    setDismissed(true);
  };
  const add = (): void => {
    if (busy || !passkeyFlowSupported() || nickname.trim() === "") return;
    setBusy(true);
    setError(null);
    void (async () => {
      await registerPasskey(nickname.trim());
      onCreated(await authApi.me());
    })().catch((reason: unknown) => {
      if (passkeyFlowCancelled(reason)) return;
      setError(passkeyErrorMessage(reason, "Could not add a Passkey"));
    }).finally(() => setBusy(false));
  };
  return (
    <ConfirmSheet
      open={open}
      onClose={dismiss}
      title="Set up a Passkey?"
      actions={
        <>
          <Button color="inherit" onClick={dismiss}>Not now</Button>
          <Button
            variant="contained"
            disabled={
              busy || !passkeyFlowSupported() || nickname.trim() === ""
            }
            onClick={add}
          >
            Add Passkey
          </Button>
        </>
      }
    >
      <Stack spacing={1.5}>
        <Typography color="text.secondary">
          A Passkey is optional. It adds phishing-resistant verification and can
          refresh this browser&apos;s session after you explicitly verify.
          Periodic Passkey verification stays off until you enable it in
          Settings.
        </Typography>
        <TextField
          label="Passkey name"
          value={nickname}
          onChange={(event) => setNickname(event.target.value)}
          slotProps={{ htmlInput: { maxLength: 64 } }}
          fullWidth
        />
        {!passkeyFlowSupported() && (
          <Alert severity="info">This browser cannot create a Passkey.</Alert>
        )}
        {error && <Alert severity="error">{error}</Alert>}
      </Stack>
    </ConfirmSheet>
  );
}

export function ProductAuthGate({
  children,
}: {
  children: ReactNode;
}): React.JSX.Element {
  const [view, setView] = useState<AuthGateView>("loading");
  const [me, setMe] = useState<ProductMe | null>(null);
  const [setupRequired, setSetupRequired] = useState(false);
  const [setupPending, setSetupPending] = useState(false);
  const [providers, setProviders] = useState<ProductOidcProvider[]>([]);
  const [passwordEnabled, setPasswordEnabled] = useState(true);
  const [passkeyPolicy, setPasskeyPolicy] = useState<
    ProductPasskeyServerPolicy
  >();
  const attemptsRef = useRef(0);
  const meRef = useRef<ProductMe | null>(null);
  const generationRef = useRef(0);

  const applyDecision = useCallback(
    async (decision: AuthGateDecision): Promise<void> => {
      if (decision.setup_required !== undefined) {
        setSetupRequired(decision.setup_required);
      }
      if (decision.setup_pending !== undefined) {
        setSetupPending(decision.setup_pending);
      }
      if (meRef.current) {
        const action = nextReadyStatusAction(meRef.current, decision);
        if (action === "stay") return;
        if (action === "update" && decision.me) {
          meRef.current = decision.me;
          setMe(decision.me);
          setView("ready");
          return;
        }
        generationRef.current += 1;
        await deleteProductHistoryCache();
        announceProductSessionEnd();
        globalThis.location.reload();
        return;
      }
      if (shouldMountProductApp(decision) && decision.me) {
        attemptsRef.current = 0;
        meRef.current = decision.me;
        setMe(decision.me);
        setView("ready");
        return;
      }
      if (decision.view === "login") {
        attemptsRef.current = 0;
        setView("login");
        return;
      }
      attemptsRef.current += 1;
      setView(decision.view);
    },
    [],
  );

  const loadStatus = useCallback(async (): Promise<void> => {
    const generation = ++generationRef.current;
    const probe = await authApi.status();
    if (probe.kind === "ok") {
      setProviders(probe.body.providers ?? []);
      setPasswordEnabled(probe.body.password_enabled !== false);
      setPasskeyPolicy(probe.body.passkeys);
    }
    const decision = classifyAuthStatus(probe);
    if (generation !== generationRef.current) return;
    await applyDecision(decision);
  }, [applyDecision]);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    const onAuthLost = (): void => {
      if (!meRef.current) return;
      // A socket can report an auth-looking failure while the Controller is
      // activating. Confirm against the public status endpoint before tearing
      // down the mounted app; auth-off local owners cannot actually log out.
      void loadStatus();
    };
    globalThis.addEventListener(PRODUCT_AUTH_LOST_EVENT, onAuthLost);
    return () =>
      globalThis.removeEventListener(PRODUCT_AUTH_LOST_EVENT, onAuthLost);
  }, [loadStatus]);

  useEffect(() => {
    if (view !== "activating" && view !== "retry") return;
    const timer = globalThis.setTimeout(() => {
      void loadStatus();
    }, nextAuthStatusBackoffMs(attemptsRef.current));
    return () => globalThis.clearTimeout(timer);
  }, [view, loadStatus]);

  const handleAuthed = useCallback((next: ProductMe): void => {
    generationRef.current += 1;
    void (async () => {
      await deleteProductHistoryCache();
      attemptsRef.current = 0;
      meRef.current = next;
      setMe(next);
      setView("ready");
    })();
  }, []);

  const signOut = useCallback(async (): Promise<void> => {
    generationRef.current += 1;
    await signOutProductSession();
  }, []);

  const updateMe = useCallback((next: ProductMe): void => {
    meRef.current = next;
    setMe(next);
  }, []);

  if (view === "ready" && me) {
    return (
      <ProductAuthContext.Provider
        value={{ me, passkeys: passkeyPolicy, updateMe, signOut }}
      >
        {children}
        {me.auth_enabled !== false &&
          passkeyPolicy?.session_refresh_enabled !== false && (
          <PasskeyReauthLock
            me={me}
            onUnlocked={updateMe}
            onSignOut={signOut}
          />
        )}
        {me.auth_enabled !== false && (
          <PasskeySetupPrompt
            me={me}
            policy={passkeyPolicy}
            onCreated={updateMe}
          />
        )}
      </ProductAuthContext.Provider>
    );
  }
  if (view === "login") {
    return (
      <ProductLoginPage
        setupRequired={setupRequired}
        setupPending={setupPending}
        providers={providers}
        passwordEnabled={passwordEnabled}
        onAuthed={handleAuthed}
        onStatus={(status) => {
          setSetupRequired(status.setup_required === true);
          setSetupPending(status.setup_pending === true);
          setProviders(status.providers ?? []);
          setPasswordEnabled(status.password_enabled !== false);
          setPasskeyPolicy(status.passkeys);
        }}
      />
    );
  }
  if (view === "activating") {
    return (
      <ProductControllerUnavailablePage onRetry={() => void loadStatus()} />
    );
  }
  if (view === "retry") {
    return <ProductAuthRetryPage onRetry={() => void loadStatus()} />;
  }
  return <ProductAuthSplash label="cowboy" />;
}
