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
  productMeFromJson,
  type ProductOidcProvider,
  type ProductPasskeyServerPolicy,
  type ProductSessionServerPolicy,
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
import { ProductRecentAuthSheet } from "./ProductRecentAuthSheet";
import { ProductSessionGuard } from "./ProductSessionGuard";
import { PRODUCT_AUTH_SESSION_EVENT } from "../productAuthEvents";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  passkeyRegistrationNeedsUserGestureResume,
  registerPasskey,
} from "./passkeyFlow";
import { ConfirmSheet } from "../Sheet";
import { ProductLoginPage } from "./ProductLoginPage";
import {
  type RecentProductAuthOptions,
  retryWithRecentProductAuth,
} from "./recentAuth";

export interface ProductAuthValue {
  me: ProductMe;
  passkeys: ProductPasskeyServerPolicy | undefined;
  session: ProductSessionServerPolicy | undefined;
  reauthenticate: (options?: RecentProductAuthOptions) => Promise<ProductMe>;
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
  reauthenticate,
  suspended,
}: {
  me: ProductMe;
  policy: ProductPasskeyServerPolicy | undefined;
  onCreated: (me: ProductMe) => void;
  reauthenticate: () => Promise<ProductMe>;
  suspended: boolean;
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
  const [notice, setNotice] = useState<string | null>(null);
  const [nickname, setNickname] = useState("");
  const open = policy?.enabled === true && policy.prompt_after_login &&
    (me.passkey_count ?? 0) === 0 && !dismissed && !suspended;
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
    setNotice(null);
    void (async () => {
      await retryWithRecentProductAuth(
        () => registerPasskey(nickname.trim()),
        reauthenticate,
        {
          resumeLabel: "Continue to Passkey",
          resumeWithUserGesture: passkeyRegistrationNeedsUserGestureResume(),
        },
      );
      onCreated({
        ...me,
        passkey_count: Math.max(1, me.passkey_count ?? 0),
      });
      void authApi.me().then(onCreated).catch(() => undefined);
    })().catch((reason: unknown) => {
      if (passkeyFlowCancelled(reason)) {
        setNotice("Passkey setup was cancelled. Nothing changed.");
        return;
      }
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
            disabled={busy || !passkeyFlowSupported() || nickname.trim() === ""}
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
        {notice && <Alert severity="info">{notice}</Alert>}
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
  const [loginMethodOrder, setLoginMethodOrder] = useState<string[]>([]);
  const [passkeyPolicy, setPasskeyPolicy] = useState<
    ProductPasskeyServerPolicy
  >();
  const [sessionPolicy, setSessionPolicy] = useState<
    ProductSessionServerPolicy
  >();
  const attemptsRef = useRef(0);
  const meRef = useRef<ProductMe | null>(null);
  const generationRef = useRef(0);
  const recentAuthRef = useRef<
    {
      promise: Promise<ProductMe>;
      resolve: (me: ProductMe) => void;
      reject: (reason: unknown) => void;
    } | null
  >(null);
  const [recentAuthOpen, setRecentAuthOpen] = useState(false);
  const [recentAuthOptions, setRecentAuthOptions] = useState<
    RecentProductAuthOptions
  >({});

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
      setLoginMethodOrder(probe.body.login_method_order ?? []);
      setPasskeyPolicy(probe.body.passkeys);
      setSessionPolicy(probe.body.session);
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

  useEffect(() => {
    const onSession = (event: Event): void => {
      const next = productMeFromJson((event as CustomEvent).detail);
      if (!next || next.account !== meRef.current?.account) return;
      updateMe(next);
    };
    globalThis.addEventListener(PRODUCT_AUTH_SESSION_EVENT, onSession);
    return () =>
      globalThis.removeEventListener(PRODUCT_AUTH_SESSION_EVENT, onSession);
  }, [updateMe]);

  const reauthenticate = useCallback((
    options: RecentProductAuthOptions = {},
  ): Promise<ProductMe> => {
    if (recentAuthRef.current) return recentAuthRef.current.promise;
    let resolve!: (me: ProductMe) => void;
    let reject!: (reason: unknown) => void;
    const promise = new Promise<ProductMe>((accept, decline) => {
      resolve = accept;
      reject = decline;
    });
    recentAuthRef.current = { promise, resolve, reject };
    setRecentAuthOptions(options);
    setRecentAuthOpen(true);
    return promise;
  }, []);

  const completeRecentAuth = useCallback((next: ProductMe): void => {
    const pending = recentAuthRef.current;
    if (!pending) return;
    recentAuthRef.current = null;
    if (meRef.current?.account !== next.account) {
      pending.reject(new Error("Verification returned a different account"));
      setRecentAuthOpen(false);
      generationRef.current += 1;
      void (async () => {
        await deleteProductHistoryCache();
        announceProductSessionEnd();
        globalThis.location.reload();
      })();
      return;
    }
    updateMe(next);
    setRecentAuthOpen(false);
    setRecentAuthOptions({});
    pending.resolve(next);
  }, [updateMe]);

  const cancelRecentAuth = useCallback((): void => {
    const pending = recentAuthRef.current;
    if (!pending) return;
    recentAuthRef.current = null;
    setRecentAuthOpen(false);
    setRecentAuthOptions({});
    pending.reject(new DOMException("Cancelled", "AbortError"));
  }, []);

  useEffect(() => () => {
    const pending = recentAuthRef.current;
    recentAuthRef.current = null;
    pending?.reject(new DOMException("Cancelled", "AbortError"));
  }, []);

  if (view === "ready" && me) {
    return (
      <ProductAuthContext.Provider
        value={{
          me,
          passkeys: passkeyPolicy,
          session: sessionPolicy,
          reauthenticate,
          updateMe,
          signOut,
        }}
      >
        {children}
        <ProductSessionGuard
          me={me}
          policy={sessionPolicy}
          providers={providers}
          passwordEnabled={passwordEnabled}
          loginMethodOrder={loginMethodOrder}
          suspended={recentAuthOpen}
          onVerified={updateMe}
          onSignOut={signOut}
        />
        {me.auth_enabled !== false && (
          <PasskeySetupPrompt
            me={me}
            policy={passkeyPolicy}
            onCreated={updateMe}
            reauthenticate={reauthenticate}
            suspended={recentAuthOpen || me.session_reauth_kind != null}
          />
        )}
        <ProductRecentAuthSheet
          open={recentAuthOpen}
          me={me}
          providers={providers}
          passwordEnabled={passwordEnabled}
          loginMethodOrder={loginMethodOrder}
          requireResumeGesture={recentAuthOptions.resumeWithUserGesture ===
            true}
          resumeLabel={recentAuthOptions.resumeLabel ?? "Continue"}
          onVerified={completeRecentAuth}
          onCancel={cancelRecentAuth}
        />
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
        loginMethodOrder={loginMethodOrder}
        onAuthed={handleAuthed}
        onStatus={(status) => {
          setSetupRequired(status.setup_required === true);
          setSetupPending(status.setup_pending === true);
          setProviders(status.providers ?? []);
          setPasswordEnabled(status.password_enabled !== false);
          setLoginMethodOrder(status.login_method_order ?? []);
          setPasskeyPolicy(status.passkeys);
          setSessionPolicy(status.session);
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
