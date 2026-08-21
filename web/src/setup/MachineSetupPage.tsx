import {
  ArrowBackRounded,
  Check,
  Close,
  ContentCopy,
  Settings as SettingsIcon,
  VisibilityOffOutlined,
  VisibilityOutlined,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  CircularProgress,
  Dialog,
  IconButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import { ProductAccountMenu } from "../auth/ProductAccountMenu";
import { ProductPasskeysPanel } from "../auth/ProductPasskeysPanel";
import { useProductAuth } from "../auth/ProductAuthGate";
import { FONT_PRESETS } from "../fonts";
import {
  FONT_SCALE_PRESETS,
  nearestPreset,
  setFontScale,
  setFontVariant,
  useReadingSettings,
} from "../readingSettings";
import { type Mode, useThemeMode } from "../theme";
import { fetchSetupMachines, needsMachineSetup } from "./machineReady";

// Fill these when the public guide and installable skill are published.
const MACHINE_SETUP_DOCS_URL = "";
const MACHINE_SETUP_SKILL_URL = "";

function SetupReference({ href, label }: { href: string; label: string }): React.JSX.Element {
  if (!href) {
    return <Button disabled variant="text">{label} · Coming soon</Button>;
  }
  return (
    <Button href={href} target="_blank" rel="noreferrer" variant="text">
      {label}
    </Button>
  );
}

function CopyBlock({
  label,
  value,
  hint,
  secret = false,
}: {
  label: string;
  value: string;
  hint: string;
  secret?: boolean;
}): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const displayedValue = secret && !revealed ? maskSecret(value) : value;
  return (
    <Stack spacing={0.75}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography variant="subtitle2" sx={{ fontWeight: 650 }}>{label}</Typography>
        <Stack direction="row" spacing={0.25}>
          {secret
            ? (
              <IconButton
                aria-label={revealed ? "Hide enrollment token" : "Show enrollment token"}
                size="small"
                onClick={(): void => setRevealed((value) => !value)}
              >
                {revealed
                  ? <VisibilityOffOutlined fontSize="small" />
                  : <VisibilityOutlined fontSize="small" />}
              </IconButton>
            )
            : null}
          <IconButton
            aria-label={`Copy ${label.toLowerCase()}`}
            size="small"
            onClick={(): void => {
              void navigator.clipboard.writeText(value).then(() => {
                setCopied(true);
                globalThis.setTimeout(() => setCopied(false), 1500);
              });
            }}
          >
            {copied ? <Check fontSize="small" /> : <ContentCopy fontSize="small" />}
          </IconButton>
        </Stack>
      </Stack>
      <Box
        sx={{
          border: 1,
          borderColor: "divider",
          borderRadius: 2,
          bgcolor: "action.hover",
          px: 1.5,
          py: 1.25,
        }}
      >
        <Typography
          component="pre"
          sx={{
            m: 0,
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: "0.8125rem",
            lineHeight: 1.5,
            whiteSpace: "pre-wrap",
            overflowWrap: "anywhere",
          }}
        >
          {displayedValue}
        </Typography>
      </Box>
      <Typography color="text.secondary" sx={{ fontSize: 13 }}>{hint}</Typography>
    </Stack>
  );
}

function maskSecret(value: string): string {
  const visible = Math.min(4, value.length);
  return `${"*".repeat(value.length - visible)}${value.slice(-visible)}`;
}

