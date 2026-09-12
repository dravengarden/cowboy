import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  createTheme,
  Stack,
  Typography,
  useTheme,
} from "@mui/material";
import {
  APP_ICON_GROUPS,
  APP_ICONS,
  appearanceStyle,
  appIcon,
  appIconAppearanceAsset,
  appIconAsset,
  appIconInstallPath,
  currentAppIcon,
  DEFAULT_APP_ICON,
  isNativeIconSurface,
  type NativeAppIconState,
  nativeAppIconState,
  selectAppIcon,
  subscribeAppIcon,
} from "./appIcons";
import { appearancePalette } from "./appearanceThemes";

function StylePreview(
  { id, dark }: { id: string; dark: boolean },
): React.JSX.Element {
  const p = useMemo(
    () => createTheme({ palette: appearancePalette(id, dark) }).palette,
    [id, dark],
  );
  return (
    <Box
      aria-label={`${dark ? "Dark" : "Light"} theme preview`}
      sx={{
        flex: 1,
        minWidth: 0,
        p: 1.5,
        borderRadius: 2,
        bgcolor: p.background.default,
        color: p.text.primary,
        border: "1px solid",
        borderColor: p.divider,
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1}>
        <Box
          component="img"
          src={appIconAppearanceAsset(id, dark)}
          alt=""
          width={28}
          height={28}
          sx={{ borderRadius: 1 }}
        />
        <Typography variant="caption" sx={{ color: p.text.primary }}>
          {dark ? "Dark" : "Light"}
        </Typography>
      </Stack>
      <Box sx={{ mt: 1.5, p: 1, borderRadius: 1, bgcolor: p.background.paper }}>
        <Box
          sx={{
            height: 5,
            width: "75%",
            borderRadius: 1,
            bgcolor: p.text.secondary,
          }}
        />
        <Box
          sx={{
            height: 5,
            width: "50%",
            mt: 0.75,
            borderRadius: 1,
            bgcolor: p.secondary.main,
          }}
        />
      </Box>
      <Box
        sx={{
          mt: 1,
          px: 1,
          py: 0.5,
          textAlign: "center",
          borderRadius: 1,
          bgcolor: p.primary.main,
          color: p.primary.contrastText,
        }}
      >
        <Typography variant="caption" sx={{ color: "inherit" }}>
          Continue
        </Typography>
      </Box>
      <Stack direction="row" useFlexGap flexWrap="wrap" gap={1} sx={{ mt: 1 }}>
        {(["success", "warning", "error"] as const).map((status) => (
          <Stack key={status} direction="row" spacing={0.5} alignItems="center">
            <Box
              sx={{
                width: 7,
                height: 7,
                borderRadius: "50%",
                bgcolor: p[status].main,
              }}
            />
            <Typography variant="caption" sx={{ color: p.text.secondary }}>
              {status === "success"
                ? "Success"
                : status === "warning"
                ? "Warning"
                : "Error"}
            </Typography>
          </Stack>
        ))}
      </Stack>
    </Box>
  );
}

export function AppIconSettings(): React.JSX.Element {
  const dark = useTheme().palette.mode === "dark";
  const selected = useSyncExternalStore(
    subscribeAppIcon,
    currentAppIcon,
    () => DEFAULT_APP_ICON,
  );
  const [expanded, setExpanded] = useState(false),
    [preview, setPreview] = useState(selected);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null),
    [message, setMessage] = useState<string | null>(null);
  const [native, setNative] = useState<NativeAppIconState | null>(null);
  const inNative = isNativeIconSurface();
  useEffect(() => {
    setPreview(selected);
  }, [selected]);
  useEffect(() => {
    if (expanded) {
      void nativeAppIconState().then(setNative).catch(() => setNative(null));
    }
  }, [expanded]);
  const unsupported = inNative &&
    (!native?.supported || !native.available.includes(preview));
  const ios = /iPhone|iPad|iPod/.test(globalThis.navigator?.userAgent ?? "") ||
    (/Mac/.test(globalThis.navigator?.platform ?? "") &&
      globalThis.navigator.maxTouchPoints > 1);
  const apply = async (id: string): Promise<void> => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const themeOnly = inNative &&
        (!native?.supported || !native.available.includes(id));
      await selectAppIcon(id, { themeOnly });
      setPreview(id);
      setMessage(
        themeOnly
          ? "Theme applied. Use the download below for your system icon controls."
          : inNative
          ? "Theme and Home Screen icon updated."
          : "Style saved. Follow the steps below to update your Home Screen icon.",
      );
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not apply this style.",
      );
    } finally {
      setBusy(false);
    }
  };
  const icon = appIcon(preview), style = appearanceStyle(preview);
  return (
    <Stack spacing={1.5} data-app-icon-settings>
      <Stack direction="row" spacing={1.5} alignItems="center">
        <Box
          component="img"
          src={appIconAppearanceAsset(selected, dark)}
          alt="Current Cowboy icon"
          width={48}
          height={48}
          sx={{ borderRadius: 2 }}
        />
        <Box sx={{ flex: 1 }}>
          <Typography variant="body2">Icon &amp; theme</Typography>
          <Typography variant="caption" color="text.secondary">
            {appearanceStyle(selected).name} · {APP_ICONS.length} curated styles
          </Typography>
        </Box>
        <Button onClick={() => setExpanded(!expanded)} aria-expanded={expanded}>
          {expanded ? "Close" : "Choose"}
        </Button>
      </Stack>
      {expanded && (
        <Stack spacing={2}>
          <Box>
            <Stack
              direction="row"
              alignItems="center"
              spacing={1}
              sx={{ mb: 1 }}
            >
              <Typography variant="subtitle2">
                {style.name} · {icon.number}
              </Typography>
              {preview === DEFAULT_APP_ICON && (
                <Chip size="small" label="Official default" />
              )}
            </Stack>
            <Typography variant="caption" color="text.secondary">
              One style for your icon, buttons, accents and surfaces. Your light
              / dark preference stays the same.
            </Typography>
            <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
              <StylePreview id={preview} dark={false} />
              <StylePreview id={preview} dark />
            </Stack>
          </Box>
          {APP_ICON_GROUPS.map((group) => (
            <Box key={group.id}>
              <Typography variant="subtitle2">{group.name}</Typography>
              <Typography variant="caption" color="text.secondary">
                {group.description}
              </Typography>
              <Box
                role="group"
                aria-label={`${group.name} styles`}
                sx={{
                  display: "grid",
                  gridTemplateColumns: "repeat(4, minmax(0, 1fr))",
                  gap: 0.75,
                  mt: 1,
                }}
              >
                {group.styles.map((item) => (
                  <ButtonBase
                    key={item.id}
                    disabled={busy}
                    aria-label={`${item.name}, icon ${appIcon(item.id).number}`}
                    aria-pressed={preview === item.id}
                    onClick={() => {
                      setPreview(item.id);
                      setError(null);
                      setMessage(null);
                    }}
                    sx={{
                      display: "flex",
                      flexDirection: "column",
                      gap: 0.5,
                      p: 0.5,
                      borderRadius: 2,
                      border: "2px solid",
                      borderColor: preview === item.id
                        ? "primary.main"
                        : "transparent",
                      "&.Mui-focusVisible": {
                        outline: "2px solid",
                        outlineColor: "primary.main",
                      },
                    }}
                  >
                    <Box
                      component="img"
                      src={appIconAppearanceAsset(item.id, dark)}
                      alt=""
                      loading="lazy"
                      width={56}
                      height={56}
                      sx={{
                        borderRadius: 1.5,
                        maxWidth: "100%",
                        height: "auto",
                      }}
                    />
                    <Typography variant="caption" sx={{ fontSize: "0.75rem" }}>
                      {item.name}
                      {item.id === selected ? " ✓" : ""}
                    </Typography>
                    <Box
                      aria-hidden
                      sx={{
                        height: 3,
                        width: 24,
                        borderRadius: 1,
                        bgcolor: item.themeColor,
                      }}
                    />
                  </ButtonBase>
                ))}
              </Box>
            </Box>
          ))}
          {error && <Alert severity="error">{error}</Alert>}
          {message && <Alert severity="success">{message}</Alert>}
          <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
            <Button
              variant="contained"
              disabled={busy || preview === selected ||
                !APP_ICONS.some((item) => item.id === preview)}
              onClick={() => {
                void apply(preview);
              }}
            >
              {busy
                ? "Applying…"
                : unsupported
                ? "Use theme"
                : "Use this style"}
            </Button>
            <Button
              disabled={busy || selected === DEFAULT_APP_ICON}
              onClick={() => {
                void apply(DEFAULT_APP_ICON);
              }}
            >
              Restore default
            </Button>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            {inNative
              ? (unsupported
                ? "This shell supports theme changes here. Update the iOS app for Home Screen icon switching, or use your system's icon controls where available."
                : "The system applies the Home Screen icon after you choose a style and may show a confirmation.")
              : ios
              ? "iPhone / iPad: open the installation page in Safari, then Share → Add to Home Screen. Existing Home Screen icons do not change automatically."
              : "Installed Chrome apps may show Review app update in the app menu. Other browsers may require adding the app again."}
          </Typography>
          <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
            {!inNative && (
              <Button
                component="a"
                href={appIconInstallPath(preview)}
                target="_blank"
                rel="noopener"
              >
                Open installation page
              </Button>
            )}
            <Button
              component="a"
              href={appIconAsset(preview, 512)}
              download={`cowboy-${icon.id}.png`}
            >
              Download icon
            </Button>
          </Stack>
        </Stack>
      )}
    </Stack>
  );
}
