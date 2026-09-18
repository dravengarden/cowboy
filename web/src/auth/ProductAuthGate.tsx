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
  type AuthHostPlugin,
  isRecentProductAuthRequired,
  type ProductAutomationServerPolicy,
  type ProductCapacityServerPolicy,
  type ProductLogoutScope,
  type ProductLogoutServerPolicy,
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
  cachedAuthDecision,
  classifyAuthStatus,
  deleteProductHistoryCache,
  forgetAuthStatus,
  nextAuthStatusBackoffMs,
  nextReadyStatusAction,
  PRODUCT_AUTH_LOST_EVENT,
  rememberAuthStatus,
  shouldMountProductApp,
} from "./authStatus";
import { ProductRecentAuthSheet } from "./ProductRecentAuthSheet";
import { ProductSessionGuard } from "./ProductSessionGuard";
import { PRODUCT_AUTH_SESSION_EVENT } from "../productAuthEvents";
import { bindProductSyncPrincipal, sameProductPrincipal } from "../productSyncIdentity";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  passkeyRegistrationNeedsUserGestureResume,
  registerPasskey,
} from "./passkeyFlow";
import { ConfirmSheet } from "../Sheet";
import { ProductActiveCapacityGuard } from "../capacity/ProductActiveCapacityGuard";
import { ProductLoginPage } from "./ProductLoginPage";
import {
  type RecentProductAuthOptions,
  retryWithRecentProductAuth,
} from "./recentAuth";

export interface ProductAuthValue {
  me: ProductMe;
  hostPlugins: AuthHostPlugin[];
  passkeys: ProductPasskeyServerPolicy | undefined;
  session: ProductSessionServerPolicy | undefined;
  capacity: ProductCapacityServerPolicy | undefined;
  logout: ProductLogoutServerPolicy | undefined;
  automation: ProductAutomationServerPolicy | undefined;
  reauthenticate: (options?: RecentProductAuthOptions) => Promise<ProductMe>;
  updateMe: (me: ProductMe) => void;
  signOut: (options?: {
    scope?: ProductLogoutScope;
    providerLogout?: boolean;
  }) => Promise<void>;
}

const ProductAuthContext = createContext<ProductAuthValue | null>(null);

export function useProductAuth(): ProductAuthValue {
  const value = useContext(ProductAuthContext);
  if (!value) {
    throw new Error("useProductAuth must be used inside ProductAuthGate");
  }
  return value;
}

