// Snooze and send-later presets (ARCHITECTURE §10: `mail.snooze` takes `until`, `POST /v1/send` takes `sendAt`).
// Times resolve in the user's zone (profiles.timezone, §1.3) and only the ms instant leaves the device.
import { addDays, safeZone, type WallTime, wallTime, zonedInstant } from "./zoned.ts";

export type PresetId =
  | "later_today"
  | "this_evening"
  | "tomorrow"
  | "tomorrow_afternoon"
  | "this_weekend"
  | "next_week"
  | "next_month"
  | "in_one_hour";

export interface TimePreset {
  id: PresetId;
  label: string;
  at: number;
  /** Short description of the resolved time, e.g. "Tue 08:00". */
  hint: string;
}

/** Hour of day the morning presets use. */
export const MORNING_HOUR = 8;
const AFTERNOON_HOUR = 13;
const EVENING_HOUR = 18;

const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number) => String(n).padStart(2, "0");

/** "Tue 08:00" within a week, "Oct 3, 08:00" further out (24-hour clock, zone-local). */
export function describeTime(at: number, now: number, timeZone: string): string {
  const w = wallTime(at, timeZone);
  const time = `${pad(w.hour)}:${pad(w.minute)}`;
  const today = wallTime(now, timeZone);
  if (sameDay(w, today)) return `Today ${time}`;
  if (sameDay(w, addDays(today, 1))) return `Tomorrow ${time}`;
  if (at - now < 6 * 86_400_000) return `${DAY_NAMES[w.weekday]} ${time}`;
  return `${MONTH_NAMES[w.month - 1]} ${w.day}, ${time}`;
}

const sameDay = (a: Pick<WallTime, "year" | "month" | "day">, b: Pick<WallTime, "year" | "month" | "day">) =>
  a.year === b.year && a.month === b.month && a.day === b.day;

function at(day: { year: number; month: number; day: number }, hour: number, tz: string): number {
  return zonedInstant(day.year, day.month, day.day, hour, 0, tz);
}

/**
 * Snooze presets in Superhuman's order. Presets that would land in the past or too close to now are left out
 * ("Later today" after 18:00, "This weekend" on a weekend).
 */
export function snoozePresets(now: number, timeZone: string): TimePreset[] {
  const tz = safeZone(timeZone);
  const today = wallTime(now, tz);
  const out: TimePreset[] = [];
  const push = (id: PresetId, label: string, when: number) => {
    if (when - now >= 15 * 60_000) out.push({ id, label, at: when, hint: describeTime(when, now, tz) });
  };

  // Later today: three hours from now, rounded up to the hour, while that is still before the evening.
  const later = Math.ceil((now + 3 * 3_600_000) / 3_600_000) * 3_600_000;
  if (wallTime(later, tz).hour <= EVENING_HOUR && sameDay(wallTime(later, tz), today))
    push("later_today", "Later today", later);
  else if (today.hour < EVENING_HOUR - 1) push("this_evening", "This evening", at(today, EVENING_HOUR, tz));

  const tomorrow = addDays(today, 1);
  push("tomorrow", "Tomorrow", at(tomorrow, MORNING_HOUR, tz));

  // This weekend: the coming Saturday, unless it is already the weekend.
  if (today.weekday >= 1 && today.weekday <= 5) {
    const sat = addDays(today, 6 - today.weekday);
    push("this_weekend", "This weekend", at(sat, MORNING_HOUR, tz));
  }

  // Next week: the next Monday (a week from today when today is Monday).
  const toMonday = ((8 - today.weekday) % 7 || 7) as number;
  push("next_week", "Next week", at(addDays(today, toMonday), MORNING_HOUR, tz));

  // Next month: same day next month (clamped to the month's length).
  const nm =
    today.month === 12 ? { year: today.year + 1, month: 1 } : { year: today.year, month: today.month + 1 };
  const lastDay = new Date(Date.UTC(nm.year, nm.month, 0)).getUTCDate();
  push("next_month", "Next month", at({ ...nm, day: Math.min(today.day, lastDay) }, MORNING_HOUR, tz));
  return out;
}

/** Send-later presets. */
export function sendLaterPresets(now: number, timeZone: string): TimePreset[] {
  const tz = safeZone(timeZone);
  const today = wallTime(now, tz);
  const tomorrow = addDays(today, 1);
  const presets: TimePreset[] = [];
  const inHour = Math.ceil((now + 3_600_000) / 300_000) * 300_000; // next 5-minute mark after an hour
  presets.push({ id: "in_one_hour", label: "In one hour", at: inHour, hint: describeTime(inHour, now, tz) });
  const morning = at(tomorrow, MORNING_HOUR, tz);
  presets.push({
    id: "tomorrow",
    label: "Tomorrow morning",
    at: morning,
    hint: describeTime(morning, now, tz),
  });
  const afternoon = at(tomorrow, AFTERNOON_HOUR, tz);
  presets.push({
    id: "tomorrow_afternoon",
    label: "Tomorrow afternoon",
    at: afternoon,
    hint: describeTime(afternoon, now, tz),
  });
  const toMonday = ((8 - today.weekday) % 7 || 7) as number;
  const monday = at(addDays(today, toMonday), MORNING_HOUR, tz);
  if (toMonday > 1)
    presets.push({
      id: "next_week",
      label: "Monday morning",
      at: monday,
      hint: describeTime(monday, now, tz),
    });
  return presets;
}

/** One day option of the custom picker. */
export interface DayOption {
  offset: number;
  label: string;
  year: number;
  month: number;
  day: number;
}

/** The next `count` days for the custom picker ("Today", "Tomorrow", "Wed 22", ...). */
export function dayOptions(now: number, timeZone: string, count = 14): DayOption[] {
  const today = wallTime(now, safeZone(timeZone));
  return Array.from({ length: count }, (_, offset) => {
    const d = addDays(today, offset);
    const label =
      offset === 0
        ? "Today"
        : offset === 1
          ? "Tomorrow"
          : `${DAY_NAMES[d.weekday]} ${d.day} ${MONTH_NAMES[d.month - 1]}`;
    return { offset, label, year: d.year, month: d.month, day: d.day };
  });
}

/** Half-hour slots of a day as minutes after midnight. */
export const TIME_SLOTS: readonly number[] = Array.from({ length: 48 }, (_, i) => i * 30);

export function slotLabel(minutes: number): string {
  return `${pad(Math.floor(minutes / 60))}:${pad(minutes % 60)}`;
}

/** The instant of a custom choice, or an error message when it is not in the future. */
export function customTime(
  day: DayOption,
  minutes: number,
  now: number,
  timeZone: string,
): { ok: true; at: number } | { ok: false; error: string } {
  const when = zonedInstant(day.year, day.month, day.day, Math.floor(minutes / 60), minutes % 60, timeZone);
  if (when <= now + 60_000) return { ok: false, error: "Pick a time in the future." };
  return { ok: true, at: when };
}
