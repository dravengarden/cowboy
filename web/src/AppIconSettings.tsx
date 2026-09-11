import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  MenuItem,
  Pagination,
  Select,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  APP_ICON_CHANGED,
  APP_ICONS,
  appIcon,
  appIconAsset,
  appIconInstallPath,
  currentAppIcon,
  DEFAULT_APP_ICON,
  filterAppIcons,
  isNativeIconSurface,
  type NativeAppIconState,
  nativeAppIconState,
  selectAppIcon,
} from "./appIcons";

function subscribe(listener: () => void): () => void {
  globalThis.addEventListener(APP_ICON_CHANGED, listener);
  return () => globalThis.removeEventListener(APP_ICON_CHANGED, listener);
}

export function AppIconSettings(): React.JSX.Element {
  const selected = useSyncExternalStore(
    subscribe,
    currentAppIcon,
    () => DEFAULT_APP_ICON,
  );
  const [expanded, setExpanded] = useState(false);
  const [preview, setPreview] = useState(selected);
  const [query, setQuery] = useState("");
  const [family, setFamily] = useState("all");
  const [tone, setTone] = useState("all");
  const [page, setPage] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
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
  const results = useMemo(() => filterAppIcons({ query, family, tone }), [
    query,
    family,
    tone,
  ]);
  const pages = Math.max(1, Math.ceil(results.length / 24));
  const activePage = Math.min(page, pages);
  const shown = results.slice((activePage - 1) * 24, activePage * 24);
  const icon = appIcon(preview);
  const ios = /iPhone|iPad|iPod/.test(globalThis.navigator?.userAgent ?? "") ||
    (/Mac/.test(globalThis.navigator?.platform ?? "") &&
      globalThis.navigator.maxTouchPoints > 1);
  const apply = async (id: string): Promise<void> => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await selectAppIcon(id);
      setPreview(id);
      setMessage(
        inNative
          ? "Home Screen icon updated."
          : "Icon preference saved for this browser. See the installation steps below for your Home Screen.",
      );
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not change the icon.",
      );
    } finally {
      setBusy(false);
    }
  };
  const unsupported = inNative &&
    (!native?.supported || !native.available.includes(preview));
  return (
    <Stack spacing={1.5} data-app-icon-settings>
      <Stack direction="row" spacing={1.5} alignItems="center">
        <Box
          component="img"
          src={appIconAsset(selected, 96)}
          alt="Current Cowboy icon"
          width={48}
          height={48}
          sx={{ borderRadius: 2 }}
        />
        <Box sx={{ flex: 1 }}>
          <Typography variant="body2">App icon</Typography>
          <Typography variant="caption" color="text.secondary">
            {APP_ICONS.length} colorways · {appIcon(selected).title}
          </Typography>
        </Box>
        <Button onClick={() => setExpanded(!expanded)} aria-expanded={expanded}>
          {expanded ? "Close" : "Choose"}
        </Button>
      </Stack>
      {expanded && (
        <Stack spacing={1.5}>
          <Stack direction="row" spacing={2} alignItems="center">
            <Box
              component="img"
              src={appIconAsset(preview, 180)}
              alt={`Preview: ${icon.title}`}
              width={80}
              height={80}
              sx={{ borderRadius: 3 }}
            />
            <Box>
              <Typography variant="subtitle2">
                {icon.collection === "original" ? "Original " : ""}
                {icon.number} · {icon.title}
              </Typography>
              <Stack direction="row" spacing={0.5} sx={{ mt: 0.5 }}>
                {[icon.crown, icon.brim, icon.background].map((
                  color,
                  index,
                ) => (
                  <Box
                    key={index}
                    title={color}
                    sx={{
                      width: 18,
                      height: 18,
                      bgcolor: color,
                      borderRadius: "50%",
                      border: "1px solid",
                      borderColor: "divider",
                    }}
                  />
                ))}
              </Stack>
              {preview === DEFAULT_APP_ICON && (
                <Chip size="small" label="Default · 54" sx={{ mt: 0.5 }} />
              )}
            </Box>
          </Stack>
          <TextField
            size="small"
            label="Search number, name, or hex color"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setPage(1);
            }}
          />
          <Stack direction="row" spacing={1}>
            <Select
              size="small"
              value={family}
              inputProps={{ "aria-label": "Icon color family" }}
              onChange={(e) => {
                setFamily(e.target.value);
                setPage(1);
              }}
              sx={{ flex: 1 }}
            >
              {[
                ["all", "All colors"],
                ["red", "Red"],
                ["pink", "Pink"],
                ["purple", "Purple"],
                ["blue", "Blue"],
                ["orange", "Orange / brown"],
                ["gold", "Gold / yellow"],
                ["neutral", "Neutral"],
              ].map(([value, label]) => (
                <MenuItem key={value} value={value}>{label}</MenuItem>
              ))}
            </Select>
            <Select
              size="small"
              value={tone}
              inputProps={{ "aria-label": "Icon background" }}
              onChange={(e) => {
                setTone(e.target.value);
                setPage(1);
              }}
              sx={{ flex: 1 }}
            >
              {[["all", "All backgrounds"], ["dark", "Dark"], [
                "medium",
                "Color",
              ], ["light", "Light"]].map(([value, label]) => (
                <MenuItem key={value} value={value}>{label}</MenuItem>
              ))}
            </Select>
          </Stack>
          <Typography variant="caption" color="text.secondary" role="status">
            {results.length} colorways
          </Typography>
          <Box
            role="group"
            aria-label="Icon colorways"
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(64px, 1fr))",
              gap: 1,
            }}
          >
            {shown.map((item) => (
              <ButtonBase
                key={item.id}
                disabled={busy}
                aria-label={`${
                  item.collection === "original" ? "Original " : ""
                }${item.number} ${item.title}`}
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
                  src={appIconAsset(item.id, 96)}
                  alt=""
                  loading="lazy"
                  width={56}
                  height={56}
                  sx={{ borderRadius: 1.5, maxWidth: "100%", height: "auto" }}
                />
                <Typography variant="caption">
                  {item.collection === "original" ? "O" : ""}
                  {item.number}
                  {item.id === selected ? " ✓" : ""}
                </Typography>
              </ButtonBase>
            ))}
          </Box>
          {pages > 1 && (
            <Pagination
              count={pages}
              page={activePage}
              onChange={(_, value) => setPage(value)}
              size="small"
              siblingCount={0}
            />
          )}
          {error && <Alert severity="error">{error}</Alert>}
          {message && <Alert severity="success">{message}</Alert>}
          <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
            <Button
              variant="contained"
              disabled={busy || unsupported || preview === selected}
              onClick={() => {
                void apply(preview);
              }}
            >
              {busy ? "Applying…" : "Use this icon"}
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
                ? (ios
                  ? "This icon needs a native iOS build that includes it. Preview and download are available here."
                  : "Automatic icon changes are unavailable in this native app. Download the icon and use your system's icon controls where supported.")
                : "The system will apply your selection to the native app. It may show a confirmation.")
              : ios
              ? "iPhone / iPad: open the installation page in Safari, then Share → Add to Home Screen. Existing Home Screen icons do not change automatically."
              : "For an installed Chrome app, look for Review app update in its menu. Other browsers may require adding the app again. Browser controls the Home Screen update."}
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
