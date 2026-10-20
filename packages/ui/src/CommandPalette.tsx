// The command palette (BRIEF: every action reachable from it). A modal combobox over a listbox: fuzzy match on
// label and keywords, sections, shortcut hints from packages/shortcuts, keyboard-first (the palette.* bindings:
// up/down or ctrl+p/ctrl+n, Enter, Escape), mouse friendly, orbs behind it.
import { type Keymap, type Platform, paletteCommands, type ShortcutId } from "@akatsuki/shortcuts";
import * as RadixDialog from "@radix-ui/react-dialog";
import { VisuallyHidden } from "@radix-ui/react-visually-hidden";
import { type KeyboardEvent, type ReactNode, useEffect, useId, useMemo, useRef, useState } from "react";
import { cx, eventMatchesShortcut, useUiConfig } from "./config.tsx";
import { fuzzyMatch, highlightRuns } from "./fuzzy.ts";
import { Kbd } from "./Kbd.tsx";
import { Orbs } from "./Orbs.tsx";

export interface PaletteItem {
  /** Unique; a shortcut id for commands from packages/shortcuts. */
  id: string;
  label: string;
  /** Section heading (e.g. "Conversation", "Navigation"). Items without one go under "Commands". */
  section?: string;
  /** Extra words that match (e.g. "delete" for "Move to Trash"). */
  keywords?: string[];
  /** Shows this shortcut's key. */
  shortcut?: ShortcutId;
  /** Free-form hint on the right when there is no shortcut (e.g. an account address). */
  hint?: string;
  icon?: ReactNode;
  disabled?: boolean;
  run: () => void;
}

export interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: readonly PaletteItem[];
  placeholder?: string;
  emptyText?: string;
  /** Ids shown first, under "Recent", while the query is empty. */
  recent?: readonly string[];
  /** Called after an item runs (e.g. to record it as recent). */
  onRun?: (item: PaletteItem) => void;
  /** Accessible title of the dialog. */
  title?: string;
}

interface Section {
  name: string;
  rows: { item: PaletteItem; indices: number[] }[];
}

function sectionOf(sections: Map<string, Section>, item: PaletteItem): Section {
  const name = item.section ?? "Commands";
  const existing = sections.get(name);
  if (existing) return existing;
  const created: Section = { name, rows: [] };
  sections.set(name, created);
  return created;
}

/** Filters and groups items: input order while the query is empty, best match first otherwise. */
export function rankPaletteItems(
  items: readonly PaletteItem[],
  query: string,
  recent: readonly string[] = [],
): Section[] {
  const q = query.trim();
  if (q === "") {
    const sections = new Map<string, Section>();
    const byId = new Map(items.map((i) => [i.id, i]));
    const recents = recent.map((id) => byId.get(id)).filter((i): i is PaletteItem => i !== undefined);
    if (recents.length > 0)
      sections.set("Recent", { name: "Recent", rows: recents.map((item) => ({ item, indices: [] })) });
    const shown = new Set(recents.map((i) => i.id));
    for (const item of items) {
      if (shown.has(item.id)) continue;
      sectionOf(sections, item).rows.push({ item, indices: [] });
    }
    return [...sections.values()];
  }
  const scored = items
    .map((item, order) => ({
      item,
      order,
      m: fuzzyMatch(q, item.label, [...(item.keywords ?? []), item.section ?? ""]),
    }))
    .filter((x): x is { item: PaletteItem; order: number; m: NonNullable<typeof x.m> } => x.m !== null)
    .sort((a, b) => b.m.score - a.m.score || a.order - b.order);
  const sections = new Map<string, Section>();
  for (const { item, m } of scored) {
    sectionOf(sections, item).rows.push({ item, indices: m.indices });
  }
  return [...sections.values()];
}

/**
 * Palette items for every command of packages/shortcuts that has a handler. Commands without a handler are
 * left out, so the palette only offers what the current screen can do.
 */
export function paletteItemsFromShortcuts(
  handlers: Partial<Record<ShortcutId, () => void>>,
  opts: { platform: Platform; keymap?: Keymap },
): PaletteItem[] {
  return paletteCommands(opts.platform, opts.keymap).flatMap((c) => {
    const run = handlers[c.id];
    return run
      ? [{ id: c.id, label: c.label, section: c.group, shortcut: c.id, keywords: c.keywords, run }]
      : [];
  });
}

