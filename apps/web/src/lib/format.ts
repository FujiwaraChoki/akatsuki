// Display formatting: dates in the user's time zone (profiles.timezone, §1.3), sender lists, sizes.
import type { ThreadRow } from "../backend/types.ts";

const cache = new Map<string, Intl.DateTimeFormat>();
function fmt(tz: string, opts: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${tz}|${JSON.stringify(opts)}`;
  let f = cache.get(key);
  if (!f) {
    try {
      f = new Intl.DateTimeFormat(undefined, { ...opts, timeZone: tz });
    } catch {
      f = new Intl.DateTimeFormat(undefined, opts);
    }
    cache.set(key, f);
  }
  return f;
}

function dayKey(ms: number, tz: string): string {
  return fmt(tz, { year: "numeric", month: "2-digit", day: "2-digit" }).format(ms);
}

/** List date: "14:05" today, "Oct 3" this year, "Oct 3, 2025" before. */
export function listDate(ms: number, now: number, tz: string): string {
  if (dayKey(ms, tz) === dayKey(now, tz)) return fmt(tz, { hour: "numeric", minute: "2-digit" }).format(ms);
  const sameYear = fmt(tz, { year: "numeric" }).format(ms) === fmt(tz, { year: "numeric" }).format(now);
  return fmt(
    tz,
    sameYear ? { month: "short", day: "numeric" } : { month: "short", day: "numeric", year: "numeric" },
  ).format(ms);
}

/** Thread view date: "Tue, Oct 3, 14:05". */
export function fullDate(ms: number, tz: string): string {
  return fmt(tz, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(ms);
}

/** A message's time in the reader: "10:56 AM" today, "Tue 10:56 AM" this week, then "Oct 3" / "Oct 3, 2025". */
export function messageWhen(ms: number, now: number, tz: string): string {
  const time = fmt(tz, { hour: "numeric", minute: "2-digit" }).format(ms);
  if (dayKey(ms, tz) === dayKey(now, tz)) return time;
  if (now - ms < 6 * 86_400_000 && ms <= now) return `${fmt(tz, { weekday: "short" }).format(ms)} ${time}`;
  return listDate(ms, now, tz);
}

/** Picker time: "18:00" today, "Wed 08:00" this week, "Oct 30 08:00" later (mono, SPEC §3.7). */
export function pickerWhen(ms: number, now: number, tz: string): string {
  const time = fmt(tz, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(ms);
  if (dayKey(ms, tz) === dayKey(now, tz)) return time;
  if (ms - now < 6.5 * 86_400_000) return `${fmt(tz, { weekday: "short" }).format(ms)} ${time}`;
  return `${fmt(tz, { month: "short", day: "numeric" }).format(ms)} ${time}`;
}

/**
 * The sender column: the other people first ("Mara Okafor" alone, "Mara, Jun" when several), "me" last for
 * the account's own addresses, at most three names.
 */
export function sendersLabel(
  row: Pick<ThreadRow, "participants" | "lastFrom">,
  myEmails: ReadonlySet<string>,
): string {
  const people = row.lastFrom ? [row.lastFrom, ...row.participants] : row.participants;
  const others: { name?: string; email: string }[] = [];
  let me = false;
  for (const p of people) {
    if (myEmails.has(p.email.toLowerCase())) me = true;
    else if (!others.some((o) => o.email === p.email)) others.push(p);
  }
  const names =
    others.length === 1
      ? [displayName(others[0])]
      : others.map((p) => firstName(p.name) || p.email.split("@")[0] || p.email);
  const shown = [...new Set(names)].slice(0, me ? 2 : 3);
  if (me) shown.push("me");
  return shown.join(", ") || "(no sender)";
}

export function displayName(p: { name?: string; email: string } | undefined): string {
  if (!p) return "(unknown sender)";
  return p.name?.trim() || p.email;
}

function firstName(name: string | undefined): string {
  const n = name?.trim();
  if (!n) return "";
  return n.includes(" ") && !n.includes(",") ? n.split(/\s+/)[0]! : n;
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** "99+" above the server cap, like `Badge`. */
export function countLabel(n: number): string {
  return n >= 100 ? "99+" : String(n);
}

/** `nowAt` for `search.threads`, rounded down to the minute so the query cache stays warm (§11.1). */
export function minuteNow(now: number = Date.now()): number {
  return now - (now % 60_000);
}
