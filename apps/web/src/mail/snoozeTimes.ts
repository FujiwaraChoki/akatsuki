// Snooze and reminder times, resolved in the user's time zone (profiles.timezone, §1.3). Only ms instants go
// to the server (`mail.snooze`, `mail.setReminder`).

export interface ZonedParts {
  y: number;
  m: number; // 1-12
  d: number;
  h: number;
  min: number;
  /** 0 = Sunday. */
  weekday: number;
}

const WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const fmtCache = new Map<string, Intl.DateTimeFormat>();

function partsFormat(tz: string): Intl.DateTimeFormat {
  let f = fmtCache.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      weekday: "short",
    });
    fmtCache.set(tz, f);
  }
  return f;
}

export function zonedParts(ms: number, tz: string): ZonedParts {
  const p = Object.fromEntries(
    partsFormat(tz)
      .formatToParts(ms)
      .map((x) => [x.type, x.value]),
  );
  return {
    y: Number(p.year),
    m: Number(p.month),
    d: Number(p.day),
    h: Number(p.hour) % 24,
    min: Number(p.minute),
    weekday: WEEKDAYS.indexOf(String(p.weekday).toLowerCase().slice(0, 3)),
  };
}

/** The UTC instant of a wall-clock time in `tz` (the earlier one when a DST change repeats it). */
export function wallToUtc(y: number, m: number, d: number, h: number, min: number, tz: string): number {
  const wall = Date.UTC(y, m - 1, d, h, min);
  const offsetAt = (t: number) => {
    const p = zonedParts(t, tz);
    return Date.UTC(p.y, p.m - 1, p.d, p.h, p.min) - (t - (t % 60_000));
  };
  let t = wall - offsetAt(wall);
  t = wall - offsetAt(t);
  return t;
}

/** Adds days to a calendar date (handles month and year ends). */
function addDays(p: { y: number; m: number; d: number }, days: number) {
  const dt = new Date(Date.UTC(p.y, p.m - 1, p.d + days));
  return { y: dt.getUTCFullYear(), m: dt.getUTCMonth() + 1, d: dt.getUTCDate() };
}

export const MORNING_HOUR = 8;

export interface SnoozePreset {
  id: "later" | "tomorrow" | "weekend" | "next_week" | "month";
  label: string;
  at: number;
}

/** The preset list for `now` in `tz`. "Later today" only before 18:00; "This weekend" only on weekdays. */
export function snoozePresets(now: number, tz: string): SnoozePreset[] {
  const p = zonedParts(now, tz);
  const at = (date: { y: number; m: number; d: number }, h = MORNING_HOUR) =>
    wallToUtc(date.y, date.m, date.d, h, 0, tz);
  const out: SnoozePreset[] = [];
  if (p.h < 18) {
    const later = now + 3 * 3_600_000;
    out.push({ id: "later", label: "Later today", at: later - (later % (15 * 60_000)) });
  }
  out.push({ id: "tomorrow", label: "Tomorrow", at: at(addDays(p, 1)) });
  if (p.weekday >= 1 && p.weekday <= 5)
    out.push({ id: "weekend", label: "This weekend", at: at(addDays(p, 6 - p.weekday)) });
  const toMonday = (8 - p.weekday) % 7 || 7;
  out.push({ id: "next_week", label: "Next week", at: at(addDays(p, toMonday)) });
  const month = new Date(Date.UTC(p.y, p.m, Math.min(p.d, 28)));
  out.push({
    id: "month",
    label: "In a month",
    at: at({ y: month.getUTCFullYear(), m: month.getUTCMonth() + 1, d: month.getUTCDate() }),
  });
  return out;
}

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/** "nov 3", "3 nov", "november 3rd at 2pm": the next such date (this year, or next year once it passed). */
function parseMonthDate(
  s: string,
  today: { y: number; m: number; d: number },
): { date: { y: number; m: number; d: number }; time?: string } | null {
  const a = /^([a-z]{3,9})\.? (\d{1,2})(?:st|nd|rd|th)?(?:,? (\d{4}))?(?: (?:at )?(.+))?$/.exec(s);
  const b = /^(\d{1,2})(?:st|nd|rd|th)? (?:of )?([a-z]{3,9})\.?(?:,? (\d{4}))?(?: (?:at )?(.+))?$/.exec(s);
  const [monthWord, day, year, time] = a ? [a[1]!, a[2]!, a[3], a[4]] : b ? [b[2]!, b[1]!, b[3], b[4]] : [];
  if (!monthWord || !day) return null;
  const m = MONTHS.indexOf(monthWord.slice(0, 3)) + 1;
  const d = Number(day);
  if (m === 0 || d < 1 || d > 31) return null;
  let y = year ? Number(year) : today.y;
  if (!year && (m < today.m || (m === today.m && d < today.d))) y++;
  return { date: { y, m, d }, ...(time && { time }) };
}

function parseClock(s: string): { h: number; min: number } | null {
  const m = /^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/.exec(s.trim().toLowerCase());
  if (!m) return null;
  let h = Number(m[1]);
  const min = m[2] ? Number(m[2]) : 0;
  if (min > 59) return null;
  if (m[3]) {
    if (h < 1 || h > 12) return null;
    if (m[3] === "pm" && h !== 12) h += 12;
    if (m[3] === "am" && h === 12) h = 0;
  } else if (h > 23) return null;
  return { h, min };
}

