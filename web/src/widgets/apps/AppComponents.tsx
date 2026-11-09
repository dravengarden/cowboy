// Cowboy components an app renders through `@cowboy/ui` (docs/widget-apps.md
// §6). Every prop is untrusted data from the app: each component reads only
// the props it knows, coerces them to closed sets or bounded values, and
// renders plain text — never HTML, Markdown or URLs. Events return sanitized
// payloads to the app.

import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Divider,
  FormControlLabel,
  LinearProgress,
  MenuItem,
  Paper,
  Slider,
  Stack,
  Switch,
  Tab,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import { AppChart } from "./AppChart";
import type { AppElement } from "./appTree";

export type Dispatch = (id: number, name: string, payload: unknown) => void;

const str = (value: unknown, max = 200): string =>
  typeof value === "string" ? value.slice(0, max) : typeof value === "number" ? String(value) : "";
const num = (value: unknown, min: number, max: number, fallback: number): number =>
  typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, value))
    : fallback;
const oneOf = <T extends string>(value: unknown, allowed: readonly T[], fallback: T): T =>
  allowed.includes(value as T) ? (value as T) : fallback;
const handles = (node: AppElement, name: string): boolean =>
  Array.isArray(node.props.__handlers) && node.props.__handlers.includes(name);

const GAP = { none: 0, xs: 0.5, sm: 1, md: 1.5, lg: 2.5 } as const;
const TONES = ["neutral", "info", "success", "warning", "error"] as const;
const toneColor = (tone: string): "default" | "info" | "success" | "warning" | "error" =>
  tone === "neutral" ? "default" : (tone as "info" | "success" | "warning" | "error");

/** A text input that stays responsive while the app echoes its value back. */
function AppTextField({
  node,
  dispatch,
}: {
  node: AppElement;
  dispatch: Dispatch;
}): React.JSX.Element {
  const value = str(node.props["value"], 8000);
  const [local, setLocal] = useState(value);
  // While the person types, the field's own text wins: echoes from the app
  // can arrive late and out of order. The app's value is adopted on blur.
  const focused = useRef(false);
  useEffect(() => {
    if (!focused.current) setLocal(value);
  }, [value]);
  return (
    <TextField
      size="small"
      fullWidth
      label={str(node.props["label"]) || undefined}
      placeholder={str(node.props["placeholder"]) || undefined}
      multiline={node.props["multiline"] === true}
      minRows={node.props["multiline"] === true ? 2 : undefined}
      value={local}
      onFocus={(): void => {
        focused.current = true;
      }}
      onBlur={(): void => {
        focused.current = false;
        setLocal(value);
      }}
      onChange={(event): void => {
        const next = event.target.value.slice(0, 8000);
        setLocal(next);
        if (handles(node, "onChange")) dispatch(node.id, "onChange", { value: next });
      }}
    />
  );
}

/** A slider that keeps up with key repeat and drags while the app echoes
 *  its value back: each step must start from the last step, not from an
 *  echo still in flight. The app's value is adopted on blur. */
function AppSlider({ node, dispatch }: { node: AppElement; dispatch: Dispatch }): React.JSX.Element {
  const p = node.props;
  const value = num(p["value"], -1e9, 1e9, 0);
  const [local, setLocal] = useState(value);
  const focused = useRef(false);
  useEffect(() => {
    if (!focused.current) setLocal(value);
  }, [value]);
  return (
    <Slider
      size="small"
      value={local}
      min={num(p["min"], -1e9, 1e9, 0)}
      max={num(p["max"], -1e9, 1e9, 100)}
      step={num(p["step"], 0, 1e9, 1) || null}
      onFocus={(): void => {
        focused.current = true;
      }}
      onBlur={(): void => {
        focused.current = false;
        setLocal(value);
      }}
      onChange={(_, next): void => {
        const single = Array.isArray(next) ? (next[0] ?? 0) : next;
        setLocal(single);
        if (handles(node, "onChange")) dispatch(node.id, "onChange", { value: single });
      }}
    />
  );
}