export function CommandPalette({
  open,
  onOpenChange,
  items,
  placeholder = "Type a command or search",
  emptyText = "No matching commands.",
  recent,
  onRun,
  title = "Command palette",
}: CommandPaletteProps) {
  const config = useUiConfig();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const listId = useId();
  const nodes = useRef(new Map<string, HTMLElement>());
  // Where focus was when the palette opened (a draft's body, the list, a field): it goes back there when the
  // palette closes, unless the command that ran moved it on purpose.
  const returnTo = useRef<HTMLElement | null>(null);

  const sections = useMemo(() => rankPaletteItems(items, query, recent), [items, query, recent]);
  const flat = useMemo(() => sections.flatMap((s) => s.rows.map((r) => r.item)), [sections]);
  const enabled = flat.filter((i) => !i.disabled);
  const activeItem = enabled.length > 0 ? enabled[Math.min(active, enabled.length - 1)] : undefined;
  const optionId = (item: PaletteItem) => `${listId}-${item.id}`;

  useEffect(() => {
    if (!open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  useEffect(() => {
    if (!activeItem) return;
    nodes.current.get(activeItem.id)?.scrollIntoView?.({ block: "nearest" });
  });

  const run = (item: PaletteItem | undefined) => {
    if (!item || item.disabled) return;
    onOpenChange(false);
    item.run();
    onRun?.(item);
  };

  const move = (delta: number) => {
    if (enabled.length === 0) return;
    const current = activeItem ? enabled.indexOf(activeItem) : 0;
    setActive((current + delta + enabled.length) % enabled.length);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const ev = e.nativeEvent;
    if (eventMatchesShortcut(ev, "palette.next", config)) {
      e.preventDefault();
      move(1);
    } else if (eventMatchesShortcut(ev, "palette.prev", config)) {
      e.preventDefault();
      move(-1);
    } else if (eventMatchesShortcut(ev, "palette.run", config)) {
      e.preventDefault();
      run(activeItem);
    } else if (e.key === "Home" && e.ctrlKey) {
      e.preventDefault();
      setActive(0);
    } else if (e.key === "End" && e.ctrlKey) {
      e.preventDefault();
      setActive(Math.max(0, enabled.length - 1));
    }
  };

  return (
    <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="ak-palette__overlay">
          <Orbs variant="subtle" />
        </RadixDialog.Overlay>
        <RadixDialog.Content
          className="ak-palette"
          aria-describedby={undefined}
          onOpenAutoFocus={() => {
            // Radix calls this before it moves focus into the palette.
            const a = document.activeElement;
            returnTo.current = a instanceof HTMLElement && a !== document.body ? a : null;
          }}
          onCloseAutoFocus={(e) => {
            // Radix would focus the trigger (there is none), leaving focus on <body>: the next keys would run list
            // shortcuts instead of reaching the field the user was typing in.
            e.preventDefault();
            const el = returnTo.current;
            returnTo.current = null;
            const a = document.activeElement;
            const lost = !a || a === document.body || !a.isConnected || a.closest(".ak-palette") !== null;
            if (el?.isConnected && lost) el.focus({ preventScroll: true });
          }}
        >
          <VisuallyHidden>
            <RadixDialog.Title>{title}</RadixDialog.Title>
          </VisuallyHidden>
          <div className="ak-palette__search">
            <input
              className="ak-palette__input"
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-autocomplete="list"
              aria-activedescendant={activeItem ? optionId(activeItem) : undefined}
              aria-label={title}
              placeholder={placeholder}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(0);
              }}
              onKeyDown={onKeyDown}
              autoComplete="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
          <div className="ak-palette__list" id={listId} role="listbox" aria-label="Commands">
            {flat.length === 0 && <p className="ak-palette__empty">{emptyText}</p>}
            {sections.map((section) => (
              // biome-ignore lint/a11y/useSemanticElements: role="group" inside a listbox is the ARIA pattern
              <div key={section.name} role="group" aria-label={section.name} className="ak-palette__section">
                <div className="ak-palette__heading" aria-hidden="true">
                  {section.name}
                </div>
                {section.rows.map(({ item, indices }) => {
                  const selected = item === activeItem;
                  return (
                    // biome-ignore lint/a11y/useFocusableInteractive: focus stays in the combobox (aria-activedescendant)
                    // biome-ignore lint/a11y/useKeyWithClickEvents: the combobox input handles the keys
                    <div
                      key={item.id}
                      ref={(el) => {
                        if (el) nodes.current.set(item.id, el);
                        else nodes.current.delete(item.id);
                      }}
                      id={optionId(item)}
                      role="option"
                      aria-selected={selected}
                      aria-disabled={item.disabled || undefined}
                      className={cx(
                        "ak-palette__item",
                        selected && "ak-palette__item--active",
                        item.disabled && "ak-palette__item--disabled",
                      )}
                      onPointerMove={() => {
                        const i = enabled.indexOf(item);
                        if (i >= 0 && !selected) setActive(i);
                      }}
                      onClick={() => run(item)}
                    >
                      {item.icon && (
                        <span className="ak-palette__icon" aria-hidden="true">
                          {item.icon}
                        </span>
                      )}
                      <span className="ak-palette__label">
                        {highlightRuns(item.label, indices).map((r, i) =>
                          r.match ? (
                            // biome-ignore lint/suspicious/noArrayIndexKey: runs are positional
                            <mark key={i} className="ak-palette__match">
                              {r.text}
                            </mark>
                          ) : (
                            // biome-ignore lint/suspicious/noArrayIndexKey: runs are positional
                            <span key={i}>{r.text}</span>
                          ),
                        )}
                      </span>
                      {item.shortcut ? (
                        <Kbd shortcut={item.shortcut} className="ak-palette__kbd" />
                      ) : (
                        item.hint && <span className="ak-palette__hint">{item.hint}</span>
                      )}
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          <div className="ak-palette__footer" aria-hidden="true">
            <span>
              <Kbd shortcut="palette.prev" /> <Kbd shortcut="palette.next" /> to move
            </span>
            <span>
              <Kbd shortcut="palette.run" /> to run
            </span>
            <span>
              <Kbd shortcut="palette.close" /> to close
            </span>
          </div>
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
