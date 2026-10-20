// Typed access to packages/shortcuts/shortcuts.json plus parsing, formatting, matching and
// conflict detection. Pure TypeScript with no DOM dependency beyond a structural KeyboardEvent.
import data from "../shortcuts.json" with { type: "json" };

export type Platform = "mac" | "windows" | "linux";
export type Keymap = "superhuman" | "gmail";
export type Scope = "global" | "mail" | "list" | "thread" | "compose" | "search" | "palette";

export const SCOPES: readonly Scope[] = ["global", "mail", "list", "thread", "compose", "search", "palette"];
/** Scopes whose focus is a text field: single keys type text there. */
export const TEXT_SCOPES: readonly Scope[] = ["compose", "search", "palette"];

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
  scope: Scope;
  /** Listed in the command palette. */
  palette: boolean;
  /** Extra words the command palette matches (synonyms). */
  keywords?: string[];
  keys: KeySet;
  /** Overrides for prefs.keymap === "gmail". */
  gmail?: KeySet;
}

export interface Combo {
  key: string;
  cmd: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
}

/** One or more combos pressed in order (e.g. "g i"). */
export type Sequence = Combo[];

export const MODIFIERS = ["cmd", "ctrl", "alt", "shift"] as const;
export const NAMED_KEYS = [
  "enter",
  "escape",
  "tab",
  "space",
  "backspace",
  "delete",
  "up",
  "down",
  "left",
  "right",
] as const;

export const SHORTCUTS_VERSION: number = data.version;
export const SHORTCUTS: readonly ShortcutDef[] = data.shortcuts as ShortcutDef[];
export type ShortcutId = string;

const byIdMap = new Map(SHORTCUTS.map((s) => [s.id, s]));

export function getShortcut(id: ShortcutId): ShortcutDef {
  const s = byIdMap.get(id);
  if (!s) throw new Error(`unknown shortcut id: ${id}`);
  return s;
}

/** Key strings for one shortcut on one platform and keymap. Empty for palette-only commands. */
export function keysFor(
  id: ShortcutId | ShortcutDef,
  platform: Platform,
  keymap: Keymap = "superhuman",
): string[] {
  const s = typeof id === "string" ? getShortcut(id) : id;
  const set = keymap === "gmail" && s.gmail ? s.gmail : s.keys;
  return set[platform] ?? set.default;
}

// ───────────── parsing ─────────────

export function parseCombo(input: string): Combo {
  if (input.length === 0) throw new Error("empty combo");
  // "+" alone (or as the final key, e.g. "ctrl++") is a literal plus key.
  const parts =
    input === "+" ? ["+"] : input.endsWith("++") ? [...input.slice(0, -2).split("+"), "+"] : input.split("+");
  const combo: Combo = { key: "", cmd: false, ctrl: false, alt: false, shift: false };
  parts.forEach((raw, i) => {
    const part = raw.length === 1 ? raw : raw.toLowerCase();
    const last = i === parts.length - 1;
    if (!last) {
      if (!(MODIFIERS as readonly string[]).includes(part))
        throw new Error(`unknown modifier "${raw}" in "${input}"`);
      const mod = part as (typeof MODIFIERS)[number];
      if (combo[mod]) throw new Error(`duplicate modifier "${raw}" in "${input}"`);
      combo[mod] = true;
      return;
    }
    if (part.length === 0) throw new Error(`missing key in "${input}"`);
    if (part.length > 1 && !(NAMED_KEYS as readonly string[]).includes(part)) {
      throw new Error(`unknown key "${raw}" in "${input}"`);
    }
    combo.key = part.length === 1 ? part.toLowerCase() : part;
  });
  return combo;
}

export function parseSequence(input: string): Sequence {
  const steps = input.trim().split(/ +/);
  if (steps.length === 0 || steps[0] === "") throw new Error("empty key sequence");
  return steps.map(parseCombo);
}

export function comboToString(c: Combo): string {
  return [...MODIFIERS.filter((m) => c[m]), c.key].join("+");
}

export function sequenceToString(s: Sequence): string {
  return s.map(comboToString).join(" ");
}