function options(value: unknown): { value: string; label: string }[] {
  if (!Array.isArray(value)) return [];
  return value.slice(0, 100).map((option) =>
    typeof option === "object" && option !== null
      ? {
          value: str((option as Record<string, unknown>)["value"]),
          label: str((option as Record<string, unknown>)["label"]),
        }
      : { value: str(option), label: str(option) },
  );
}

function AppTable({ node }: { node: AppElement }): React.JSX.Element {
  const columns = Array.isArray(node.props["columns"])
    ? node.props["columns"].slice(0, 12).map((c) => str(c))
    : [];
  const rows = Array.isArray(node.props["rows"])
    ? (node.props["rows"] as unknown[]).slice(0, 200)
    : [];
  return (
    <Box component="table" sx={{ borderCollapse: "collapse", width: "100%", fontSize: "0.875rem" }}>
      <thead>
        <tr>
          {columns.map((column, c) => (
            <Box
              component="th"
              key={c}
              sx={{
                textAlign: "left",
                p: 0.75,
                borderBottom: 1,
                borderColor: "divider",
                color: "text.secondary",
              }}
            >
              {column}
            </Box>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, r) => (
          <tr key={r}>
            {(Array.isArray(row) ? row.slice(0, 12) : []).map((cell, c) => (
              <Box
                component="td"
                key={c}
                sx={{
                  p: 0.75,
                  borderBottom: 1,
                  borderColor: "divider",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {str(cell, 500)}
              </Box>
            ))}
          </tr>
        ))}
      </tbody>
    </Box>
  );
}

function containers(node: AppElement, children: ReactNode): React.JSX.Element | null {
  const p = node.props;
  switch (node.type) {
    case "Stack":
      return (
        <Stack
          direction={oneOf(p["direction"], ["row", "column"] as const, "column")}
          gap={GAP[oneOf(p["gap"], ["none", "xs", "sm", "md", "lg"] as const, "sm")]}
          flexWrap={p["wrap"] === true ? "wrap" : "nowrap"}
          alignItems={oneOf(
            p["align"],
            ["stretch", "flex-start", "center", "flex-end"] as const,
            "stretch",
          )}
          sx={{ minWidth: 0 }}
        >
          {children}
        </Stack>
      );
    case "Box":
      return (
        <Box
          sx={{
            p: GAP[oneOf(p["padding"], ["none", "xs", "sm", "md", "lg"] as const, "none")],
            minWidth: 0,
          }}
        >
          {children}
        </Box>
      );
    case "Grid":
      return (
        <Box
          sx={{
            display: "grid",
            gap: 1,
            gridTemplateColumns: `repeat(auto-fill, minmax(min(${num(p["minColumn"], 80, 600, 180)}px, 100%), 1fr))`,
          }}
        >
          {children}
        </Box>
      );
    case "Card":
      return (
        <Paper variant="outlined" sx={{ p: 1.5, display: "grid", gap: 1, minWidth: 0 }}>
          {str(p["title"]) ? (
            <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
              {str(p["title"])}
            </Typography>
          ) : null}
          {children}
        </Paper>
      );
    case "Alert":
      return (
        <Alert
          severity={oneOf(p["severity"], ["info", "success", "warning", "error"] as const, "info")}
          variant="outlined"
        >
          {children}
        </Alert>
      );
    case "Text":
      return (
        <Typography
          variant={oneOf(p["variant"], ["body2", "caption"] as const, "body2")}
          color={p["tone"] === "secondary" ? "text.secondary" : undefined}
          sx={{
            fontWeight: p["bold"] === true ? 600 : undefined,
            overflowWrap: "anywhere",
            whiteSpace: "pre-wrap",
          }}
        >
          {children}
        </Typography>
      );
    case "Heading":
      return (
        <Typography
          variant={p["level"] === 1 ? "h6" : "subtitle1"}
          sx={{ fontWeight: 600, overflowWrap: "anywhere" }}
        >
          {children}
        </Typography>
      );
    default:
      return null;
  }
}

function controls(node: AppElement, dispatch: Dispatch, label: string): React.JSX.Element | null {
  const p = node.props;
  const on = (name: string, payload: unknown): void => {
    if (handles(node, name)) dispatch(node.id, name, payload);
  };
  switch (node.type) {
    case "Button":
    case "IconButton":
      return (
        <Box>
          <Button
            size="small"
            variant={oneOf(p["variant"], ["contained", "outlined", "text"] as const, "outlined")}
            disabled={p["disabled"] === true}
            onClick={(): void => on("onPress", {})}
            sx={{ textTransform: "none", minHeight: { xs: 44, sm: 32 } }}
          >
            {label || str(p["label"]) || "Button"}
          </Button>
        </Box>
      );
    case "TextField":
      return <AppTextField node={node} dispatch={dispatch} />;
    case "Select":
      return (
        <TextField
          select
          size="small"
          label={str(p["label"]) || undefined}
          value={str(p["value"])}
          onChange={(e): void => on("onChange", { value: str(e.target.value) })}
        >
          {options(p["options"]).map((option) => (
            <MenuItem key={option.value} value={option.value}>
              {option.label}
            </MenuItem>
          ))}
        </TextField>
      );
    case "Checkbox":
    case "Switch": {
      const Control = node.type === "Checkbox" ? Checkbox : Switch;
      return (
        <FormControlLabel
          control={
            <Control
              checked={p["checked"] === true}
              onChange={(e): void => on("onChange", { checked: e.target.checked })}
            />
          }
          label={str(p["label"])}
        />
      );
    }
    case "Slider":
      return <AppSlider node={node} dispatch={dispatch} />;
    case "Tabs":
      return (
        <Tabs
          value={str(p["value"]) || false}
          onChange={(_, value): void => on("onChange", { value: str(value) })}
          variant="scrollable"
        >
          {options(p["options"]).map((option) => (
            <Tab
              key={option.value}
              value={option.value}
              label={option.label}
              sx={{ textTransform: "none" }}
            />
          ))}
        </Tabs>
      );
    default:
      return null;
  }
}

function leaves(node: AppElement): React.JSX.Element | null {
  const p = node.props;
  switch (node.type) {
    case "Divider":
      return <Divider />;
    case "Chip":
    case "Badge":
      return (
        <Box>
          <Chip
            size="small"
            label={str(p["label"])}
            color={toneColor(oneOf(p["tone"], TONES, "neutral"))}
            variant="outlined"
          />
        </Box>
      );
    case "Progress":
      return (
        <LinearProgress
          variant={typeof p["value"] === "number" ? "determinate" : "indeterminate"}
          value={num(p["value"], 0, 1, 0) * 100}
          sx={{ borderRadius: 999 }}
        />
      );
    case "Table":
      return <AppTable node={node} />;
    case "List":
      return (
        <Box component="ul" sx={{ m: 0, pl: 3 }}>
          {(Array.isArray(p["items"]) ? p["items"].slice(0, 500) : []).map((item, i) => (
            <Typography component="li" variant="body2" key={i}>
              {str(item, 500)}
            </Typography>
          ))}
        </Box>
      );
    case "Chart":
      return <AppChart props={p} />;
    case "Image":
    case "Tooltip":
      return (
        <Typography variant="caption" color="text.secondary">
          {str(p["alt"] ?? p["title"])}
        </Typography>
      );
    default:
      return null;
  }
}

/** Renders one element; `children` are its rendered children, `label` their plain text. */
export function renderAppElement(
  node: AppElement,
  children: ReactNode,
  label: string,
  dispatch: Dispatch,
): React.JSX.Element | null {
  return containers(node, children) ?? controls(node, dispatch, label) ?? leaves(node);
}
