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