function Choice({
  active,
  onClick,
  children,
  ariaLabel,
  wide,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  ariaLabel: string;
  wide?: boolean;
}): React.JSX.Element {
  return (
    <ButtonBase
      aria-label={ariaLabel}
      aria-pressed={active}
      onClick={onClick}
      sx={{
        minHeight: 40,
        px: 1.25,
        py: 0.75,
        gridColumn: wide ? "1 / -1" : undefined,
        borderRadius: 1.5,
        border: 1,
        borderColor: active ? "primary.main" : "divider",
        bgcolor: active ? "action.selected" : "background.paper",
        color: active ? "primary.main" : "text.primary",
        fontSize: "0.875rem",
        fontWeight: active ? 700 : 500,
        justifyContent: "center",
        textAlign: "center",
        transition: "background-color 120ms ease, border-color 120ms ease",
        "&:hover": { bgcolor: "action.hover", borderColor: "primary.light" },
      }}
    >
      {children}
    </ButtonBase>
  );
}

function SetupSettings({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const { mode, setMode } = useThemeMode();
  const reading = useReadingSettings();
  const themes: Mode[] = ["system", "light", "dark"];
  return (
    <Dialog
      open={open}
      onClose={onClose}
      fullWidth
      maxWidth="sm"
      scroll="paper"
      slotProps={{
        paper: {
          sx: {
            borderRadius: { xs: 0, sm: 3 },
            m: { xs: 0, sm: 2 },
            maxHeight: { xs: "100%", sm: "calc(100% - 64px)" },
            height: { xs: "100%", sm: "auto" },
          },
        },
      }}
    >
      <Box sx={{ px: 3, pt: 2.5, pb: 4 }}>
        <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 1 }}>
          <Typography sx={{ fontWeight: 700, fontSize: "1.25rem", letterSpacing: -0.4 }}>
            Settings
          </Typography>
          <IconButton aria-label="Close settings" onClick={onClose}>
            <Close />
          </IconButton>
        </Stack>
        <Typography color="text.secondary" sx={{ mb: 3, fontSize: 14 }}>
          Appearance and account only. Session, Provider, and machine settings
          appear after a computer is connected.
        </Typography>
        <Stack spacing={3.5}>
          <Stack spacing={1.25}>
            <Typography variant="overline" color="text.secondary" sx={{ letterSpacing: "0.08em" }}>
              Appearance
            </Typography>
            <Typography variant="subtitle2">Theme</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 0.75 }}>
              {themes.map((choice) => (
                <Choice
                  key={choice}
                  active={mode === choice}
                  onClick={(): void => setMode(choice)}
                  ariaLabel={`${choice} theme`}
                >
                  {choice.charAt(0).toUpperCase() + choice.slice(1)}
                </Choice>
              ))}
            </Box>
            <Typography variant="subtitle2" sx={{ pt: 1 }}>Typeface</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 0.75 }}>
              {FONT_PRESETS.map((preset, index) => (
                <Choice
                  key={preset.id}
                  active={reading.fontVariant === preset.id}
                  onClick={(): void => setFontVariant(preset.id)}
                  ariaLabel={`${preset.label} typeface`}
                  wide={index === FONT_PRESETS.length - 1 && FONT_PRESETS.length % 2 === 1}
                >
                  <Typography component="span" sx={{ fontFamily: preset.stack, fontSize: "0.9rem" }} noWrap>
                    {preset.label}
                  </Typography>
                </Choice>
              ))}
            </Box>
            <Typography variant="subtitle2" sx={{ pt: 1 }}>Size</Typography>
            <Box sx={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 0.75 }}>
              {FONT_SCALE_PRESETS.map((value) => (
                <Choice
                  key={value}
                  active={nearestPreset(reading.fontScale, FONT_SCALE_PRESETS) === value}
                  onClick={(): void => setFontScale(value)}
                  ariaLabel={`${Math.round(value * 100)} percent font size`}
                >
                  {Math.round(value * 100)}%
                </Choice>
              ))}
            </Box>
          </Stack>
          <Stack spacing={1.25}>
            <Typography variant="overline" color="text.secondary" sx={{ letterSpacing: "0.08em" }}>
              Account
            </Typography>
            <ProductPasskeysPanel />
            <ProductAccountMenu />
          </Stack>
        </Stack>
      </Box>
    </Dialog>
  );
}

