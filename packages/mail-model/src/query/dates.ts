// Date resolution for `before:`/`after:` and relative ages (crates/search::date twin, D-094).
import { asciiLower, rustTrim } from "./chars.ts";

// ───────────── dates (crates/search::date) ─────────────

export function zoneOk(timezone: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: timezone });
    return true;
  } catch {
    return false;
  }
}

/** UTC ms of a wall-clock time, valid for every year (Date.UTC maps years 0–99 to 1900–1999). */
function utcMs(y: number, mo: number, d: number, h = 0, mi = 0, s = 0): number {
  const t = new Date(0);
  t.setUTCFullYear(y, mo - 1, d);
  t.setUTCHours(h, mi, s, 0);
  return t.getTime();
}

/** The zone's UTC offset (ms) in effect at instant `ms`. */
function offsetAt(ms: number, timezone: string): number {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: timezone,
    hourCycle: "h23",
    era: "short",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(new Date(ms));
  const get = (t: string) => Number(parts.find((p) => p.type === t)?.value);
  const era = parts.find((p) => p.type === "era")?.value ?? "AD";
  const year = era.startsWith("B") ? 1 - get("year") : get("year");
  const wall = utcMs(year, get("month"), get("day"), get("hour"), get("minute"), get("second"));
  return wall - Math.floor(ms / 1000) * 1000;
}

const isLeap = (y: number) => (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
const DAYS_IN_MONTH = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/** `YYYY-MM-DD` or `YYYY/MM/DD` (month and day 1–2 digits), validated against the calendar. */
export function parseDay(value: string): { y: number; m: number; d: number } | null {
  const sep = value.includes("-") ? "-" : "/";
  const parts = value.split(sep);
  if (parts.length !== 3) return null;
  const [y, m, d] = parts as [string, string, string];
  if (y.length !== 4 || m.length < 1 || m.length > 2 || d.length < 1 || d.length > 2) return null;
  if (!/^[0-9]+$/.test(y + m + d)) return null;
  const [yy, mm, dd] = [Number(y), Number(m), Number(d)];
  if (mm < 1 || mm > 12 || dd < 1) return null;
  const max = mm === 2 && isLeap(yy) ? 29 : DAYS_IN_MONTH[mm - 1]!;
  return dd > max ? null : { y: yy, m: mm, d: dd };
}

/**
 * Midnight of a calendar date in an IANA zone as a UTC ms instant. When midnight falls in a DST gap the first
 * instant after the gap is used, and in a fold the earlier one (jiff's "compatible" rule, as in Rust).
 */
export function zonedMidnight(y: number, m: number, d: number, timezone: string): number {
  const wall = utcMs(y, m, d);
  const DAY = 86_400_000;
  const before = offsetAt(wall - DAY, timezone);
  const after = offsetAt(wall + DAY, timezone);
  const candidates = [...new Set([before, offsetAt(wall, timezone), after])]
    .map((o) => wall - o)
    .filter((t) => wall - offsetAt(t, timezone) === t)
    .sort((a, b) => a - b);
  if (candidates.length > 0) return candidates[0]!;
  return wall - before; // gap: wall time + gap length, i.e. the instant at the pre-transition offset
}

/** The start of the day `value` in `timezone`, as Unix ms (`start_of_day_ms`). */
export function startOfDayMs(
  value: string,
  timezone: string,
): { ok: true; at: number } | { ok: false; error: string } {
  const day = parseDay(value);
  if (!day) return { ok: false, error: `invalid date: ${value}` };
  if (!zoneOk(timezone)) return { ok: false, error: `unknown time zone: ${timezone}` };
  return { ok: true, at: zonedMidnight(day.y, day.m, day.d, timezone) };
}

/** `YYYY-MM-DD` of an instant in `timezone` (UTC if the zone is unknown). */
export function formatDay(atMs: number, timezone: string): string {
  const tz = zoneOk(timezone) ? timezone : "UTC";
  const date = new Date(atMs);
  if (Number.isNaN(date.getTime())) return String(atMs);
  const off = offsetAt(atMs, tz);
  const local = new Date(atMs + off);
  const y = local.getUTCFullYear();
  const pad = (n: number, w: number) => String(n).padStart(w, "0");
  const year = y < 0 ? `-${pad(-y, 4)}` : pad(y, 4);
  return `${year}-${pad(local.getUTCMonth() + 1, 2)}-${pad(local.getUTCDate(), 2)}`;
}

const U32_MAX = 4_294_967_295;

/** `N(d|w|m|y)` → days (`m` = 30, `y` = 365), unit case-insensitive (`relative_days`). */
export function relativeDays(value: string): number | null {
  const c = Array.from(rustTrim(value));
  if (c.length === 0) return null;
  const unit = asciiLower(c[c.length - 1]!);
  const digits = c.slice(0, -1).join("");
  if (!/^\+?[0-9]+$/.test(digits)) return null;
  const n = Number(digits);
  if (n > U32_MAX) return null;
  const mult = ({ d: 1, w: 7, m: 30, y: 365 } as Record<string, number>)[unit];
  if (mult === undefined) return null;
  return n * mult > U32_MAX ? null : n * mult;
}
