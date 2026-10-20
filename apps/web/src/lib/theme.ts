// Light / dark / system theme (BRIEF design theme, `prefs.theme`). The tokens stylesheet follows
// `prefers-color-scheme` unless `<html data-theme>` forces a mode. The choice is mirrored in localStorage so
// `public/theme-boot.js` can apply it before the first paint; the server copy lives in `prefs.theme`.
import { useEffect, useSyncExternalStore } from "react";

export type ThemePref = "system" | "light" | "dark";
export type Density = "compact" | "comfortable";
export const THEME_KEY = "ak:theme";
export const DENSITY_KEY = "ak:density";

const listeners = new Set<() => void>();
const emit = () => {
  for (const l of listeners) l();
};

function read<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return (allowed as readonly string[]).includes(v ?? "") ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* private mode: in-memory only */
  }
}

export function storedTheme(): ThemePref {
  return read(THEME_KEY, ["system", "light", "dark"] as const, "system");
}

/** Sets `<html data-theme>` (none for system) and remembers the choice. */
export function applyTheme(pref: ThemePref): void {
  const root = document.documentElement;
  if (pref === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", pref);
  write(THEME_KEY, pref === "system" ? null : pref);
  emit();
}

export function applyDensity(d: Density): void {
  document.documentElement.setAttribute("data-density", d);
  write(DENSITY_KEY, d);
  emit();
}

export function storedDensity(): Density {
  return read(DENSITY_KEY, ["compact", "comfortable"] as const, "comfortable");
}

/** The next theme in the toggle cycle: system → light → dark → system. */
export function nextTheme(t: ThemePref): ThemePref {
  return t === "system" ? "light" : t === "light" ? "dark" : "system";
}

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** The mode actually shown. */
export function resolvedMode(pref: ThemePref = storedTheme()): "light" | "dark" {
  if (pref !== "system") return pref;
  return typeof matchMedia === "function" && matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

function subscribe(cb: () => void) {
  listeners.add(cb);
  const mq = typeof matchMedia === "function" ? matchMedia(DARK_QUERY) : null;
  mq?.addEventListener?.("change", cb);
  return () => {
    listeners.delete(cb);
    mq?.removeEventListener?.("change", cb);
  };
}

/** `{ pref, mode }`, live across toggles and OS changes. */
export function useTheme(): { pref: ThemePref; mode: "light" | "dark" } {
  const key = useSyncExternalStore(
    subscribe,
    () => `${storedTheme()}|${resolvedMode()}`,
    () => "system|light",
  );
  const [pref, mode] = key.split("|") as [ThemePref, "light" | "dark"];
  return { pref, mode };
}

export function useDensity(): Density {
  return useSyncExternalStore(subscribe, storedDensity, () => "comfortable" as const);
}

// ───────────── unsaved local choices (pending saves) ─────────────
// A theme or density picked here is applied at once and saved with `me.updatePrefs`. Until the server has
// confirmed it, the choice is also kept under PENDING_KEY, so a reload (or a lost connection) before the save
// lands does not let the older server value overwrite it: the next load re-sends it instead.

export type AppearancePatch = { theme?: ThemePref; density?: Density };
export const PENDING_KEY = "ak:prefs-pending";
/** A pending choice older than this is dropped (the save kept failing, or another device won meanwhile). */
export const PENDING_MAX_AGE_MS = 24 * 60 * 60 * 1000;

type Pending = AppearancePatch & { at: number };

export function pendingAppearance(now = Date.now()): AppearancePatch {
  try {
    const raw = localStorage.getItem(PENDING_KEY);
    if (!raw) return {};
    const p = JSON.parse(raw) as Partial<Pending>;
    if (typeof p.at !== "number" || now - p.at > PENDING_MAX_AGE_MS || now < p.at - 60_000) {
      localStorage.removeItem(PENDING_KEY);
      return {};
    }
    const out: AppearancePatch = {};
    if (p.theme === "system" || p.theme === "light" || p.theme === "dark") out.theme = p.theme;
    if (p.density === "compact" || p.density === "comfortable") out.density = p.density;
    return out;
  } catch {
    return {};
  }
}

function writePending(p: AppearancePatch) {
  if (p.theme === undefined && p.density === undefined) write(PENDING_KEY, null);
  else write(PENDING_KEY, JSON.stringify({ ...p, at: Date.now() }));
}

function rememberPending(patch: AppearancePatch) {
  writePending({ ...pendingAppearance(), ...patch });
}

/** Forgets the pending values the server now holds (only those still equal to `saved`). */
function settlePending(saved: AppearancePatch) {
  const p = pendingAppearance();
  if (saved.theme !== undefined && p.theme === saved.theme) delete p.theme;
  if (saved.density !== undefined && p.density === saved.density) delete p.density;
  writePending(p);
}

export type SavePrefs = (patch: AppearancePatch) => Promise<unknown>;

/** Applies `patch` now and saves it; until the save resolves the choice survives reloads (see above). */
export function setAppearance(patch: AppearancePatch, save: SavePrefs): Promise<void> {
  if (patch.theme !== undefined) applyTheme(patch.theme);
  if (patch.density !== undefined) applyDensity(patch.density);
  rememberPending(patch);
  return save(patch).then(() => settlePending(patch));
}

const resending = new Set<string>();

/**
 * Keeps the local theme in step with the server preference when it changes elsewhere. A local choice that is
 * newer than the server value (still pending, see `setAppearance`) wins and is sent again with `save`.
 */
export function useSyncServerTheme(
  serverTheme: ThemePref | undefined,
  serverDensity: Density | undefined,
  save?: SavePrefs,
) {
  useEffect(() => {
    if (!serverTheme) return;
    const pending = pendingAppearance().theme;
    if (pending !== undefined) {
      if (pending === serverTheme) return settlePending({ theme: pending });
      if (pending !== storedTheme()) applyTheme(pending);
      resend({ theme: pending }, save);
      return;
    }
    if (serverTheme !== storedTheme()) applyTheme(serverTheme);
  }, [serverTheme, save]);
  useEffect(() => {
    if (!serverDensity) return;
    const pending = pendingAppearance().density;
    if (pending !== undefined) {
      if (pending === serverDensity) return settlePending({ density: pending });
      if (pending !== storedDensity()) applyDensity(pending);
      resend({ density: pending }, save);
      return;
    }
    if (serverDensity !== storedDensity()) applyDensity(serverDensity);
  }, [serverDensity, save]);
}

function resend(patch: AppearancePatch, save: SavePrefs | undefined) {
  if (!save) return;
  const key = JSON.stringify(patch);
  if (resending.has(key)) return;
  resending.add(key);
  save(patch)
    .then(() => settlePending(patch))
    .catch(() => undefined)
    .finally(() => resending.delete(key));
}