export async function signOutProductSession(options: {
  scope?: ProductLogoutScope;
  providerLogout?: boolean;
} = {}): Promise<void> {
  let providerLogoutUrl: string | null | undefined;
  try {
    const result = await authApi.logout(
      options.scope ?? "current",
      options.providerLogout === true,
    );
    providerLogoutUrl = result.provider_logout_url;
  } catch (reason) {
    if (isRecentProductAuthRequired(reason)) throw reason;
    // Logout is best-effort: still drop the socket graph and local history.
  }
  const ending = announceProductSessionEnd();
  await Promise.all([deleteProductHistoryCache(), ending]);
  if (providerLogoutUrl) {
    globalThis.location.assign(providerLogoutUrl);
  } else {
    globalThis.location.reload();
  }
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
          This web build needs an authenticated, immutable user identity and
          the product sync dataset protocol. The controller is still activating
          or older than this PWA. /admin remains the break-glass.
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
  const [hostPlugins, setHostPlugins] = useState<AuthHostPlugin[]>([]);
  const [passwordEnabled, setPasswordEnabled] = useState(true);
  const [loginMethodOrder, setLoginMethodOrder] = useState<string[]>([]);
  const [passkeyPolicy, setPasskeyPolicy] = useState<
    ProductPasskeyServerPolicy
  >();
  const [sessionPolicy, setSessionPolicy] = useState<
    ProductSessionServerPolicy
  >();
  const [capacityPolicy, setCapacityPolicy] = useState<
    ProductCapacityServerPolicy
  >();
  const [logoutPolicy, setLogoutPolicy] = useState<
    ProductLogoutServerPolicy
  >();
  const [automationPolicy, setAutomationPolicy] = useState<
    ProductAutomationServerPolicy
  >();
  const attemptsRef = useRef(0);
  const meRef = useRef<ProductMe | null>(null);
  const generationRef = useRef(0);
  const cachedIdentityRef = useRef(false);
  const [cachedIdentity, setCachedIdentity] = useState(false);
  const [pollTick, setPollTick] = useState(0);
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
        if (action === "stay") {
          if (cachedIdentityRef.current) attemptsRef.current += 1;
          return;
        }
        if (action === "update" && decision.me) {
          meRef.current = decision.me;
          setMe(decision.me);
          setView("ready");
          if (!decision.cached && cachedIdentityRef.current) {
            cachedIdentityRef.current = false;
            setCachedIdentity(false);
          }
          return;
        }
        forgetAuthStatus();
        generationRef.current += 1;
        const ending = announceProductSessionEnd();
        await Promise.all([deleteProductHistoryCache(), ending]);
        globalThis.location.reload();
        return;
      }
      if (shouldMountProductApp(decision) && decision.me) {
        if (!bindProductSyncPrincipal(decision.me.user_id)) {
          setView("activating");
          return;
        }
        // A cached principal mounts the app from its local replica while the
        // status probe keeps retrying in the background (see the poll effect).
        attemptsRef.current = decision.cached ? attemptsRef.current + 1 : 0;
        cachedIdentityRef.current = decision.cached === true;
        setCachedIdentity(decision.cached === true);
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
      setHostPlugins(probe.body.host_plugins ?? []);
      setPasswordEnabled(probe.body.password_enabled !== false);
      setLoginMethodOrder(probe.body.login_method_order ?? []);
      setPasskeyPolicy(probe.body.passkeys);
      setSessionPolicy(probe.body.session);
      setCapacityPolicy(probe.body.capacity);
      setLogoutPolicy(probe.body.logout);
      setAutomationPolicy(probe.body.automation);
    }
    const decision = classifyAuthStatus(probe);
    if (probe.kind === "ok") rememberAuthStatus(probe.body);
    if (generation !== generationRef.current) return;
    // Cowboy unreachable: a still-valid cached principal opens the app on its
    // local replica instead of a retry page. Contact later decides for real.
    await applyDecision(meRef.current ? decision : cachedAuthDecision(decision) ?? decision);
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
    if (view !== "activating" && view !== "retry" && !cachedIdentity) return;
    const timer = globalThis.setTimeout(() => {
      void loadStatus();
    }, nextAuthStatusBackoffMs(attemptsRef.current));
    return () => globalThis.clearTimeout(timer);
  }, [view, cachedIdentity, pollTick, loadStatus]);

  // While mounted on a cached principal, keep probing until the server answers
  // for real. Each timer fires `loadStatus`; a `stay` outcome re-arms it.
  useEffect(() => {
    if (!cachedIdentity) return undefined;
    const timer = globalThis.setInterval(() => setPollTick((tick) => tick + 1), 15_000);
    return () => globalThis.clearInterval(timer);
  }, [cachedIdentity]);

  const handleAuthed = useCallback((next: ProductMe): void => {
    const generation = ++generationRef.current;
    void (async () => {
      await deleteProductHistoryCache();
      if (generation !== generationRef.current) return;
      if (!bindProductSyncPrincipal(next.user_id)) {
        setView("activating");
        return;
      }
      attemptsRef.current = 0;
      meRef.current = next;
      setMe(next);
      setView("ready");
    })();
  }, []);

  const signOut = useCallback(async (options: {
    scope?: ProductLogoutScope;
    providerLogout?: boolean;
  } = {}): Promise<void> => {
    generationRef.current += 1;
    forgetAuthStatus();
    await signOutProductSession(options);
  }, []);

  const updateMe = useCallback((next: ProductMe): void => {
    if (meRef.current && !sameProductPrincipal(meRef.current, next)) {
      void applyDecision({ view: "ready", me: next });
      return;
    }
    meRef.current = next;
    setMe(next);
  }, [applyDecision]);

  useEffect(() => {
    const onSession = (event: Event): void => {
      const next = productMeFromJson((event as CustomEvent).detail);
      if (!next || !meRef.current) return;
      if (!sameProductPrincipal(meRef.current, next)) {
        void applyDecision({ view: "ready", me: next });
        return;
      }
      updateMe(next);
    };
    globalThis.addEventListener(PRODUCT_AUTH_SESSION_EVENT, onSession);
    return () =>
      globalThis.removeEventListener(PRODUCT_AUTH_SESSION_EVENT, onSession);
  }, [updateMe, applyDecision]);

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
    if (!meRef.current || !sameProductPrincipal(meRef.current, next)) {
      pending.reject(new Error("Verification returned a different account"));
      setRecentAuthOpen(false);
      generationRef.current += 1;
      void (async () => {
        const ending = announceProductSessionEnd();
        await Promise.all([deleteProductHistoryCache(), ending]);
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
          hostPlugins,
          passkeys: passkeyPolicy,
          session: sessionPolicy,
          capacity: capacityPolicy,
          logout: logoutPolicy,
          automation: automationPolicy,
          reauthenticate,
          updateMe,
          signOut,
        }}
      >
        {children}
        <ProductActiveCapacityGuard />
        <ProductSessionGuard
          me={me}
          policy={sessionPolicy}
          providers={providers}
          hostPlugins={hostPlugins}
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
          hostPlugins={hostPlugins}
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
        hostPlugins={hostPlugins}
        passwordEnabled={passwordEnabled}
        loginMethodOrder={loginMethodOrder}
        onAuthed={handleAuthed}
        onStatus={(status) => {
          setSetupRequired(status.setup_required === true);
          setSetupPending(status.setup_pending === true);
          setProviders(status.providers ?? []);
          setHostPlugins(status.host_plugins ?? []);
          setPasswordEnabled(status.password_enabled !== false);
          setLoginMethodOrder(status.login_method_order ?? []);
          setPasskeyPolicy(status.passkeys);
          setSessionPolicy(status.session);
          setCapacityPolicy(status.capacity);
          setLogoutPolicy(status.logout);
          setAutomationPolicy(status.automation);
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
