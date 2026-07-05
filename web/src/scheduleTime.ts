// Time formatting + quick-preset math for scheduled drafts. Dependency-free
// (the app pulls in no date library — see conventions/ui.md) and China-user
// facing, so the labels are Chinese to match cowboy's existing UI strings
// ("中断", "已等待 X 分钟无响应"). All times are absolute epoch-ms.

const WEEK_CN = ["日", "一", "二", "三", "四", "五", "六"] as const;

function hhmm(d: Date): string {
  return `${String(d.getHours())}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function sameDay(a: Date, b: Date): boolean {
  return a.toDateString() === b.toDateString();
}

// Compact absolute label: "今天 20:00" / "明天 9:00" / "周三 9:00" (within a
// week) / "7月12日 9:00" (further out). `now` is injectable for tests.
export function fireLabel(ms: number, now: number = Date.now()): string {
  const d = new Date(ms);
  const n = new Date(now);
  const tomorrow = new Date(n);
  tomorrow.setDate(n.getDate() + 1);
  if (sameDay(d, n)) return `今天 ${hhmm(d)}`;
  if (sameDay(d, tomorrow)) return `明天 ${hhmm(d)}`;
  const days = (d.getTime() - startOfDay(now)) / 86_400_000;
  if (days >= 0 && days < 7) return `周${WEEK_CN[d.getDay()] ?? ""} ${hhmm(d)}`;
  return `${String(d.getMonth() + 1)}月${String(d.getDate())}日 ${hhmm(d)}`;
}

// Relative countdown: "即将" / "45 分钟后" / "8 小时后" / "3 天后".
export function fireRel(ms: number, now: number = Date.now()): string {
  const s = Math.round((ms - now) / 1000);
  if (s <= 30) return "即将";
  if (s < 3600) return `${String(Math.round(s / 60))} 分钟后`;
  if (s < 86_400) return `${String(Math.round(s / 3600))} 小时后`;
  return `${String(Math.round(s / 86_400))} 天后`;
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

// The next occurrence of a wall-clock time (hour:min) at/after `now` — today if
// still ahead, else tomorrow. Used by the "今晚 20:00" / "明早 9:00" presets.
function nextAt(hour: number, minute: number, now: number = Date.now()): number {
  const d = new Date(now);
  d.setHours(hour, minute, 0, 0);
  if (d.getTime() <= now) d.setDate(d.getDate() + 1);
  return d.getTime();
}

export interface Preset {
  key: string;
  label: string;
  ms: number;
}

// Quick chips offered in the schedule sheet. Computed at open time; a preset in
// the past (e.g. "今晚 20:00" after 20:00) is dropped by the caller via `future`.
export function quickPresets(now: number = Date.now()): Preset[] {
  const inHour = now + 3_600_000;
  const tonight = nextAt(20, 0, now);
  const tmrMorning = nextAt(9, 0, now);
  // "明早" must be strictly tomorrow morning even if it's before 9am today.
  const d = new Date(now);
  const beforeNine = d.getHours() < 9;
  const morning = beforeNine ? tmrMorning + 86_400_000 : tmrMorning;
  const tmrEvening = tonight <= now + 86_400_000 ? tonight + 86_400_000 : tonight;
  return [
    { key: "1h", label: "1 小时后", ms: inHour },
    { key: "tonight", label: "今晚 20:00", ms: tonight },
    { key: "morning", label: "明早 9:00", ms: morning },
    { key: "evening", label: "明晚 20:00", ms: tmrEvening },
  ];
}

// Combine a native <input type="date"> (YYYY-MM-DD) + <input type="time">
// (HH:MM) into epoch-ms in LOCAL time. Returns null if either is blank/invalid.
export function combineDateTime(date: string, time: string): number | null {
  if (!date || !time) return null;
  const [y, mo, da] = date.split("-").map(Number);
  const [h, mi] = time.split(":").map(Number);
  if ([y, mo, da, h, mi].some((v) => v === undefined || Number.isNaN(v))) return null;
  const d = new Date(y ?? 0, (mo ?? 1) - 1, da ?? 1, h ?? 0, mi ?? 0, 0, 0);
  const t = d.getTime();
  return Number.isNaN(t) ? null : t;
}

// Split an epoch-ms into the local {date, time} strings the native inputs want.
export function splitDateTime(ms: number): { date: string; time: string } {
  const d = new Date(ms);
  const p = (n: number): string => String(n).padStart(2, "0");
  return {
    date: `${String(d.getFullYear())}-${p(d.getMonth() + 1)}-${p(d.getDate())}`,
    time: `${p(d.getHours())}:${p(d.getMinutes())}`,
  };
}
