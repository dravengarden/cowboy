import {
  DnsOutlined,
  HubOutlined,
  Inventory2Outlined,
  ManageAccountsOutlined,
  PolicyOutlined,
  TuneOutlined,
} from "@mui/icons-material";
import {
  Alert,
  AppBar,
  Box,
  Button,
  Container,
  Drawer,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Toolbar,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  adminApi,
  type AdminAuthStatus,
  type AdminMachine,
  type AdminOverview,
  type AdminSession,
  type AdminUser,
  type PermissionPolicy,
  type ProductUser,
  type ProviderRelease,
  type SessionLimits,
} from "./adminApi";
import { AdminPasskeyLock, AdminPasskeysCard } from "./AdminPasskeys";

export type AdminRoute =
  | "/admin"
  | "/admin/login"
  | "/admin/accounts"
  | "/admin/permissions"
  | "/admin/releases"
  | "/admin/sessions"
  | "/admin/limits";

const NAV: { path: AdminRoute; label: string; icon: ReactNode }[] = [
  { path: "/admin", label: "Overview", icon: <HubOutlined /> },
  { path: "/admin/accounts", label: "Accounts", icon: <ManageAccountsOutlined /> },
  { path: "/admin/permissions", label: "Permissions", icon: <PolicyOutlined /> },
  { path: "/admin/releases", label: "Releases", icon: <Inventory2Outlined /> },
  { path: "/admin/sessions", label: "Sessions", icon: <DnsOutlined /> },
  { path: "/admin/limits", label: "Session limits", icon: <TuneOutlined /> },
];

function currentRoute(): AdminRoute {
  const path = globalThis.location.pathname.replace(/\/$/, "") || "/admin";
  return NAV.some((item) => item.path === path) ? path as AdminRoute : "/admin";
}

export function AdminApp(): React.JSX.Element {
  const [auth, setAuth] = useState<AdminAuthStatus | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);
  const loadAuth = useCallback(async () => {
    setAuth(await adminApi.auth());
  }, []);
  useEffect(() => {
    void loadAuth().catch((error: Error) => setAuthError(error.message));
  }, [loadAuth]);
  if (authError) return <Alert severity="error">{authError}</Alert>;
  if (!auth) return <Typography sx={{ p: 4 }}>Loading…</Typography>;
  if (!auth.authenticated) {
    return (
      <AdminLoginPage
        bootstrap={auth.bootstrap_required}
        onAuthed={setAuth}
      />
    );
  }
  return (
    <>
      <AdminShell auth={auth} onAuth={setAuth} onLogout={() => void adminApi.logout().then(setAuth)} />
      <AdminPasskeyLock auth={auth} onAuth={setAuth} />
    </>
  );
}

function AdminLoginPage({
  bootstrap,
  onAuthed,
}: {
  bootstrap: boolean;
  onAuthed: (auth: AdminAuthStatus) => void;
}): React.JSX.Element {
  const [account, setAccount] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (bootstrap) {
    return (
      <Box sx={{ minHeight: "100vh", display: "grid", placeItems: "center", p: 3 }}>
        <Paper sx={{ p: 4, width: "100%", maxWidth: 420 }}>
          <Stack spacing={2}>
            <Typography variant="h5">Cowboy Admin</Typography>
            <Typography color="text.secondary">
              This instance has no user yet. Open / to enter the setup code and create
              the only account. Admin login uses that same account afterward.
            </Typography>
            <Button variant="contained" href="/">Open Cowboy</Button>
          </Stack>
        </Paper>
      </Box>
    );
  }
  return (
    <Box
      component="form"
      method="post"
      action="#"
      autoComplete="on"
      onSubmit={(event) => {
        event.preventDefault();
        void adminApi.login(account, password).then(onAuthed).catch((err: Error) => setError(err.message));
      }}
      sx={{ minHeight: "100vh", display: "grid", placeItems: "center", p: 3 }}
    >
      <Paper sx={{ p: 4, width: "100%", maxWidth: 420 }}>
        <Stack spacing={2}>
          <Typography variant="h5">Cowboy Admin</Typography>
          <Typography color="text.secondary">
            Sign in to the admin console. Admin sessions last 12 hours.
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          <TextField
            label="Account"
            name="username"
            value={account}
            onChange={(event) => setAccount(event.target.value)}
            autoComplete="username"
            slotProps={{
              htmlInput: {
                id: "admin-username",
                name: "username",
                autoCapitalize: "none",
                autoCorrect: "off",
                spellCheck: false,
              },
            }}
          />
          <TextField
            label="Password"
            name="password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
            slotProps={{
              htmlInput: {
                id: "admin-password",
                name: "password",
                autoCapitalize: "off",
                autoCorrect: "off",
                spellCheck: false,
              },
            }}
          />
          <Button type="submit" variant="contained">Sign in</Button>
        </Stack>
      </Paper>
    </Box>
  );
}

