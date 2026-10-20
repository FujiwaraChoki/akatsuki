// "?" — every shortcut of the current keymap, grouped as in packages/shortcuts.
import { SHORTCUTS } from "@akatsuki/shortcuts";
import { Dialog, firstKeys, Kbd, useUiConfig } from "@akatsuki/ui";
import { useEffect, useRef, useState } from "react";

/** Web-only accessibility bindings outside packages/shortcuts (D-399): listed with the Navigation group. */
const WEB_ONLY: Record<string, { label: string; keys: string }[]> = {
  Navigation: [
    { label: "Move between panes", keys: "F6" },
    { label: "Move between panes, backwards", keys: "Shift+F6" },
  ],
};

export function HelpDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const [q, setQ] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    setQ("");
    // A letter typed while the sheet itself has focus goes to the filter (and stays there).
    const onKey = (e: KeyboardEvent) => {
      const a = document.activeElement;
      if (e.key.length !== 1 || e.ctrlKey || e.metaKey || e.altKey || e.key === " ") return;
      if (a instanceof HTMLElement && a.classList.contains("ak-dialog--help")) input.current?.focus();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title="Keyboard shortcuts"
      size="lg"
      closeButton
      className="ak-dialog--help"
      // The dialog itself is the one scroll container: focus it, so Arrow keys, Page Down and Space scroll the
      // list (focus on the Close button would make Space close it).
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        (e.currentTarget as HTMLElement | null)?.focus();
      }}
    >
      {/* Typing anywhere in the sheet filters it (SPEC §3.11); the sheet keeps focus so the keys scroll it. */}
      <input
        ref={input}
        className="ak-picker__input ak-picker__input--filter ak-help__filter"
        type="search"
        aria-label="Filter shortcuts"
        placeholder="Filter shortcuts"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        autoComplete="off"
        spellCheck={false}
      />
      <ShortcutGroups filter={q} />
    </Dialog>
  );
}

/** Keycaps for keys the shortcut notation cannot express yet (function keys), styled like `<Kbd>`. */
function FunctionKeys({ keys }: { keys: string }) {
  const parts = keys.split("+");
  return (
    <kbd className="ak-kbd">
      <span className="ak-kbd__label">{parts.join(" ")}</span>
      <span className="ak-kbd__step" aria-hidden="true">
        {parts.map((k) => (
          <kbd key={k} className="ak-kbd__key">
            {k}
          </kbd>
        ))}
      </span>
    </kbd>
  );
}

/** The shortcut list of the current keymap (the help dialog and Settings, Shortcuts). */
export function ShortcutGroups({ filter = "" }: { filter?: string }) {
  const config = useUiConfig();
  const groups = new Map<string, (typeof SHORTCUTS)[number][]>();
  const f = filter.trim().toLowerCase();
  for (const s of SHORTCUTS) {
    if (!firstKeys(s.id, config) || s.scope === "palette") continue;
    if (f && !`${s.label} ${s.group}`.toLowerCase().includes(f)) continue;
    const g = groups.get(s.group) ?? [];
    g.push(s);
    groups.set(s.group, g);
  }
  return (
    <div className="ak-help">
      {[...groups].map(([group, items]) => (
        <section key={group} className="ak-help__group" aria-label={group}>
          <h3 className="ak-help__heading">{group}</h3>
          <dl className="ak-help__list">
            {items.map((s) => (
              <div key={s.id} className="ak-help__row">
                <dt>{s.label}</dt>
                <dd>
                  <Kbd shortcut={s.id} />
                </dd>
              </div>
            ))}
            {(f ? [] : (WEB_ONLY[group] ?? [])).map((w) => (
              <div key={w.keys} className="ak-help__row">
                <dt>{w.label}</dt>
                <dd>
                  <FunctionKeys keys={w.keys} />
                </dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
    </div>
  );
}
