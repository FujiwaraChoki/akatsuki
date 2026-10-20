// The status bar's echo (First Light, SPEC §3.1.3): after an action the bar says what happened for four seconds
// ("Archived “Offsite agenda”"), so feedback never covers content. Also counts the conversations cleared out of
// the inbox today, for the inbox-zero tally.
import { useSyncExternalStore } from "react";

export const ECHO_MS = 4000;

let current: { text: string; at: number } | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();
const emit = () => {
  for (const l of listeners) l();
};

/** Shows `text` in the status bar for four seconds. */
export function echo(text: string): void {
  current = { text, at: Date.now() };
  clearTimeout(timer);
  timer = setTimeout(() => {
    current = null;
    emit();
  }, ECHO_MS);
  emit();
}

export function useEcho(): string | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current?.text ?? null,
    () => null,
  );
}

const clearedKey = (now: number) => `ak:cleared:${new Date(now).toDateString()}`;

/** Notes `n` conversations leaving the inbox (archive, snooze, trash, move). */
export function noteCleared(n: number, now = Date.now()): void {
  if (n <= 0) return;
  try {
    localStorage.setItem(clearedKey(now), String(clearedToday(now) + n));
  } catch {
    /* storage off */
  }
}

export function clearedToday(now = Date.now()): number {
  try {
    return Number(localStorage.getItem(clearedKey(now)) ?? 0) || 0;
  } catch {
    return 0;
  }
}