function AdminShell({
  auth,
  onAuth,
  onLogout,
}: {
  auth: AdminAuthStatus;
  onAuth: (auth: AdminAuthStatus) => void;
  onLogout: () => void;
}): React.JSX.Element {
  const [route, setRoute] = useState<AdminRoute>(currentRoute);
  useEffect(() => {
    const onPop = (): void => setRoute(currentRoute());
    globalThis.addEventListener("popstate", onPop);
    return () => globalThis.removeEventListener("popstate", onPop);
  }, []);
  const go = (path: AdminRoute): void => {
    if (path !== route) {
      globalThis.history.pushState({}, "", path);
      setRoute(path);
    }
  };
  return (
    <Box sx={{ display: "flex", minHeight: "100vh" }}>
      <AppBar position="fixed" color="inherit" elevation={0} sx={{ borderBottom: 1, borderColor: "divider" }}>
        <Toolbar>
          <Typography variant="h6" sx={{ fontWeight: 700, flexGrow: 1 }}>Cowboy Admin</Typography>
          <Typography variant="body2" sx={{ mr: 2 }}>{auth.account} · {auth.role}</Typography>
          <Button color="inherit" onClick={onLogout}>Sign out</Button>
        </Toolbar>
      </AppBar>
      <Drawer variant="permanent" sx={{ width: 240, [`& .MuiDrawer-paper`]: { width: 240, top: 64 } }}>
        <List>
          {NAV.map((item) => (
            <ListItemButton key={item.path} selected={route === item.path} onClick={() => go(item.path)}>
              <ListItemIcon>{item.icon}</ListItemIcon>
              <ListItemText primary={item.label} />
            </ListItemButton>
          ))}
        </List>
      </Drawer>
      <Container maxWidth="lg" sx={{ pt: 12, pb: 6, ml: "240px" }}>
        {route === "/admin" && <OverviewPage />}
        {route === "/admin/accounts" && <AccountsPage auth={auth} onAuth={onAuth} />}
        {route === "/admin/permissions" && <PermissionsPage />}
        {route === "/admin/releases" && <ReleasesPage />}
        {route === "/admin/sessions" && <SessionsPage />}
        {route === "/admin/limits" && <LimitsPage />}
      </Container>
    </Box>
  );
}

function OverviewPage(): React.JSX.Element {
  const [data, setData] = useState<AdminOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void adminApi.overview().then(setData).catch((err: Error) => setError(err.message));
  }, []);
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!data) return <Typography>Loading…</Typography>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Overview</Typography>
      <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
        <Stat title="Health" value={data.healthy ? "ok" : "degraded"} />
        <Stat title="Persistence" value={data.persistence} />
        <Stat title="Sessions" value={String(data.sessions_live)} />
        <Stat title="Accounts" value="single-user" />
      </Stack>
      <Paper sx={{ p: 2 }}>
        <Typography variant="subtitle2" color="text.secondary">Runtime</Typography>
        <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
          {`backend ${data.backend}\nworkers ${data.runtime_workers} busy ${data.runtime_busy_workers}\nevents ${data.events_rows}\ndeleted ${data.sessions_deleted}\nrss ${data.daemon_rss_bytes}`}
        </Typography>
      </Paper>
    </Stack>
  );
}

function Stat({ title, value }: { title: string; value: string }): React.JSX.Element {
  return (
    <Paper sx={{ p: 2, flex: 1 }}>
      <Typography variant="subtitle2" color="text.secondary">{title}</Typography>
      <Typography variant="h5">{value}</Typography>
    </Paper>
  );
}

