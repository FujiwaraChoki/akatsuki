// Small shared helpers for the vanilla controllers.

export const $ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document) =>
  root.querySelector<T>(sel);
export const $$ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document) =>
  Array.from(root.querySelectorAll<T>(sel));

export const reducedMotion = (): boolean =>
  typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

export const isMac = (): boolean => document.documentElement.dataset.platform === "mac";

export function isEditable(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  const tag = t.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (t as HTMLInputElement).type;
    return !["button", "checkbox", "radio", "submit", "reset", "range", "color"].includes(type);
  }
  return false;
}

/** "mod" is ⌘ on macOS and Ctrl elsewhere. */
export const isMod = (e: KeyboardEvent): boolean =>
  isMac() ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;

const NAMED: Record<string, string> = {
  Enter: "enter",
  Escape: "escape",
  Esc: "escape",
  Tab: "tab",
  " ": "space",
  Backspace: "backspace",
  Delete: "delete",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

/** The event as a key-map combo string, e.g. "shift+h", "cmd+k", "#", "enter". */
export function comboOf(e: KeyboardEvent): string {
  let key = NAMED[e.key] ?? (e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase());
  if (/^Key[A-Z]$/.test(e.code) && (e.altKey || e.ctrlKey || e.metaKey)) key = e.code.slice(3).toLowerCase();
  const symbol = key.length === 1 && !/[a-z0-9]/.test(key);
  const mods: string[] = [];
  if (e.metaKey) mods.push("cmd");
  if (e.ctrlKey) mods.push("ctrl");
  if (e.altKey) mods.push("alt");
  if (e.shiftKey && !symbol) mods.push("shift");
  return [...mods, key].join("+");
}

/** Keycap text for the HUD. */
export function capOf(e: KeyboardEvent): string {
  const named: Record<string, string> = {
    Enter: "↵",
    Escape: "Esc",
    Tab: "Tab",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    " ": "Space",
  };
  let k = named[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key);
  if (e.shiftKey && /^[A-Z]$/.test(k)) k = `⇧${k}`;
  if (e.shiftKey && e.key === "Tab") k = "⇧Tab";
  if (e.metaKey || e.ctrlKey) k = `${isMac() ? "⌘" : "Ctrl "}${k}`;
  return k;
}

// ───────────── timers that pause while the tab is hidden ─────────────

interface Timer {
  fn: () => void;
  remaining: number;
  start: number;
  id: number;
  done: boolean;
}
const timers = new Set<Timer>();

function arm(t: Timer) {
  t.start = performance.now();
  t.id = window.setTimeout(
    () => {
      t.done = true;
      timers.delete(t);
      t.fn();
    },
    Math.max(0, t.remaining),
  );
}

export function after(ms: number, fn: () => void): () => void {
  const t: Timer = { fn, remaining: ms, start: 0, id: 0, done: false };
  timers.add(t);
  if (!document.hidden) arm(t);
  return () => {
    clearTimeout(t.id);
    timers.delete(t);
  };
}

let visBound = false;
export function bindVisibility() {
  if (visBound) return;
  visBound = true;
  document.addEventListener("visibilitychange", () => {
    const now = performance.now();
    for (const t of timers) {
      if (document.hidden) {
        clearTimeout(t.id);
        t.remaining -= now - t.start;
      } else arm(t);
    }
  });
}

// ───────────── easing ─────────────

/** cubic-bezier(x1,y1,x2,y2) as a function of linear progress. */
export function bezier(x1: number, y1: number, x2: number, y2: number) {
  const cx = 3 * x1,
    bx = 3 * (x2 - x1) - cx,
    ax = 1 - cx - bx;
  const cy = 3 * y1,
    by = 3 * (y2 - y1) - cy,
    ay = 1 - cy - by;
  const sx = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sy = (t: number) => ((ay * t + by) * t + cy) * t;
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  return (x: number) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    let t = x;
    for (let i = 0; i < 6; i++) {
      const d = dx(t);
      if (Math.abs(d) < 1e-6) break;
      t -= (sx(t) - x) / d;
    }
    return sy(Math.min(1, Math.max(0, t)));
  };
}
export const easeCine = bezier(0.16, 1, 0.3, 1);

/** Visible fraction tracking with IntersectionObserver. */
export function watch(
  el: Element,
  thresholds: number[],
  cb: (entry: IntersectionObserverEntry) => void,
  rootMargin = "0px",
) {
  const io = new IntersectionObserver((entries) => entries.forEach(cb), {
    threshold: thresholds,
    rootMargin,
  });
  io.observe(el);
  return io;
}

export function scrollToEl(el: Element, block: ScrollLogicalPosition = "start") {
  el.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block });
}
