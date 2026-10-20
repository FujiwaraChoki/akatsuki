// Wall-clock time in an IANA zone (ARCHITECTURE §1.3: "tomorrow 8am" resolves in profiles.timezone on the
// client; only ms instants reach the server). Pure, Intl-only, so it runs in Hermes, Node and the browser.

export interface WallTime {
  year: number;
  /** 1–12 */
  month: number;
  /** 1–31 */
  day: number;
  hour: number;
  minute: number;
  /** 0 = Sunday … 6 = Saturday */
  weekday: number;
}

const WEEKDAYS: Record<string, number> = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };
const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(timeZone: string): Intl.DateTimeFormat {
  let f = formatters.get(timeZone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      weekday: "short",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
    formatters.set(timeZone, f);
  }
  return f;
}

/** A usable zone: the given one when Intl knows it, otherwise UTC. */
export function safeZone(timeZone: string | undefined | null): string {
  if (!timeZone) return "UTC";
  try {
    formatter(timeZone);
    return timeZone;
  } catch {
    return "UTC";
  }
}

/** The wall-clock fields of instant `at` in `timeZone`. */
export function wallTime(at: number, timeZone: string): WallTime {
  const parts = formatter(safeZone(timeZone)).formatToParts(new Date(at));
  const get = (t: string) => Number(parts.find((p) => p.type === t)?.value);
  const wd = parts.find((p) => p.type === "weekday")?.value ?? "Sun";
  return {
    year: get("year"),
    month: get("month"),
    day: get("day"),
    hour: get("hour") % 24,
    minute: get("minute"),
    weekday: WEEKDAYS[wd] ?? 0,
  };
}

function utcOf(y: number, mo: number, d: number, h: number, mi: number): number {
  const t = new Date(0);
  t.setUTCFullYear(y, mo - 1, d);
  t.setUTCHours(h, mi, 0, 0);
  return t.getTime();
}

/** The zone's UTC offset in ms at instant `at`. */
export function offsetAt(at: number, timeZone: string): number {
  const w = wallTime(at, timeZone);
  // Minute precision: every zone in use today has whole-minute offsets.
  return utcOf(w.year, w.month, w.day, w.hour, w.minute) - Math.floor(at / 60_000) * 60_000;
}

/**
 * The instant of a wall-clock time in `timeZone`. Day overflow is normalised (day 32 → next month). In a DST gap
 * the time is moved forward by the gap; in a fold the earlier instant wins.
 */
export function zonedInstant(
  year: number,
  month: number,
  day: number,
  hour: number,
  minute: number,
  timeZone: string,
): number {
  const tz = safeZone(timeZone);
  const wall = utcOf(year, month, day, hour, minute);
  const H = 3_600_000;
  const offsets = [
    ...new Set([offsetAt(wall - 24 * H, tz), offsetAt(wall, tz), offsetAt(wall + 24 * H, tz)]),
  ];
  const exact = offsets
    .map((o) => wall - o)
    .filter((t) => wall - offsetAt(t, tz) === t)
    .sort((a, b) => a - b);
  if (exact.length > 0) return exact[0]!;
  // Gap: use the offset in effect before the transition, which lands after the gap.
  return wall - offsetAt(wall - 24 * H, tz);
}

/** Adds `days` calendar days to a wall date (month/year roll over). */
export function addDays(w: Pick<WallTime, "year" | "month" | "day">, days: number) {
  const t = new Date(Date.UTC(2000, 0, 1));
  t.setUTCFullYear(w.year, w.month - 1, w.day + days);
  return {
    year: t.getUTCFullYear(),
    month: t.getUTCMonth() + 1,
    day: t.getUTCDate(),
    weekday: t.getUTCDay(),
  };
}
