// Command palette filtering: a small fuzzy scorer (prefix and word-start matches rank first). Commands come from
// packages/shortcuts (`mobileCommands`), plus app-level rows without a key (theme, density, settings pages, sign
// out; `appPaletteItems`) and dynamic entries (labels, mailboxes) supplied by the screen.

export interface PaletteItem {
  id: string;
  label: string;
  group: string;
  hint: string | null;
}

/** Score of `query` against `text`, or null when it does not match. Higher is better. */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  if (t === q) return 1000;
  if (t.startsWith(q)) return 800 - t.length;
  const wordStart = t.split(/[\s/._-]+/).some((w) => w.startsWith(q));
  if (wordStart) return 600 - t.length;
  const idx = t.indexOf(q);
  if (idx >= 0) return 400 - idx;
  // Subsequence: every query character in order; reward consecutive runs and word starts.
  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const found = t.indexOf(ch, ti);
    if (found < 0) return null;
    run = found === ti ? run + 1 : 0;
    const atWord = found === 0 || /[\s/._-]/.test(t[found - 1]!);
    score += 10 + run * 5 + (atWord ? 15 : 0) - Math.min(found - ti, 10);
    ti = found + 1;
  }
  return score;
}

/** Filters and ranks items; an empty query keeps the given order. */
export function filterPalette<T extends PaletteItem>(items: readonly T[], query: string, limit = 50): T[] {
  if (!query.trim()) return items.slice(0, limit);
  return items
    .map((item, i) => {
      const s = fuzzyScore(query, item.label);
      const g = fuzzyScore(query, `${item.group} ${item.label}`);
      const best = s === null ? (g === null ? null : g - 50) : s;
      return { item, i, best };
    })
    .filter((x): x is { item: T; i: number; best: number } => x.best !== null)
    .sort((a, b) => b.best - a.best || a.i - b.i)
    .slice(0, limit)
    .map((x) => x.item);
}

/** Cursor movement in the result list (wraps). */
export function movePaletteCursor(length: number, index: number, delta: number): number {
  if (length === 0) return 0;
  return (index + delta + length) % length;
}

/** Ids of the palette rows that have no key binding (handled by the palette screen itself). */
export const APP_COMMAND_PREFIX = "app.";

export type ThemeChoice = "system" | "light" | "dark";
export type DensityChoice = "comfortable" | "compact";

/**
 * App-level rows every screen's palette offers (BRIEF: every action is reachable from the palette), like web's
 * Appearance, Settings and Account sections. The current theme and density are not offered again.
 */
export function appPaletteItems(current: { theme: ThemeChoice; density: DensityChoice }): PaletteItem[] {
  const rows: PaletteItem[] = [];
  for (const th of ["light", "dark", "system"] as const) {
    if (th === current.theme) continue;
    rows.push({
      id: `app.theme.${th}`,
      label: th === "system" ? "Use the system theme" : `Use ${th} theme`,
      group: "Appearance",
      hint: null,
    });
  }
  rows.push(
    current.density === "compact"
      ? { id: "app.density.comfortable", label: "Use comfortable rows", group: "Appearance", hint: null }
      : { id: "app.density.compact", label: "Use compact rows", group: "Appearance", hint: null },
    { id: "app.settings", label: "Settings", group: "Settings", hint: null },
    { id: "app.settings.ai", label: "Settings: AI", group: "Settings", hint: null },
    { id: "app.settings.shortcuts", label: "Settings: Keyboard shortcuts", group: "Settings", hint: null },
    { id: "app.mailboxes", label: "Mailboxes", group: "Navigation", hint: null },
    { id: "app.signOut", label: "Sign out", group: "Account", hint: null },
  );
  return rows;
}

/** Id of the "Create label" row in the label picker. */
export const CREATE_LABEL_ID = "create-label";

/**
 * The label picker's rows after filtering: a "Create label “<query>”" row is added when the typed name matches no
 * label exactly (labels.create, §4.2).
 */
export function withCreateLabel<T extends PaletteItem>(
  filtered: readonly T[],
  all: readonly T[],
  query: string,
): (T | PaletteItem)[] {
  const name = query.trim();
  if (!name || all.some((i) => i.label.toLowerCase() === name.toLowerCase())) return [...filtered];
  return [...filtered, { id: CREATE_LABEL_ID, label: `Create label “${name}”`, group: "New", hint: null }];
}