// ───────────── formatting ─────────────

const MAC_SYMBOLS: Record<string, string> = { cmd: "⌘", ctrl: "⌃", alt: "⌥", shift: "⇧" };
const KEY_LABELS: Record<string, string> = {
  enter: "Enter",
  escape: "Esc",
  tab: "Tab",
  space: "Space",
  backspace: "Backspace",
  delete: "Delete",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
};

/** Human label, e.g. "⌘K" on macOS, "Ctrl+K" elsewhere, "G then I" for sequences. */
export function formatKeys(keys: string, platform: Platform): string {
  return parseSequence(keys)
    .map((c) => {
      const key = KEY_LABELS[c.key] ?? c.key.toUpperCase();
      if (platform === "mac") {
        const order = ["ctrl", "alt", "shift", "cmd"] as const;
        return `${order
          .filter((m) => c[m])
          .map((m) => MAC_SYMBOLS[m])
          .join("")}${key}`;
      }
      const names = { cmd: "Win", ctrl: "Ctrl", alt: "Alt", shift: "Shift" } as const;
      return [...MODIFIERS.filter((m) => c[m]).map((m) => names[m]), key].join("+");
    })
    .join(" then ");
}

// ───────────── matching ─────────────

/** Structural subset of the DOM KeyboardEvent, so this works in React Native and tests. */
export interface KeyEventLike {
  key: string;
  /** Physical key (DOM `KeyboardEvent.code`), when available. */
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

const EVENT_KEY_NAMES: Record<string, string> = {
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

const isSymbol = (k: string) => k.length === 1 && !/[a-z0-9]/i.test(k);

const CODE_KEYS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
};

function keyFromCode(code: string): string | undefined {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return CODE_KEYS[code];
}

/**
 * Normalizes a key event into a Combo. Shift is dropped for symbols typed without a command
 * modifier ("#", "?"), because it is part of the character. With cmd/ctrl/alt held, the physical
 * key (`code`) wins, so "ctrl+shift+," matches although the browser reports "<".
 */
export function comboFromEvent(e: KeyEventLike): Combo {
  const command = e.metaKey || e.ctrlKey || e.altKey;
  const physical = command && e.code ? keyFromCode(e.code) : undefined;
  const key = physical ?? EVENT_KEY_NAMES[e.key] ?? e.key.toLowerCase();
  return {
    key,
    cmd: e.metaKey,
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: physical === undefined && isSymbol(key) ? false : e.shiftKey,
  };
}

export function comboEquals(a: Combo, b: Combo): boolean {
  return a.key === b.key && a.cmd === b.cmd && a.ctrl === b.ctrl && a.alt === b.alt && a.shift === b.shift;
}

export const hasCommandModifier = (c: Combo) => c.cmd || c.ctrl || c.alt;

/** Whether a shortcut in `shortcutScope` may fire while the UI is in `active` scopes. */
export function scopeApplies(shortcutScope: Scope, active: ReadonlySet<Scope>, combo: Combo): boolean {
  const inText = TEXT_SCOPES.some((s) => active.has(s));
  if (shortcutScope === "global") return !inText || hasCommandModifier(combo);
  if (shortcutScope === "mail") return active.has("list") || active.has("thread") || active.has("mail");
  return active.has(shortcutScope);
}

export interface Binding {
  id: ShortcutId;
  scope: Scope;
  sequence: Sequence;
}

export function bindingsFor(platform: Platform, keymap: Keymap = "superhuman"): Binding[] {
  return SHORTCUTS.flatMap((s) =>
    keysFor(s, platform, keymap).map((k) => ({ id: s.id, scope: s.scope, sequence: parseSequence(k) })),
  );
}

export interface MatcherOptions {
  platform: Platform;
  keymap?: Keymap;
  /** Max gap between the steps of a sequence. */
  sequenceTimeoutMs?: number;
  now?: () => number;
}

export type MatchResult = { kind: "match"; id: ShortcutId } | { kind: "pending" } | { kind: "none" };

/**
 * Stateful matcher for single combos and multi-step sequences ("g i").
 * Feed it every keydown with the currently active scopes.
 */
export function createMatcher(opts: MatcherOptions) {
  const bindings = bindingsFor(opts.platform, opts.keymap ?? "superhuman");
  const timeout = opts.sequenceTimeoutMs ?? 1200;
  const now = opts.now ?? (() => Date.now());
  let pending: Combo[] = [];
  let lastAt = 0;

  return {
    reset() {
      pending = [];
    },
    handle(e: KeyEventLike, active: ReadonlySet<Scope>): MatchResult {
      const combo = comboFromEvent(e);
      if (["shift", "control", "meta", "alt"].includes(combo.key)) return { kind: "none" };
      const t = now();
      if (pending.length > 0 && t - lastAt > timeout) pending = [];
      lastAt = t;
      const typed = [...pending, combo];
      let prefix = false;
      for (const b of bindings) {
        if (!scopeApplies(b.scope, active, b.sequence[0]!)) continue;
        if (b.sequence.length < typed.length) continue;
        if (!typed.every((c, i) => comboEquals(c, b.sequence[i]!))) continue;
        if (b.sequence.length === typed.length) {
          pending = [];
          return { kind: "match", id: b.id };
        }
        prefix = true;
      }
      if (prefix) {
        pending = typed;
        return { kind: "pending" };
      }
      pending = [];
      return { kind: "none" };
    },
  };
}

// ───────────── palette and conflicts ─────────────

export interface PaletteCommand {
  id: ShortcutId;
  label: string;
  group: string;
  scope: Scope;
  /** Formatted key hint, e.g. "⌘K", or null for palette-only commands. */
  hint: string | null;
  /** Synonyms the palette also matches. */
  keywords: string[];
}

export function paletteCommands(platform: Platform, keymap: Keymap = "superhuman"): PaletteCommand[] {
  return SHORTCUTS.filter((s) => s.palette).map((s) => {
    const first = keysFor(s, platform, keymap)[0];
    return {
      id: s.id,
      label: s.label,
      group: s.group,
      scope: s.scope,
      hint: first ? formatKeys(first, platform) : null,
      keywords: s.keywords ?? [],
    };
  });
}

/** Two scopes can be active at once (e.g. "global" while the list has focus). */
export function scopesOverlap(a: Scope, b: Scope, combo: Combo): boolean {
  if (a === b) return true;
  const pair = new Set([a, b]);
  if (pair.has("mail") && (pair.has("list") || pair.has("thread"))) return true;
  if (pair.has("global")) {
    const other = a === "global" ? b : a;
    // Global single keys are off in text scopes; global command combos still fire there.
    return !(TEXT_SCOPES as readonly Scope[]).includes(other) || hasCommandModifier(combo);
  }
  return false;
}

export interface Conflict {
  platform: Platform;
  keymap: Keymap;
  a: string;
  b: string;
  keys: string;
}

/** Ambiguous bindings: equal sequences, or one a strict prefix of another, in overlapping scopes. */
export function findConflicts(): Conflict[] {
  const out: Conflict[] = [];
  for (const platform of ["mac", "windows", "linux"] as const) {
    for (const keymap of ["superhuman", "gmail"] as const) {
      const bs = bindingsFor(platform, keymap);
      for (let i = 0; i < bs.length; i++) {
        for (let j = i + 1; j < bs.length; j++) {
          const a = bs[i]!;
          const b = bs[j]!;
          if (a.id === b.id) continue;
          const n = Math.min(a.sequence.length, b.sequence.length);
          const samePrefix = a.sequence.slice(0, n).every((c, k) => comboEquals(c, b.sequence[k]!));
          if (samePrefix && scopesOverlap(a.scope, b.scope, a.sequence[0]!)) {
            out.push({ platform, keymap, a: a.id, b: b.id, keys: sequenceToString(a.sequence) });
          }
        }
      }
    }
  }
  return out;
}

export function detectPlatform(userAgentOrPlatform: string): Platform {
  const s = userAgentOrPlatform.toLowerCase();
  if (/mac|iphone|ipad|darwin/.test(s)) return "mac";
  if (/win/.test(s)) return "windows";
  return "linux";
}
