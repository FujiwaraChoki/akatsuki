// "?" — every shortcut of the current keymap, grouped as in packages/shortcuts.
import { SHORTCUTS } from "@akatsuki/shortcuts";
import { Dialog, firstKeys, Kbd, useUiConfig } from "@akatsuki/ui";

/** Web-only accessibility bindings outside packages/shortcuts (D-399): listed with the Navigation group. */
const WEB_ONLY: Record<string, { label: string; keys: string }[]> = {
  Navigation: [
    { label: "Move between panes", keys: "F6" },
    { label: "Move between panes, backwards", keys: "Shift+F6" },
  ],
};

export function HelpDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
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
      <ShortcutGroups />
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
export function ShortcutGroups() {
  const config = useUiConfig();
  const groups = new Map<string, (typeof SHORTCUTS)[number][]>();
  for (const s of SHORTCUTS) {
    if (!firstKeys(s.id, config) || s.scope === "palette") continue;
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
            {(WEB_ONLY[group] ?? []).map((w) => (
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
