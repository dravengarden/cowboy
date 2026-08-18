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
  Checkbox,
  Container,
  Drawer,
  FormControl,
  FormControlLabel,
  InputLabel,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  MenuItem,
  Paper,
  Select,
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
  type AdminRole,
  type AdminSession,
  type AdminUser,
  type PermissionPolicy,
  type ProductUser,
  type ProviderRelease,
  type RegistrationMode,
  type RegistrationPolicy,
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
  return (
    <Box sx={{ minHeight: "100vh", display: "grid", placeItems: "center", p: 3 }}>
      <Paper sx={{ p: 4, width: "100%", maxWidth: 420 }}>
        <Stack spacing={2}>
          <Typography variant="h5">Cowboy Admin</Typography>
          <Typography color="text.secondary">
            {bootstrap
              ? "Create the first owner account (12+ character password). This page is a separate admin site and is not the session UI."
              : "Sign in to the admin console. Admin sessions last 12 hours."}
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          <TextField label="Account" value={account} onChange={(event) => setAccount(event.target.value)} autoComplete="username" />
          <TextField
            label="Password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete={bootstrap ? "new-password" : "current-password"}
          />
          <Button
            variant="contained"
            onClick={() => {
              const submit = bootstrap ? adminApi.bootstrap : adminApi.login;
              void submit(account, password).then(onAuthed).catch((err: Error) => setError(err.message));
            }}
          >
            {bootstrap ? "Create owner" : "Sign in"}
          </Button>
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
        <Stat title="Registration" value={data.registration.accepts_registration ? data.registration.mode : "closed"} />
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
  const [policy, setPolicy] = useState<RegistrationPolicy | null>(null);
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [productUsers, setProductUsers] = useState<ProductUser[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [productError, setProductError] = useState<string | null>(null);
  const [issued, setIssued] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [mode, setMode] = useState<RegistrationMode>("disabled");
  const [name, setName] = useState("");
  const [newAccount, setNewAccount] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [productAccount, setProductAccount] = useState("");
  const [productPassword, setProductPassword] = useState("");
  const [productRole, setProductRole] = useState<AdminRole>("operator");
  const [passwordDrafts, setPasswordDrafts] = useState<Record<string, string>>({});
  const canGrantOwner = auth.role === "owner";
  const canSetPassword = auth.role === "owner";
  const reload = useCallback(async () => {
    const [next, accountData] = await Promise.all([
      adminApi.registration(),
      adminApi.accounts(),
    ]);
    setPolicy(next);
    setEnabled(next.enabled);
    setMode(next.mode);
    setUsers(accountData.accounts);
  }, []);
  const reloadProductUsers = useCallback(async () => {
    try {
      const productData = await adminApi.productUsers();
      setProductUsers(productData.users);
      setProductError(null);
    } catch (err) {
      setProductUsers([]);
      setProductError(err instanceof Error ? err.message : String(err));
    }
  }, []);
  useEffect(() => {
    void reload().catch((err: Error) => setError(err.message));
    void reloadProductUsers();
  }, [reload, reloadProductUsers]);
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!policy) return <Typography>Loading…</Typography>;
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Accounts</Typography>
      <Typography color="text.secondary">
        This admin site has its own login and is not the session PWA login. Create admin
        operators here. / is login-only until a product user exists. Public signup is a
        separate Matrix-style switch below.
      </Typography>
      <AdminPasskeysCard auth={auth} onAuth={onAuth} />
      <Paper sx={{ p: 2 }}>
        <Typography variant="h6" gutterBottom>Admin users</Typography>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ mb: 2 }}>
          <TextField label="Account" value={newAccount} onChange={(event) => setNewAccount(event.target.value)} size="small" />
          <TextField label="Password" type="password" value={newPassword} onChange={(event) => setNewPassword(event.target.value)} size="small" />
          <Button
            variant="outlined"
            onClick={() => {
              void adminApi.createAccount(newAccount, newPassword).then(() => {
                setNewAccount("");
                setNewPassword("");
                return reload();
              }).catch((err: Error) => setError(err.message));
            }}
          >
            Add operator
          </Button>
        </Stack>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Account</TableCell>
              <TableCell>Role</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {users.map((user) => (
              <TableRow key={user.account}>
                <TableCell>{user.account}</TableCell>
                <TableCell>{user.role}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
      <Paper sx={{ p: 2 }}>
        <Typography variant="h6" gutterBottom>Product users</Typography>
        <Typography color="text.secondary" sx={{ mb: 2 }}>
          This is not the session PWA login. Product users sign in on /. / is login-only
          until a product user exists.
        </Typography>
        {productError && <Alert severity="error" sx={{ mb: 2 }}>{productError}</Alert>}
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ mb: 2 }}>
          <TextField
            label="Account"
            value={productAccount}
            onChange={(event) => setProductAccount(event.target.value)}
            size="small"
          />
          <TextField
            label="Password"
            type="password"
            value={productPassword}
            onChange={(event) => setProductPassword(event.target.value)}
            size="small"
          />
          <FormControl size="small" sx={{ minWidth: 140 }}>
            <InputLabel>Role</InputLabel>
            <Select
              label="Role"
              value={productRole}
              onChange={(event) => setProductRole(event.target.value as AdminRole)}
            >
              <MenuItem value="operator">Operator</MenuItem>
              <MenuItem value="viewer">Viewer</MenuItem>
              {canGrantOwner && <MenuItem value="owner">Owner</MenuItem>}
            </Select>
          </FormControl>
          <Button
            variant="outlined"
            onClick={() => {
              void adminApi.createProductUser(productAccount, productPassword, productRole).then(() => {
                setProductAccount("");
                setProductPassword("");
                setProductRole("operator");
                return reloadProductUsers();
              }).catch((err: Error) => setProductError(err.message));
            }}
          >
            Create user
          </Button>
        </Stack>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Account</TableCell>
              <TableCell>Role</TableCell>
              <TableCell>Status</TableCell>
              {canSetPassword && <TableCell>Password</TableCell>}
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {productUsers.map((user) => (
              <TableRow key={user.id}>
                <TableCell>{user.username}</TableCell>
                <TableCell>{user.role}</TableCell>
                <TableCell>{user.disabled_at_ms == null ? "active" : "disabled"}</TableCell>
                {canSetPassword && (
                  <TableCell>
                    <Stack direction="row" spacing={1}>
                      <TextField
                        label="New password"
                        type="password"
                        size="small"
                        value={passwordDrafts[user.id] ?? ""}
                        onChange={(event) =>
                          setPasswordDrafts((drafts) => ({
                            ...drafts,
                            [user.id]: event.target.value,
                          }))}
                      />
                      <Button
                        onClick={() => {
                          void adminApi.setProductUserPassword(
                            user.id,
                            passwordDrafts[user.id] ?? "",
                          ).then(() => {
                            setPasswordDrafts((drafts) => ({ ...drafts, [user.id]: "" }));
                            setProductError(null);
                          }).catch((err: Error) => setProductError(err.message));
                        }}
                      >
                        Set password
                      </Button>
                    </Stack>
                  </TableCell>
                )}
                <TableCell>
                  <Button
                    disabled={user.disabled_at_ms != null}
                    onClick={() =>
                      void adminApi.disableProductUser(user.id).then(() => reloadProductUsers()).catch(
                        (err: Error) => setProductError(err.message),
                      )}
                  >
                    Disable
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
      <Typography variant="h5">End-user registration</Typography>
      <Typography color="text.secondary">
        Matrix-style service switch. The controller decides whether public registration is closed, token-gated, or open.
      </Typography>
      <Paper sx={{ p: 2 }}>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={2} alignItems="center">
          <FormControlLabel
            control={<Checkbox checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />}
            label="Enable registration"
          />
          <FormControl sx={{ minWidth: 180 }}>
            <InputLabel>Mode</InputLabel>
            <Select label="Mode" value={mode} onChange={(event) => setMode(event.target.value as RegistrationMode)}>
              <MenuItem value="disabled">Disabled</MenuItem>
              <MenuItem value="token">Registration token</MenuItem>
              <MenuItem value="open">Open</MenuItem>
            </Select>
          </FormControl>
          <Button
            variant="contained"
            onClick={() => void adminApi.saveRegistration(enabled, mode).then(reload).catch((err: Error) => setError(err.message))}
          >
            Save
          </Button>
        </Stack>
      </Paper>
      <Paper sx={{ p: 2 }}>
        <Typography variant="h6" gutterBottom>Invite tokens</Typography>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ mb: 2 }}>
          <TextField label="Name" value={name} onChange={(event) => setName(event.target.value)} size="small" />
          <Button
            variant="outlined"
            onClick={() => {
              void adminApi.issueToken(name, 3, 86_400).then((created) => {
                setIssued(created.token);
                setName("");
                return reload();
              }).catch((err: Error) => setError(err.message));
            }}
          >
            Issue token
          </Button>
        </Stack>
        {issued && <Alert severity="info">Copy now; it is not shown again: {issued}</Alert>}
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Name</TableCell>
              <TableCell>Prefix</TableCell>
              <TableCell>Uses</TableCell>
              <TableCell>State</TableCell>
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {policy.tokens.map((token) => (
              <TableRow key={token.id}>
                <TableCell>{token.name}</TableCell>
                <TableCell>{token.token_prefix}</TableCell>
                <TableCell>{token.uses_count}/{token.uses_allowed ?? "∞"}</TableCell>
                <TableCell>{token.disabled ? "disabled" : "active"}</TableCell>
                <TableCell>
                  <Button
                    disabled={token.disabled}
                    onClick={() => void adminApi.disableToken(token.id).then(reload).catch((err: Error) => setError(err.message))}
                  >
                    Disable
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </Paper>
    </Stack>
  );
}

function PermissionsPage(): React.JSX.Element {
  const [policy, setPolicy] = useState<PermissionPolicy | null>(null);
  const [account, setAccount] = useState("");
  const [role, setRole] = useState<AdminRole>("operator");
  const [error, setError] = useState<string | null>(null);
  const reload = useCallback(async () => {
    setPolicy(await adminApi.permissions());
  }, []);
  useEffect(() => {
    void reload().catch((err: Error) => setError(err.message));
  }, [reload]);
  if (error) return <Alert severity="error">{error}</Alert>;
  if (!policy) return <Typography>Loading…</Typography>;
  const save = (next: PermissionPolicy): void => {
    void adminApi.savePermissions(next).then(setPolicy).catch((err: Error) => setError(err.message));
  };
  return (
    <Stack spacing={2}>
      <Typography variant="h4">Permissions</Typography>
      <Typography color="text.secondary">
        Service-owned roles. Future accounts inherit the default role unless a grant overrides it.
      </Typography>
      <Paper sx={{ p: 2 }}>
        <FormControl sx={{ minWidth: 200 }}>
          <InputLabel>Default role</InputLabel>
          <Select
            label="Default role"
            value={policy.default_role}
            onChange={(event) => save({ ...policy, default_role: event.target.value as AdminRole })}
          >
            <MenuItem value="owner">Owner</MenuItem>
            <MenuItem value="operator">Operator</MenuItem>
            <MenuItem value="viewer">Viewer</MenuItem>
          </Select>
        </FormControl>
      </Paper>
      <Paper sx={{ p: 2 }}>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1} sx={{ mb: 2 }}>
          <TextField label="Account" value={account} onChange={(event) => setAccount(event.target.value)} size="small" />
          <Select size="small" value={role} onChange={(event) => setRole(event.target.value as AdminRole)}>
            <MenuItem value="owner">Owner</MenuItem>
            <MenuItem value="operator">Operator</MenuItem>
            <MenuItem value="viewer">Viewer</MenuItem>
          </Select>
          <Button
            variant="contained"
            onClick={() => {
              if (!account.trim()) return;
              save({
                ...policy,
                grants: [...policy.grants.filter((grant) => grant.account !== account.trim()), {
                  account: account.trim(),
                  role,
                }],
              });
              setAccount("");
            }}
          >
            Grant
          </Button>
        </Stack>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Account</TableCell>
              <TableCell>Role</TableCell>
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {policy.grants.map((grant) => (
              <TableRow key={grant.account}>
                <TableCell>{grant.account}</TableCell>
                <TableCell>{grant.role}</TableCell>
                <TableCell>
                  <Button onClick={() => save({
                    ...policy,
                    grants: policy.grants.filter((item) => item.account !== grant.account),
                  })}>
                    Remove
                  </Button>
                </TableCell>
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
