// `Chart` for apps (docs/widget-apps.md §6): a small, bounded SVG renderer
// owned by Cowboy. Data are numbers from the app; at most 2,000 points across
// at most 8 series, drawn as lines, bars or areas scaled to the container.

import { useTheme } from "@mui/material";

const MAX_POINTS = 2000;
const MAX_SERIES = 8;
const W = 600;
const H = 220;
const PAD = 24;

type Series = { key: string; label: string };

// Props are untrusted JSON: read only primitives, never call String() or
// Number() on an object (its own `toString` could throw on the main thread).
const text = (value: unknown, max = 200): string =>
  typeof value === "string" ? value.slice(0, max) : "";
const finite = (value: unknown): number | null =>
  typeof value === "number" && Number.isFinite(value) ? value : null;
const field = (row: unknown, key: string): unknown =>
  typeof row === "object" && row !== null && !Array.isArray(row)
    ? (row as Record<string, unknown>)[key]
    : undefined;

export function AppChart({
  props,
}: {
  props: Readonly<Record<string, unknown>>;
}): React.JSX.Element {
  const theme = useTheme();
  const palette = [
    theme.palette.primary.main,
    theme.palette.success.main,
    theme.palette.warning.main,
    theme.palette.error.main,
    theme.palette.info.main,
    theme.palette.secondary.main,
    theme.palette.text.secondary,
    theme.palette.primary.light,
  ];
  const kind = props["type"] === "bar" || props["type"] === "area" ? props["type"] : "line";
  const series: Series[] = (
    Array.isArray(props["series"]) ? props["series"] : [{ key: "y", label: "" }]
  )
    .slice(0, MAX_SERIES)
    .map(
      (s): Series =>
        typeof s === "string"
          ? { key: s, label: s }
          : { key: text(field(s, "key")), label: text(field(s, "label")) },
    )
    .filter((s) => s.key !== "");
  const perSeries = Math.max(1, Math.floor(MAX_POINTS / Math.max(1, series.length)));
  const data: unknown[] = (Array.isArray(props["data"]) ? props["data"] : []).slice(0, perSeries);
  const values = data.flatMap((row) =>
    series.map((s) => finite(field(row, s.key))).filter((v): v is number => v !== null),
  );
  const min = Math.min(0, ...values);
  const max = Math.max(1, ...values);
  const x = (i: number): number =>
    PAD + (data.length <= 1 ? 0 : (i * (W - 2 * PAD)) / (data.length - 1));
  const y = (v: number): number => H - PAD - ((v - min) / (max - min || 1)) * (H - 2 * PAD);
  const band = (W - 2 * PAD) / Math.max(1, data.length) / Math.max(1, series.length);
  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={text(props["title"]) || "Chart"}
      // Wide rows keep the aspect ratio but never grow past a readable height.
      style={{ width: "100%", height: "auto", maxHeight: 260, display: "block" }}
    >
      <line x1={PAD} y1={y(0)} x2={W - PAD} y2={y(0)} stroke={theme.palette.divider} />
      {series.map((s, si) => {
        const color = palette[si % palette.length];
        const points = data.map((row, i) => [x(i), y(finite(field(row, s.key)) ?? 0)] as const);
        if (kind === "bar") {
          return points.map(([, py], i) => (
            <rect
              key={`${si}-${i}`}
              x={PAD + i * band * series.length + si * band}
              y={Math.min(py, y(0))}
              width={Math.max(1, band - 1)}
              height={Math.abs(y(0) - py)}
              fill={color}
            />
          ));
        }
        const path = points
          .map(([px, py], i) => `${i === 0 ? "M" : "L"}${px.toFixed(1)},${py.toFixed(1)}`)
          .join("");
        return kind === "area" ? (
          <path
            key={si}
            d={`${path}L${x(data.length - 1)},${y(0)}L${x(0)},${y(0)}Z`}
            fill={color}
            fillOpacity={0.25}
            stroke={color}
          />
        ) : (
          <path key={si} d={path} fill="none" stroke={color} strokeWidth={2} />
        );
      })}
    </svg>
  );
}