/**
 * Parses a typed snooze time: "3h", "2d", "1w", "45m"; "tonight"; "tomorrow [9am]"; a weekday ("fri 3pm");
 * a date ("2026-11-02 [9:30]"). Returns `null` when unrecognised or not in the future.
 */
export function parseSnoozeInput(input: string, now: number, tz: string): number | null {
  const s = input
    .trim()
    .toLowerCase()
    .replace(/\s+/g, " ")
    .replace(/^(?:on|at|until|till|by) /, "")
    .replace(/\b(an?|one) (minute|hour|day|week|month)\b/, "1 $2");
  if (!s) return null;
  const rel = /^(?:in )?(\d{1,3}) ?(m|min|mins|minutes?|h|hrs?|hours?|d|days?|w|weeks?|months?)$/.exec(s);
  if (rel) {
    const n = Number(rel[1]);
    const unit = rel[2]!;
    if (unit.startsWith("mo")) {
      const p = zonedParts(now, tz);
      const t = new Date(Date.UTC(p.y, p.m - 1 + n, Math.min(p.d, 28)));
      return n > 0
        ? wallToUtc(t.getUTCFullYear(), t.getUTCMonth() + 1, t.getUTCDate(), p.h, p.min, tz)
        : null;
    }
    const u = unit[0];
    const ms = u === "m" ? 60_000 : u === "h" ? 3_600_000 : u === "d" ? 86_400_000 : 7 * 86_400_000;
    return n > 0 ? now + n * ms : null;
  }
  const p = zonedParts(now, tz);
  const future = (t: number) => (t > now ? t : null);
  const at = (date: { y: number; m: number; d: number }, h: number, min = 0) =>
    wallToUtc(date.y, date.m, date.d, h, min, tz);
  /** Today at h:min, or tomorrow when that has passed. */
  const nextAt = (h: number, min = 0) => {
    const t = at(p, h, min);
    return t > now ? t : at(addDays(p, 1), h, min);
  };
  const named: Record<string, () => number | null> = {
    tonight: () => future(at(p, 19)),
    "this evening": () => nextAt(19),
    evening: () => nextAt(19),
    "end of day": () => nextAt(17),
    eod: () => nextAt(17),
    "this afternoon": () => nextAt(13),
    afternoon: () => nextAt(13),
    noon: () => nextAt(12),
    morning: () => nextAt(MORNING_HOUR),
    "tomorrow morning": () => at(addDays(p, 1), MORNING_HOUR),
    "tomorrow afternoon": () => at(addDays(p, 1), 13),
    "tomorrow evening": () => at(addDays(p, 1), 19),
    "this weekend": () => future(at(addDays(p, (6 - p.weekday + 7) % 7), MORNING_HOUR)),
    weekend: () => future(at(addDays(p, (6 - p.weekday + 7) % 7 || 7), MORNING_HOUR)),
    "next week": () => at(addDays(p, (8 - p.weekday) % 7 || 7), MORNING_HOUR),
    "next month": () => {
      const t = new Date(Date.UTC(p.y, p.m, 1));
      return at({ y: t.getUTCFullYear(), m: t.getUTCMonth() + 1, d: 1 }, MORNING_HOUR);
    },
  };
  const hit = named[s];
  if (hit) return hit();
  const nextWd = /^next (sun|mon|tue|wed|thu|fri|sat)\w*(?: (?:at )?(.+))?$/.exec(s);
  if (nextWd) {
    const clock = nextWd[2] ? parseClock(nextWd[2]) : { h: MORNING_HOUR, min: 0 };
    if (!clock) return null;
    const wd = WEEKDAYS.indexOf(nextWd[1]!);
    const days = ((wd - p.weekday + 7) % 7 || 7) + (wd > p.weekday ? 7 : 0);
    return at(addDays(p, days), clock.h, clock.min);
  }
  const monthDate = parseMonthDate(s, p);
  if (monthDate) {
    const clock = monthDate.time ? parseClock(monthDate.time) : { h: MORNING_HOUR, min: 0 };
    if (!clock) return null;
    return future(at(monthDate.date, clock.h, clock.min));
  }
  const dayAndTime =
    /^(today|tomorrow|tmrw|sun\w*|mon\w*|tue\w*|wed\w*|thu\w*|fri\w*|sat\w*|\d{4}-\d{2}-\d{2})(?: (?:at )?(.+))?$/.exec(
      s,
    );
  if (!dayAndTime) {
    const clock = parseClock(s);
    if (!clock) return null;
    const today = wallToUtc(p.y, p.m, p.d, clock.h, clock.min, tz);
    if (today > now) return today;
    const t = addDays(p, 1);
    return wallToUtc(t.y, t.m, t.d, clock.h, clock.min, tz);
  }
  const clock = dayAndTime[2] ? parseClock(dayAndTime[2]) : { h: MORNING_HOUR, min: 0 };
  if (!clock) return null;
  const word = dayAndTime[1]!;
  let date: { y: number; m: number; d: number };
  if (word === "today") date = p;
  else if (word === "tomorrow" || word === "tmrw") date = addDays(p, 1);
  else if (/^\d{4}-/.test(word)) {
    const [y, m, d] = word.split("-").map(Number) as [number, number, number];
    if (m < 1 || m > 12 || d < 1 || d > 31) return null;
    date = { y, m, d };
  } else {
    const wd = WEEKDAYS.indexOf(word.slice(0, 3));
    if (wd === -1) return null;
    date = addDays(p, (wd - p.weekday + 7) % 7 || 7);
  }
  return future(wallToUtc(date.y, date.m, date.d, clock.h, clock.min, tz));
}
