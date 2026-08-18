import { Alert, Box, Button, CircularProgress, Stack, Typography } from "@mui/material";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { authApi, type ProductMe, type RegistrationPublicStatus } from "./authApi";
import {
  announceProductSessionEnd,
  classifyAuthStatus,
  deleteProductHistoryCache,
  nextAuthStatusBackoffMs,
  nextReadyStatusAction,
  shouldMountProductApp,
  type AuthGateDecision,
  type AuthGateView,
} from "./authStatus";
import { ProductLoginPage } from "./ProductLoginPage";

export interface ProductAuthValue {
  me: ProductMe;
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
        <Typography sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}>
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
        <Typography sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}>
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
        <Typography sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}>
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

export function ProductAuthGate({
  children,
}: {
  children: ReactNode;
}): React.JSX.Element {
  const [view, setView] = useState<AuthGateView>("loading");
  const [me, setMe] = useState<ProductMe | null>(null);
  const [registration, setRegistration] = useState<RegistrationPublicStatus>({
    enabled: false,
    mode: "disabled",
    accepts_registration: false,
  });
  const attemptsRef = useRef(0);
  const meRef = useRef<ProductMe | null>(null);
  const generationRef = useRef(0);

  const applyDecision = useCallback(async (decision: AuthGateDecision): Promise<void> => {
    if (decision.registration) setRegistration(decision.registration);
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
  }, []);

  const loadStatus = useCallback(async (): Promise<void> => {
    const generation = ++generationRef.current;
    const decision = classifyAuthStatus(await authApi.status());
    if (generation !== generationRef.current) return;
    await applyDecision(decision);
  }, [applyDecision]);

  useEffect(() => {
    void loadStatus();
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

  if (view === "ready" && me) {
    return (
      <ProductAuthContext.Provider value={{ me, signOut }}>
        {children}
      </ProductAuthContext.Provider>
    );
  }
  if (view === "login") {
    return <ProductLoginPage registration={registration} onAuthed={handleAuthed} />;
  }
  if (view === "activating") {
    return <ProductControllerUnavailablePage onRetry={() => void loadStatus()} />;
  }
  if (view === "retry") {
    return <ProductAuthRetryPage onRetry={() => void loadStatus()} />;
  }
  return <ProductAuthSplash label="cowboy" />;
}