export function MachineSetupPage(): React.JSX.Element {
  const { me } = useProductAuth();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [displayName, setDisplayName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [waiting, setWaiting] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [issued, setIssued] = useState<{
    token: string;
    machine_id: string;
    display_name: string;
    expires_in_seconds: number;
    expires_at_ms: number;
    origin: string;
  } | null>(null);

  const watchMachines = useCallback((): void => {
    void fetchSetupMachines()
      .then((machines) => {
        if (!needsMachineSetup(machines)) setWaiting(false);
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!issued) return;
    watchMachines();
    const timer = globalThis.setInterval(() => {
      if (Date.now() < issued.expires_at_ms) watchMachines();
    }, 3000);
    return () => globalThis.clearInterval(timer);
  }, [issued, watchMachines]);

  useEffect(() => {
    if (!issued) return;
    setNowMs(Date.now());
    const timer = globalThis.setInterval(() => setNowMs(Date.now()), 1000);
    return () => globalThis.clearInterval(timer);
  }, [issued]);

  const create = (): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    const name = displayName.trim();
    void fetch("/api/machines/enrollment", {
      method: "POST",
      cache: "no-store",
      credentials: "same-origin",
      headers: { accept: "application/json", "content-type": "application/json" },
      body: JSON.stringify(name ? { display_name: name } : {}),
    })
      .then(async (response) => {
        const text = await response.text();
        if (!response.ok) throw new Error(text || response.statusText);
        return text
          ? JSON.parse(text) as {
            token: string;
            machine_id: string;
            display_name: string;
            expires_in_seconds: number;
          }
          : null;
      })
      .then((body) => {
        if (!body?.token) throw new Error("Enrollment token missing");
        setIssued({
          token: body.token,
          machine_id: body.machine_id,
          display_name: body.display_name,
          expires_in_seconds: body.expires_in_seconds,
          expires_at_ms: Date.now() + body.expires_in_seconds * 1000,
          origin: globalThis.location.origin,
        });
        setWaiting(true);
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : "Could not create machine");
      })
      .finally(() => setBusy(false));
  };

  const abandon = (): void => {
    if (busy || !issued) return;
    const token = issued.token;
    setBusy(true);
    setError(null);
    void fetch("/api/machines/enrollment", {
      method: "DELETE",
      cache: "no-store",
      credentials: "same-origin",
      headers: { accept: "application/json", "content-type": "application/json" },
      body: JSON.stringify({ token }),
    })
      .then(async (response) => {
        if (response.ok) return;
        const text = await response.text();
        throw new Error(text || response.statusText);
      })
      .then(() => {
        setIssued(null);
        setWaiting(false);
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : "Could not discard enrollment code");
      })
      .finally(() => setBusy(false));
  };

  const command = issued ? `cowboy register ${issued.origin}` : "";
  const backgroundCommand = issued ? `${command} --background` : "";
  const remainingSeconds = issued
    ? Math.max(0, Math.ceil((issued.expires_at_ms - nowMs) / 1000))
    : 0;
  const expired = issued !== null && remainingSeconds === 0;
  const expiryLabel = `${Math.floor(remainingSeconds / 60)}:${String(remainingSeconds % 60).padStart(2, "0")}`;

  return (
    <Box
      sx={{
        minHeight: "100dvh",
        boxSizing: "border-box",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        px: 3,
        py: 6,
        bgcolor: "background.default",
        color: "text.primary",
        fontFamily: "var(--cowboy-reading-font, inherit)",
      }}
    >
      <IconButton
        aria-label="settings"
        onClick={(): void => setSettingsOpen(true)}
        sx={{ position: "fixed", top: 16, right: 16 }}
      >
        <SettingsIcon />
      </IconButton>
      <SetupSettings open={settingsOpen} onClose={(): void => setSettingsOpen(false)} />
      <Stack spacing={3} sx={{ width: "100%", maxWidth: 420 }}>
        <Box>
          <Typography
            component="p"
            sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}
          >
            cowboy
          </Typography>
          <Typography
            component="h1"
            variant="h5"
            sx={{ fontWeight: 700, mt: 1, letterSpacing: -0.4 }}
          >
            Connect a computer
          </Typography>
          <Typography color="text.secondary" sx={{ mt: 0.75 }}>
            Cowboy runs agents on a machine you register, not in this browser.
            Create a one-time code, then run the command on that computer.
          </Typography>
        </Box>
        {error ? <Alert severity="error">{error}</Alert> : null}
        {issued
          ? (
            <>
              <CopyBlock
                label="On that computer, run"
                value={command}
                hint="Runs in this terminal. Keep it open while you want this computer online."
              />
              <CopyBlock
                label="Or keep it online in the background"
                value={backgroundCommand}
                hint="Installs and starts a per-user background service. Choose one command, then paste the token."
              />
              <CopyBlock
                label="Then paste this token"
                value={issued.token}
                hint={expired ? "This one-time token has expired." : `Shown once. Expires in ${expiryLabel}.`}
                secret
              />
              {expired
                ? (
                  <Alert
                    severity="warning"
                    action={
                      <Button color="warning" disabled={busy} onClick={create}>
                        {busy ? "Creating…" : "Create a new code"}
                      </Button>
                    }
                  >
                    Enrollment code expired. Generate a fresh code before registering this computer.
                  </Alert>
                )
                : waiting
                ? (
                  <Stack direction="row" spacing={1.25} alignItems="center">
                    <CircularProgress size={18} />
                    <Typography color="text.secondary">
                      Waiting for {issued.display_name} to come online…
                    </Typography>
                  </Stack>
                )
                : (
                  <Alert severity="success">Computer connected. Opening Cowboy…</Alert>
                )}
              {waiting || expired
                ? (
                  <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
                    <Button
                      variant="text"
                      color="inherit"
                      disabled={busy}
                      startIcon={<ArrowBackRounded />}
                      onClick={abandon}
                    >
                      {busy ? "Discarding…" : "Back"}
                    </Button>
                    <Typography color="text.secondary" sx={{ fontSize: 13 }}>
                      Discard this code and edit the computer name.
                    </Typography>
                  </Stack>
                )
                : null}
            </>
          )
          : (
            <>
              <TextField
                label="Name this computer"
                value={displayName}
                autoFocus
                placeholder="MacBook, studio PC…"
                helperText="Optional. Cowboy assigns the machine id."
                onChange={(event) => setDisplayName(event.target.value)}
                fullWidth
              />
              <Button
                variant="contained"
                size="large"
                disabled={busy}
                onClick={create}
              >
                {busy ? "Creating…" : "Create code"}
              </Button>
              <Typography color="text.secondary" sx={{ fontSize: 13 }}>
                You will get
                {" "}
                <Box
                  component="span"
                  sx={{
                    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                    fontSize: 13,
                  }}
                >
                  cowboy register {globalThis.location.origin}
                </Box>
                {" "}
                and a one-time token to paste.
              </Typography>
              <Typography color="text.secondary" sx={{ fontSize: 13 }}>
                The default command runs in the current terminal. Add
                {" "}
                <Box
                  component="span"
                  sx={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" }}
                >
                  --background
                </Box>
                {" "}
                to install and start a background service instead.
              </Typography>
              <Typography color="text.secondary" sx={{ fontSize: 13, opacity: 0.8 }}>
                Signed in as {me.account}
              </Typography>
            </>
          )}
        <Stack direction="row" spacing={1} flexWrap="wrap">
          <SetupReference href={MACHINE_SETUP_DOCS_URL} label="Setup guide" />
          <SetupReference href={MACHINE_SETUP_SKILL_URL} label="Setup skill" />
        </Stack>
      </Stack>
    </Box>
  );
}
