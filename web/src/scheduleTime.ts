// Time formatting + quick-preset math for scheduled drafts. Dependency-free
// (the app pulls in no date library — see conventions/ui.md). All times are
// absolute epoch-ms.

const WEEK_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;
const MONTH_SHORT = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

function hhmm(d: Date): string {
  return `${String(d.getHours())}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function sameDay(a: Date, b: Date): boolean {
  return a.toDateString() === b.toDateString();
}

// Compact absolute label: "Today 20:00" / "Tomorrow 9:00" / "Wed 9:00" (within
// a week) / "Jul 12 9:00" (further out). `now` is injectable for tests.
export function fireLabel(ms: number, now: number = Date.now()): string {
  const d = new Date(ms);
  const n = new Date(now);
  const tomorrow = new Date(n);
  tomorrow.setDate(n.getDate() + 1);
  if (sameDay(d, n)) return `Today ${hhmm(d)}`;
  if (sameDay(d, tomorrow)) return `Tomorrow ${hhmm(d)}`;
  const days = (d.getTime() - startOfDay(now)) / 86_400_000;
  if (days >= 0 && days < 7) return `${WEEK_SHORT[d.getDay()] ?? ""} ${hhmm(d)}`;
  return `${MONTH_SHORT[d.getMonth()] ?? ""} ${String(d.getDate())} ${hhmm(d)}`;
}

// Relative countdown: "Soon" / "in 45m" / "in 8h" / "in 3d".
export function fireRel(ms: number, now: number = Date.now()): string {
  const s = Math.round((ms - now) / 1000);
  if (s <= 30) return "Soon";
  if (s < 3600) return `in ${String(Math.round(s / 60))}m`;
  if (s < 86_400) return `in ${String(Math.round(s / 3600))}h`;
  return `in ${String(Math.round(s / 86_400))}d`;
}

export type ScheduleQuickPresetId = "15m" | "1h" | "3h" | "tomorrow-9";

export interface ScheduleQuickPreset {
  readonly id: ScheduleQuickPresetId;
  readonly label: string;
  readonly fireAtMs: number;
}

export const SCHEDULE_QUICK_PRESET_LABELS: readonly {
  readonly id: ScheduleQuickPresetId;
  readonly label: string;
}[] = [
  { id: "15m", label: "+15m" },
  { id: "1h", label: "+1h" },
  { id: "3h", label: "+3h" },
  { id: "tomorrow-9", label: "Tomorrow 9:00" },
];

/** Compute a preset's unambiguous target epoch-ms at activation time. */
export function computeSchedulePreset(
  id: ScheduleQuickPresetId,
  now: number = Date.now(),
): number {
  const rounded = (minutes: number): number =>
    Math.ceil((now + minutes * 60_000) / 60_000) * 60_000;
  switch (id) {
    case "15m":
      return rounded(15);
    case "1h":
      return rounded(60);
    case "3h":
      return rounded(180);
    case "tomorrow-9": {
      const tomorrow9 = new Date(now);
      tomorrow9.setDate(tomorrow9.getDate() + 1);
      tomorrow9.setHours(9, 0, 0, 0);
      return tomorrow9.getTime();
    }
  }
}

/** One-click / keyboard-friendly future targets for the schedule sheet. */
export function scheduleQuickPresets(now: number = Date.now()): ScheduleQuickPreset[] {
  return SCHEDULE_QUICK_PRESET_LABELS.map((entry) => ({
    ...entry,
    fireAtMs: computeSchedulePreset(entry.id, now),
  }));
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

// Parse a native <input type="datetime-local"> value ("YYYY-MM-DDTHH:MM") into
// epoch-ms in LOCAL time. Returns null if blank/invalid.
export function parseDtLocal(value: string): number | null {
  if (!value) return null;
  const [date, time] = value.split("T");
  if (!date || !time) return null;
  const [y, mo, da] = date.split("-").map(Number);
  const [h, mi] = time.split(":").map(Number);
  if ([y, mo, da, h, mi].some((v) => v === undefined || Number.isNaN(v))) return null;
  const d = new Date(y ?? 0, (mo ?? 1) - 1, da ?? 1, h ?? 0, mi ?? 0, 0, 0);
  const t = d.getTime();
  return Number.isNaN(t) ? null : t;
}

// Format an epoch-ms as the local "YYYY-MM-DDTHH:MM" a datetime-local input wants.
export function toDtLocal(ms: number): string {
  const d = new Date(ms);
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${String(d.getFullYear())}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
