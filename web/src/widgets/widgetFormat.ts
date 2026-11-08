// Value formatting and tone colours for widget cells (docs/widgets.md §5.3).

import type { Theme } from "@mui/material";
import type { Cell, Column, Tone } from "../../../contracts/widget-v1.ts";

export function toneColor(theme: Theme, tone: Tone | undefined): string | undefined {
  switch (tone) {
    case "success":
      return theme.palette.success.main;
    case "warning":
      return theme.palette.mode === "dark"
        ? theme.palette.warning.light
        : theme.palette.warning.dark;
    case "error":
      return theme.palette.error.main;
    case "info":
      return theme.palette.info.main;
    case "running":
      return theme.palette.primary.main;
    case "neutral":
      return theme.palette.text.secondary;
    default:
      return undefined;
  }
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

function bytes(value: number): string {
  let amount = value;
  let unit = 0;
  while (Math.abs(amount) >= 1000 && unit < UNITS.length - 1) {
    amount /= 1000;
    unit += 1;
  }
  return `${unit === 0 ? amount : amount.toFixed(amount >= 100 ? 0 : 1)} ${UNITS[unit]}`;
}

function duration(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${Math.round(seconds % 60)}s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** Renders a plain cell value; a value of the wrong shape renders as text. */
export function formatValue(
  value: string | number | boolean | null,
  format: Column["format"],
): string {
  if (value === null) return "—";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (typeof value === "number") {
    switch (format) {
      case "percent":
        return `${Math.round(value * 1000) / 10}%`;
      case "bytes":
        return bytes(value);
      case "duration":
        return duration(value);
      case "date":
        return new Date(value).toLocaleString();
      default:
        return value.toLocaleString();
    }
  }
  if (format === "date") {
    const parsed = Date.parse(value);
    if (!Number.isNaN(parsed)) return new Date(parsed).toLocaleString();
  }
  return value;
}

export type CellParts = { text: string; tone?: Tone; url?: string };

export function cellParts(cell: Cell, format: Column["format"]): CellParts {
  if (cell === null || typeof cell !== "object") return { text: formatValue(cell, format) };
  const parts: CellParts = { text: formatValue(cell.v, format) };
  if (cell.tone !== undefined) parts.tone = cell.tone;
  if (cell.to?.kind === "url") parts.url = cell.to.url;
  return parts;
}

export function columnOf(column: string | Column): Column {
  return typeof column === "string" ? { label: column } : column;
}
