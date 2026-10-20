// Typed, DOM-free access to the workspace key map (@akatsuki/shortcuts, packages/shortcuts/shortcuts.json).
// Labels on the site are generated from it at build time, so they cannot drift from the apps.
import data from "@akatsuki/shortcuts/shortcuts.json";

export interface KeySet {
  default: string[];
  mac?: string[];
  windows?: string[];
  linux?: string[];
}
export interface ShortcutDef {
  id: string;
  label: string;
  group: string;
  scope: string;
  palette: boolean;
  keys: KeySet;
}

export const SHORTCUTS: readonly ShortcutDef[] = data.shortcuts as ShortcutDef[];
export const SHORTCUT_COUNT = SHORTCUTS.length;

const byId = new Map(SHORTCUTS.map((s) => [s.id, s]));
export function shortcut(id: string): ShortcutDef {
  const s = byId.get(id);
  if (!s) throw new Error(`unknown shortcut ${id}`);
  return s;
}

const NUMBER_WORDS: Record<number, string> = {
  50: "Fifty",
  51: "Fifty-one",
  52: "Fifty-two",
  53: "Fifty-three",
  54: "Fifty-four",
  55: "Fifty-five",
  56: "Fifty-six",
  57: "Fifty-seven",
  58: "Fifty-eight",
  59: "Fifty-nine",
  60: "Sixty",
  61: "Sixty-one",
  62: "Sixty-two",
  63: "Sixty-three",
  64: "Sixty-four",
  65: "Sixty-five",
  66: "Sixty-six",
  67: "Sixty-seven",
  68: "Sixty-eight",
  69: "Sixty-nine",
  70: "Seventy",
};
/** "Sixty", from the real length of the key map. Falls back to digits outside the table. */
export const SHORTCUT_COUNT_WORD = NUMBER_WORDS[SHORTCUT_COUNT] ?? String(SHORTCUT_COUNT);

export type Plat = "mac" | "other";

const MAC_MOD: Record<string, string> = { ctrl: "⌃", alt: "⌥", shift: "⇧", cmd: "⌘" };
const OTHER_MOD: Record<string, string> = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", cmd: "Win" };
const NAMED: Record<string, string> = {
  enter: "↵",
  escape: "Esc",
  tab: "Tab",
  space: "Space",
  backspace: "⌫",
  delete: "Del",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
};

/** "cmd+shift+l" → ["⌘","⇧","L"] on mac, ["Ctrl","Shift","L"] elsewhere. */
export function comboCaps(combo: string, plat: Plat): string[] {
  const parts = combo === "+" ? ["+"] : combo.split("+");
  const key = parts.pop() ?? "";
  const mods = parts;
  const order = plat === "mac" ? ["ctrl", "alt", "shift", "cmd"] : ["ctrl", "alt", "shift", "cmd"];
  const map = plat === "mac" ? MAC_MOD : OTHER_MOD;
  const caps = order.filter((m) => mods.includes(m)).map((m) => map[m] ?? m);
  caps.push(NAMED[key] ?? key.toUpperCase());
  return caps;
}

/** A key string ("g i", "ctrl+k") as steps of keycaps. */
export function sequenceCaps(keys: string, plat: Plat): string[][] {
  return keys
    .trim()
    .split(/ +/)
    .map((c) => comboCaps(c, plat));
}

export function keysFor(s: ShortcutDef, plat: Plat): string[] {
  if (plat === "mac") return s.keys.mac ?? s.keys.default;
  return s.keys.linux ?? s.keys.windows ?? s.keys.default;
}

export const GROUP_ORDER = [
  "General",
  "Navigation",
  "Selection",
  "Conversation",
  "AI",
  "Compose",
  "Search",
  "Command palette",
];
