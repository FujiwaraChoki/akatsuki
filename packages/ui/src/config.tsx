// Platform and keymap for shortcut hints, plus small helpers shared by every component.
import {
  comboEquals,
  comboFromEvent,
  detectPlatform,
  formatKeys,
  type KeyEventLike,
  type Keymap,
  keysFor,
  type Platform,
  parseSequence,
  type ShortcutId,
} from "@akatsuki/shortcuts";
import { createContext, type ReactNode, useContext, useMemo } from "react";

export interface UiConfig {
  platform: Platform;
  /** `prefs.keymap`. */
  keymap: Keymap;
}

const UiConfigContext = createContext<UiConfig | null>(null);

/** The platform of this browser (user agent), `linux` when unknown (SSR, tests). */
export function currentPlatform(): Platform {
  const nav = (globalThis as { navigator?: { userAgent?: string; platform?: string } }).navigator;
  return detectPlatform(nav?.userAgent || nav?.platform || "linux");
}

/** Provides platform and keymap to shortcut hints. Without it, hints use this browser's platform. */
export function UiConfigProvider({
  platform,
  keymap = "superhuman",
  children,
}: {
  platform?: Platform;
  keymap?: Keymap;
  children: ReactNode;
}) {
  const value = useMemo(() => ({ platform: platform ?? currentPlatform(), keymap }), [platform, keymap]);
  return <UiConfigContext.Provider value={value}>{children}</UiConfigContext.Provider>;
}

export function useUiConfig(): UiConfig {
  const ctx = useContext(UiConfigContext);
  return useMemo(() => ctx ?? { platform: currentPlatform(), keymap: "superhuman" }, [ctx]);
}

/** The first key binding of a shortcut, unformatted (e.g. `"cmd+k"`), or `null` for palette-only commands. */
export function firstKeys(id: ShortcutId, config: UiConfig): string | null {
  return keysFor(id, config.platform, config.keymap)[0] ?? null;
}

/** The formatted hint of a shortcut (e.g. "⌘K", "Ctrl+K", "G then I"), or `null`. */
export function useShortcutHint(id: ShortcutId | undefined): string | null {
  const config = useUiConfig();
  if (!id) return null;
  const keys = firstKeys(id, config);
  return keys ? formatKeys(keys, config.platform) : null;
}

const ARIA_KEYS: Record<string, string> = {
  enter: "Enter",
  escape: "Escape",
  tab: "Tab",
  space: "Space",
  backspace: "Backspace",
  delete: "Delete",
  up: "ArrowUp",
  down: "ArrowDown",
  left: "ArrowLeft",
  right: "ArrowRight",
};

/**
 * The `aria-keyshortcuts` value of a shortcut (e.g. "Meta+K", "Shift+E"), or `undefined` for sequences
 * ("g i"), which the attribute cannot express, and for palette-only commands.
 */
export function useAriaKeyShortcuts(id: ShortcutId | undefined): string | undefined {
  const config = useUiConfig();
  if (!id) return undefined;
  const keys = firstKeys(id, config);
  if (!keys) return undefined;
  const steps = parseSequence(keys);
  if (steps.length !== 1) return undefined;
  const c = steps[0]!;
  const mods = [c.ctrl && "Control", c.alt && "Alt", c.shift && "Shift", c.cmd && "Meta"].filter(Boolean);
  return [...mods, ARIA_KEYS[c.key] ?? c.key.toUpperCase()].join("+");
}

/** Whether a key event is one of a shortcut's single-step bindings (e.g. `palette.next`: down or ctrl+n). */
export function eventMatchesShortcut(e: KeyEventLike, id: ShortcutId, config: UiConfig): boolean {
  const combo = comboFromEvent(e);
  return keysFor(id, config.platform, config.keymap).some((k) => {
    const seq = parseSequence(k);
    return seq.length === 1 && comboEquals(seq[0]!, combo);
  });
}

/** Joins class names, skipping falsy values. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}
