// Site command palette (§9) and the shortcuts sheet (§5.4). Static markup, opened by the client.

import { GROUP_ORDER, SHORTCUT_COUNT, SHORTCUTS } from "@/lib/shortcuts";
import { PAGES, SITE } from "@/lib/site";
import { Kbd, Keys } from "./Kbd";

interface Item {
  id: string;
  label: string;
  sub?: string;
  hint?: string;
  data: Record<string, string>;
}

const GROUPS: { label: string; items: Item[] }[] = [
  {
    label: "Pages",
    items: PAGES.map((p) => ({
      id: `page-${p.href}`,
      label: p.title,
      sub: p.summary,
      data: { "data-href": p.href },
    })),
  },
  {
    label: "Actions",
    items: [
      { id: "act-waitlist", label: "Join the waitlist", hint: "W", data: { "data-cmd": "waitlist" } },
      { id: "act-inbox", label: "Try the inbox", data: { "data-cmd": "inbox" } },
      { id: "act-shortcuts", label: "Show all shortcuts", hint: "?", data: { "data-cmd": "shortcuts" } },
      { id: "act-copy", label: "Copy contact address", sub: SITE.contact, data: { "data-cmd": "copy" } },
    ],
  },
  {
    label: "Theme",
    items: [
      { id: "theme-system", label: "Theme: Auto", data: { "data-cmd": "theme:system" } },
      { id: "theme-light", label: "Theme: Light", data: { "data-cmd": "theme:light" } },
      { id: "theme-dark", label: "Theme: Dark", data: { "data-cmd": "theme:dark" } },
    ],
  },
];

export function Palette() {
  return (
    <>
      <div className="backdrop" data-backdrop hidden />
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Search the site"
        data-palette
        hidden
      >
        <input
          className="pal-input"
          type="text"
          placeholder="Search pages and actions…"
          aria-label="Search pages and actions"
          role="combobox"
          aria-expanded="true"
          aria-controls="pal-list"
          autoComplete="off"
          spellCheck={false}
          data-pal-input
        />
        <div className="pal-list" id="pal-list" role="listbox" aria-label="Results">
          {GROUPS.map((g) => (
            // biome-ignore lint/a11y/useSemanticElements: option groups inside an ARIA listbox (ARIA 1.2 group role)
            <div key={g.label} role="group" aria-label={g.label} data-pal-group>
              <p className="label pal-group">{g.label}</p>
              {g.items.map((it) => (
                <button
                  key={it.id}
                  type="button"
                  id={it.id}
                  role="option"
                  aria-selected="false"
                  className="pal-item"
                  data-pal-item
                  data-search={`${g.label} ${it.label} ${it.sub ?? ""}`.toLowerCase()}
                  tabIndex={-1}
                  {...it.data}
                >
                  <span>
                    {it.label}
                    {it.sub && <span className="pal-sub">{it.sub}</span>}
                  </span>
                  {it.hint && <Kbd>{it.hint}</Kbd>}
                </button>
              ))}
            </div>
          ))}
          <p className="pal-empty" data-pal-empty hidden>
            Nothing matches. Try “pricing” or “theme”.
          </p>
        </div>
        <div className="pal-foot" aria-hidden="true">
          <span>
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> move
          </span>
          <span>
            <Kbd>↵</Kbd> open
          </span>
          <span>
            <Kbd>Esc</Kbd> close
          </span>
        </div>
      </div>
    </>
  );
}

export function ShortcutsSheet() {
  const groups = GROUP_ORDER.map((g) => ({ g, items: SHORTCUTS.filter((s) => s.group === g) })).filter(
    (x) => x.items.length,
  );
  return (
    <div className="sheet" role="dialog" aria-modal="true" aria-labelledby="sc-title" data-shortcuts hidden>
      <div className="sheet-head">
        <h2 className="sheet-title" id="sc-title">
          Shortcuts
        </h2>
        <button type="button" className="sheet-close" data-close-sheet>
          Close <Kbd>Esc</Kbd>
        </button>
      </div>
      {/* biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable region must take focus so the keyboard can scroll it */}
      {/* biome-ignore lint/a11y/useSemanticElements: the sheet's scroll container, named for screen readers */}
      <div className="sheet-body" tabIndex={0} role="region" aria-label="All shortcuts" data-sheet-body>
        <p className="small" style={{ marginTop: 20 }}>
          All {SHORTCUT_COUNT} actions from the one key map every Akatsuki app reads.
        </p>
        {/* Rows are parsed into a template and only enter the DOM on first open. */}
        <template data-sheet-tpl>
          {groups.map(({ g, items }) => (
            <section key={g} className="sc-group" aria-label={g}>
              <span className="label">{g}</span>
              {items.map((s) => (
                <div key={s.id} className="sc-row">
                  <span>
                    {s.keys.default.length ? (
                      <Keys def={s} />
                    ) : (
                      <span className="small">From the palette</span>
                    )}
                  </span>
                  <span>{s.label}</span>
                </div>
              ))}
            </section>
          ))}
        </template>
      </div>
    </div>
  );
}
