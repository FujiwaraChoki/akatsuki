// A tiny toast queue (undo after actions, send status, op failures). Toasts with the same id replace each other,
// so a failure keyed by opId shows once (§5.7).
import { useSyncExternalStore } from "react";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: string;
  text: string;
  tone?: "default" | "error";
  actions?: ToastAction[];
  /** ms until it hides; 0 = until dismissed. */
  durationMs?: number;
}

type Listener = () => void;
let toasts: Toast[] = [];
const listeners = new Set<Listener>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();

function emit() {
  for (const l of listeners) l();
}

export function showToast(t: Toast): void {
  // At most one info toast (a new one replaces it) and two errors above it (design spec §3.9, §4.8).
  const kept = toasts.filter((x) => x.id !== t.id && (t.tone === "error" || x.tone === "error"));
  const errors = [...kept.filter((x) => x.tone === "error"), ...(t.tone === "error" ? [t] : [])].slice(-2);
  const info = t.tone === "error" ? kept.filter((x) => x.tone !== "error").slice(-1) : [t];
  const next = [...errors, ...info];
  for (const x of toasts) {
    if (next.includes(x)) continue;
    const timer = timers.get(x.id);
    if (timer) clearTimeout(timer);
    timers.delete(x.id);
  }
  toasts = next;
  const old = timers.get(t.id);
  if (old) clearTimeout(old);
  const ms = t.durationMs ?? 4000;
  if (ms > 0)
    timers.set(
      t.id,
      setTimeout(() => dismissToast(t.id), ms),
    );
  emit();
}

export function dismissToast(id: string): void {
  const timer = timers.get(id);
  if (timer) clearTimeout(timer);
  timers.delete(id);
  const next = toasts.filter((t) => t.id !== id);
  if (next.length !== toasts.length) {
    toasts = next;
    emit();
  }
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => toasts,
    () => toasts,
  );
}
