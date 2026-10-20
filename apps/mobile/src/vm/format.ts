// Display helpers for addresses and dates (no React, no React Native: unit-tested in Node).
import { wallTime } from "./zoned.ts";

export interface AddrLike {
  name?: string;
  email: string;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const pad = (n: number) => String(n).padStart(2, "0");

/** The name to show for an address: its display name, else the local part of the email. */
export function displayName(addr: AddrLike | undefined | null): string {
  if (!addr) return "Unknown sender";
  const name = addr.name?.trim().replace(/^["']|["']$/g, "");
  if (name) return name;
  const local = addr.email.split("@")[0];
  return local || addr.email;
}

/** First name for compact participant lists ("Ana Lopez" → "Ana"; "support" stays). */
export function shortName(addr: AddrLike): string {
  const name = displayName(addr);
  if (name.includes("@")) return name;
  return name.split(/\s+/)[0] ?? name;
}

/** "Name <email>" or just the email. */
export function formatAddress(addr: AddrLike): string {
  const name = addr.name?.trim();
  return name ? `${name} <${addr.email}>` : addr.email;
}

/**
 * The sender column of a thread row. One participant: full name. Several: first names, the user's own address
 * shown as "me", at most three, then the message count when the thread has more than one message.
 */
export function participantsLabel(
  participants: readonly AddrLike[],
  lastFrom: AddrLike | undefined,
  messageCount: number,
  selfEmails: ReadonlySet<string>,
): string {
  // A single message shows its sender only; longer threads list who took part, the user as "me".
  if (messageCount <= 1 && lastFrom)
    return selfEmails.has(lastFrom.email.toLowerCase()) ? "me" : displayName(lastFrom);
  const list = participants.length > 0 ? participants : lastFrom ? [lastFrom] : [];
  const isSelf = (a: AddrLike) => selfEmails.has(a.email.toLowerCase());
  let label: string;
  if (list.length === 0) label = "Unknown sender";
  else if (list.length === 1) label = isSelf(list[0]!) ? "me" : displayName(list[0]);
  else {
    const names = list.slice(0, 3).map((a) => (isSelf(a) ? "me" : shortName(a)));
    label = names.join(", ") + (list.length > 3 ? ", …" : "");
  }
  return messageCount > 1 ? `${label} (${messageCount})` : label;
}

/**
 * The date column of a list row, in the user's zone: "14:05" today, "Tue" this week, "Oct 3" this year,
 * "2025-10-03" before that.
 */
export function listDate(at: number, now: number, timeZone: string): string {
  const w = wallTime(at, timeZone);
  const n = wallTime(now, timeZone);
  if (w.year === n.year && w.month === n.month && w.day === n.day) return `${pad(w.hour)}:${pad(w.minute)}`;
  if (now - at < 6 * 86_400_000 && now >= at) return DAYS[w.weekday]!;
  if (w.year === n.year) return `${MONTHS[w.month - 1]} ${w.day}`;
  return `${w.year}-${pad(w.month)}-${pad(w.day)}`;
}

/** A full timestamp for message headers: "Tue, Oct 21, 14:05" (year added when not this year). */
export function fullDate(at: number, now: number, timeZone: string): string {
  const w = wallTime(at, timeZone);
  const n = wallTime(now, timeZone);
  const year = w.year === n.year ? "" : ` ${w.year}`;
  return `${DAYS[w.weekday]}, ${MONTHS[w.month - 1]} ${w.day}${year}, ${pad(w.hour)}:${pad(w.minute)}`;
}

/** "99+" for counts at the server cap (`mail.unreadCounts` caps at 100). */
export function countLabel(count: number | undefined): string | null {
  if (!count || count <= 0) return null;
  return count >= 100 ? "99+" : String(count);
}

/** "12 KB", "3.4 MB" (attachments). */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
