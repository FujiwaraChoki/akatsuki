// Toasts: action results, op failures (§5.7: once per opId, with Retry and Dismiss) and Undo (10 s, §5.2).
// A toast with an `id` replaces the one already shown under that id, so a failure keyed by opId never stacks.
// Errors are announced assertively, everything else politely. Timers pause while the pointer or focus is
// inside the toast region.

import type { ShortcutId } from "@akatsuki/shortcuts";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { cx } from "./config.tsx";
import { Kbd } from "./Kbd.tsx";

export type ToastTone = "neutral" | "success" | "error";

export interface ToastAction {
  label: string;
  onAction: () => void;
  /** Key hint shown in the button (e.g. `undo`). The app binds the key itself. */
  shortcut?: ShortcutId;
}

export interface ToastOptions {
  /** Dedupe key (e.g. the opId). Defaults to a fresh id. */
  id?: string;
  title: string;
  description?: string;
  tone?: ToastTone;
  action?: ToastAction;
  /** A second action, e.g. Dismiss next to Retry. */
  secondaryAction?: ToastAction;
  /** Auto-dismiss delay; `null` keeps it until dismissed. Default 5 s (errors: sticky). */
  durationMs?: number | null;
  onDismiss?: () => void;
}

interface ToastState extends ToastOptions {
  id: string;
}

export interface ToastApi {
  /** Shows (or replaces, by id) a toast and returns its id. */
  toast(options: ToastOptions): string;
  dismiss(id: string): void;
  dismissAll(): void;
}

const ToastContext = createContext<ToastApi | null>(null);

/** Default Undo window (§5.2: clients offer Undo for 10 s after any action). */
export const UNDO_TOAST_MS = 10_000;
export const DEFAULT_TOAST_MS = 5_000;

let seq = 0;

export function ToastProvider({ children, max = 3 }: { children: ReactNode; max?: number }) {
  const [toasts, setToasts] = useState<ToastState[]>([]);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const paused = useRef(false);
  const remaining = useRef(new Map<string, { ms: number; startedAt: number }>());
  // The options of every visible toast, so dismiss callbacks run outside state updaters.
  const live = useRef(new Map<string, ToastState>());

  const clearTimer = useCallback((id: string) => {
    const t = timers.current.get(id);
    if (t) clearTimeout(t);
    timers.current.delete(id);
  }, []);

  const dismiss = useCallback(
    (id: string) => {
      clearTimer(id);
      remaining.current.delete(id);
      const t = live.current.get(id);
      if (!t) return;
      live.current.delete(id);
      setToasts([...live.current.values()]);
      t.onDismiss?.();
    },
    [clearTimer],
  );

  const schedule = useCallback(
    (id: string, ms: number) => {
      clearTimer(id);
      remaining.current.set(id, { ms, startedAt: Date.now() });
      if (paused.current) return;
      timers.current.set(
        id,
        setTimeout(() => dismiss(id), ms),
      );
    },
    [clearTimer, dismiss],
  );

  const toast = useCallback(
    (options: ToastOptions) => {
      const id = options.id ?? `toast-${++seq}`;
      const tone = options.tone ?? "neutral";
      const duration =
        options.durationMs === undefined ? (tone === "error" ? null : DEFAULT_TOAST_MS) : options.durationMs;
      const next: ToastState = { ...options, id, tone };
      live.current.set(id, next);
      // Over the limit, the oldest toasts leave (Map order is insertion order; a replaced id keeps its place).
      const evicted: ToastState[] = [];
      for (const old of live.current.values()) {
        if (live.current.size - evicted.length <= max) break;
        evicted.push(old);
      }
      for (const old of evicted) {
        live.current.delete(old.id);
        clearTimer(old.id);
        remaining.current.delete(old.id);
      }
      setToasts([...live.current.values()]);
      for (const old of evicted) old.onDismiss?.();
      if (duration === null) {
        clearTimer(id);
        remaining.current.delete(id);
      } else schedule(id, duration);
      return id;
    },
    [clearTimer, max, schedule],
  );

  const dismissAll = useCallback(() => {
    for (const id of [...timers.current.keys()]) clearTimer(id);
    remaining.current.clear();
    const all = [...live.current.values()];
    live.current.clear();
    setToasts([]);
    for (const t of all) t.onDismiss?.();
  }, [clearTimer]);

  const pause = useCallback(() => {
    if (paused.current) return;
    paused.current = true;
    const now = Date.now();
    for (const [id, r] of remaining.current) {
      clearTimer(id);
      remaining.current.set(id, { ms: Math.max(0, r.ms - (now - r.startedAt)), startedAt: now });
    }
  }, [clearTimer]);

  const resume = useCallback(() => {
    if (!paused.current) return;
    paused.current = false;
    for (const [id, r] of [...remaining.current]) schedule(id, r.ms);
  }, [schedule]);

  useEffect(() => {
    const map = timers.current;
    return () => {
      for (const t of map.values()) clearTimeout(t);
    };
  }, []);

  const api = useMemo<ToastApi>(() => ({ toast, dismiss, dismissAll }), [toast, dismiss, dismissAll]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <section
        className="ak-toasts"
        aria-label="Notifications"
        onPointerEnter={pause}
        onPointerLeave={resume}
        onFocus={pause}
        onBlur={resume}
      >
        {toasts.map((t) => (
          <div
            key={t.id}
            className={cx("ak-toast", `ak-toast--${t.tone}`)}
            role={t.tone === "error" ? "alert" : "status"}
            aria-live={t.tone === "error" ? "assertive" : "polite"}
            aria-atomic="true"
          >
            <div className="ak-toast__text">
              <p className="ak-toast__title">{t.title}</p>
              {t.description && <p className="ak-toast__description">{t.description}</p>}
            </div>
            <div className="ak-toast__actions">
              {[t.action, t.secondaryAction].map(
                (a) =>
                  a && (
                    <button
                      key={a.label}
                      type="button"
                      className="ak-toast__action"
                      onClick={() => {
                        a.onAction();
                        dismiss(t.id);
                      }}
                    >
                      {a.label}
                      {a.shortcut && <Kbd shortcut={a.shortcut} className="ak-toast__kbd" />}
                    </button>
                  ),
              )}
              <button
                type="button"
                className="ak-toast__close"
                aria-label="Dismiss notification"
                onClick={() => dismiss(t.id)}
              >
                <span aria-hidden="true">×</span>
              </button>
            </div>
          </div>
        ))}
      </section>
    </ToastContext.Provider>
  );
}

/** The toast API. Must be inside `<ToastProvider>` (or `<AkatsukiUiProvider>`). */
export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>.");
  return ctx;
}
