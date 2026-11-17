const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function bytes(n: number | null | undefined, digits = 1): string {
  if (n == null || !isFinite(n)) return "—";
  let i = 0;
  let v = Math.abs(n);
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : v < 10 ? digits : 0)} ${UNITS[i]}`;
}

export const rate = (n: number | null | undefined) => (n == null ? "—" : `${bytes(n)}/s`);

export const pct = (n: number | null | undefined, digits = 0) =>
  n == null || !isFinite(n) ? "—" : `${n.toFixed(digits)}%`;

export function duration(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d} д ${h} ч`;
  if (h > 0) return `${h} ч ${m} мин`;
  return `${m} мин`;
}

const rtf = new Intl.RelativeTimeFormat("ru", { numeric: "auto" });

export function ago(input: string | number | Date): string {
  const t = typeof input === "number" ? input * (input < 1e12 ? 1000 : 1) : new Date(input).getTime();
  const diff = (t - Date.now()) / 1000;
  const abs = Math.abs(diff);
  if (abs < 60) return rtf.format(Math.round(diff), "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(diff / 86400), "day");
  return rtf.format(Math.round(diff / (86400 * 30)), "month");
}

/** Whether an ISO timestamp is still in the future. */
export const isFuture = (s: string | null | undefined) => !!s && new Date(s).getTime() > Date.now();

/** Seconds elapsed since a unix timestamp (seconds) or ISO string. */
export const secondsSince = (t: number | string) => (Date.now() - (typeof t === "number" ? t * 1000 : new Date(t).getTime())) / 1000;

export const dateTime = (s: string) =>
  new Date(s).toLocaleString("ru", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit", second: "2-digit" });
