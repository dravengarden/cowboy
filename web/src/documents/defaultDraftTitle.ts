/** Capture device-local time once at creation; never change a title while typing. */
export function defaultDraftTitle(now = new Date()): string {
  const pad = (value: number): string => String(value).padStart(2, "0");
  return `Draft ${now.getFullYear()}-${pad(now.getMonth() + 1)}-${
    pad(now.getDate())
  } ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;
}