function AccountsPage({
  auth,
  onAuth,
}: {
  auth: AdminAuthStatus;
  onAuth: (auth: AdminAuthStatus) => void;
}): React.JSX.Element {
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [productUsers, setProductUsers] = useState<ProductUser[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void Promise.all([adminApi.accounts(), adminApi.productUsers()])
      .then(([accountData, productData]) => {
        setUsers(accountData.accounts);
        setProductUsers(productData.users);
      })
      .catch((err: Error) => setError(err.message));
  }, []);
  if (error) return <Alert severity="error">{error}</Alert>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Accounts</Typography>
      <Typography color="text.secondary">
        This instance is single-user. The owner is created on / during first-run.
        Admin login uses the same account. Extra users, invites, and open
        registration are not available.
      </Typography>
      <AdminPasskeysCard auth={auth} onAuth={onAuth} />
      <Paper sx={{ p: 2 }}>
        <Typography variant="h6" gutterBottom>Owner</Typography>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Account</TableCell>
              <TableCell>Admin role</TableCell>
              <TableCell>Product role</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {users.map((user) => {
              const product = productUsers.find((item) => item.username === user.account);
              return (
                <TableRow key={user.account}>
                  <TableCell>{user.account}</TableCell>
                  <TableCell>{user.role}</TableCell>
                  <TableCell>{product?.role ?? "—"}</TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </Paper>
    </Stack>
  );
}

function PermissionsPage(): React.JSX.Element {
  const [policy, setPolicy] = useState<PermissionPolicy | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void adminApi.permissions().then(setPolicy).catch((err: Error) => setError(err.message));
  }, []);
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!policy) return <Typography>Loading…</Typography>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Permissions</Typography>
      <Typography color="text.secondary">
        This instance is single-user. The account created on / is the owner.
        Extra grants are not available.
      </Typography>
      <Paper sx={{ p: 2 }}>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Account</TableCell>
              <TableCell>Role</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {policy.grants.map((grant) => (
              <TableRow key={grant.account}>
                <TableCell>{grant.account}</TableCell>
                <TableCell>{grant.role}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
    </Stack>
  );
}

function ReleasesPage(): React.JSX.Element {
  const [providers, setProviders] = useState<ProviderRelease[]>([]);
  const [root, setRoot] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const reload = useCallback(async () => {
    const data = await adminApi.providers();
    setProviders(data.providers);
    setRoot(data.catalog_root);
  }, []);
  useEffect(() => {
    void reload().catch((err: Error) => setError(err.message));
  }, [reload]);
  if (error) return <Alert severity="error">{error}</Alert>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Provider releases</Typography>
      <Typography color="text.secondary">
        Signed Catalog versions are installable. Unbound entries need `just provider-sign` then `just provider-publish` into the catalog directory, then refresh.
      </Typography>
      {root && <Alert severity="info">Catalog: {root}</Alert>}
      <Button
        variant="contained"
        onClick={() => {
          void adminApi.refreshCatalog().then((result) => {
            setMessage(`Loaded ${result.external_releases} signed releases`);
            return reload();
          }).catch((err: Error) => setError(err.message));
        }}
      >
        Refresh catalog
      </Button>
      {message && <Alert severity="success">{message}</Alert>}
      <Paper>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Provider</TableCell>
              <TableCell>Version</TableCell>
              <TableCell>State</TableCell>
              <TableCell>Publisher</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {providers.map((provider) => (
              <TableRow key={`${provider.provider_id}:${provider.provider_version}:${provider.artifact_digest ?? "embedded"}`}>
                <TableCell>{provider.provider_id}</TableCell>
                <TableCell>{provider.provider_version}</TableCell>
                <TableCell>{provider.release_state}</TableCell>
                <TableCell>{provider.publisher}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
    </Stack>
  );
}

function SessionsPage(): React.JSX.Element {
  const [sessions, setSessions] = useState<AdminSession[]>([]);
  const [machines, setMachines] = useState<AdminMachine[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void Promise.all([adminApi.sessions(), adminApi.machines()]).then(([sessionData, machineData]) => {
      setSessions(sessionData.sessions);
      setMachines(machineData.machines);
    }).catch((err: Error) => setError(err.message));
  }, []);
  if (error) return <Alert severity="error">{error}</Alert>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Sessions</Typography>
      <Paper>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Title</TableCell>
              <TableCell>Provider</TableCell>
              <TableCell>Machine</TableCell>
              <TableCell>Status</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {sessions.map((session) => (
              <TableRow key={session.id}>
                <TableCell>{session.title || session.id}</TableCell>
                <TableCell>{session.provider}</TableCell>
                <TableCell>{session.machine_id}</TableCell>
                <TableCell>{session.status}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
      <Typography variant="h5">Machines</Typography>
      <Paper>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Name</TableCell>
              <TableCell>Mode</TableCell>
              <TableCell>Status</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {machines.map((machine) => (
              <TableRow key={machine.id}>
                <TableCell>{machine.display_name || machine.id}</TableCell>
                <TableCell>{machine.connection_mode}</TableCell>
                <TableCell>{machine.status}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
    </Stack>
  );
}

function LimitsPage(): React.JSX.Element {
  const [limits, setLimits] = useState<SessionLimits | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void adminApi.sessionLimits().then(setLimits).catch((err: Error) => setError(err.message));
  }, []);
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!limits) return <Typography>Loading…</Typography>;
  const numberOrNull = (value: string): number | null => {
    if (value.trim() === "") return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  };
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Session limits</Typography>
      <Typography color="text.secondary">
        Last N and last time are an OR: an event is kept if it is in the newest N or newer than the time window.
      </Typography>
      <Paper sx={{ p: 2 }}>
        <Stack spacing={2} maxWidth={420}>
          <TextField
            label="Max sessions"
            type="number"
            value={limits.max_sessions ?? ""}
            onChange={(event) => setLimits({ ...limits, max_sessions: numberOrNull(event.target.value) })}
          />
          <TextField
            label="Max retention days"
            type="number"
            value={limits.max_retention_days ?? ""}
            onChange={(event) => setLimits({ ...limits, max_retention_days: numberOrNull(event.target.value) })}
          />
          <TextField
            label="Last N events"
            type="number"
            value={limits.last_n ?? ""}
            onChange={(event) => setLimits({ ...limits, last_n: numberOrNull(event.target.value) })}
          />
          <TextField
            label="Last time (hours)"
            type="number"
            value={limits.last_time_hours ?? ""}
            onChange={(event) => setLimits({ ...limits, last_time_hours: numberOrNull(event.target.value) })}
          />
          <Button
            variant="contained"
            onClick={() => void adminApi.saveSessionLimits(limits).then(setLimits).catch((err: Error) => setError(err.message))}
          >
            Save limits
          </Button>
        </Stack>
      </Paper>
    </Stack>
  );
}
