// One global keydown listener driven by packages/shortcuts (`createMatcher`): single keys, chords and
// sequences ("g i"), per scope. Only ids with a handler fire; everything else falls through to the browser.
import { createMatcher, type Keymap, type Platform, type Scope } from "@akatsuki/shortcuts";
import { useEffect, useMemo, useRef } from "react";

export type Handlers = Partial<Record<string, () => void>>;

export function isTextInput(el: Element | null): boolean {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return true;
  if (el instanceof HTMLInputElement)
    return !["checkbox", "radio", "button", "submit", "reset", "range", "color", "file"].includes(el.type);
  return (el as HTMLElement).isContentEditable === true;
}

export interface ShortcutOptions {
  platform: Platform;
  keymap: Keymap;
  /** Scopes active right now (e.g. `list`, `thread`). `global` is always added. */
  scopes: readonly Scope[];
  handlers: Handlers;
  /** When false (a dialog or the palette owns the keyboard), nothing fires except what they handle. */
  enabled: boolean;
}

export function useShortcuts({ platform, keymap, scopes, handlers, enabled }: ShortcutOptions) {
  const matcher = useMemo(() => createMatcher({ platform, keymap }), [platform, keymap]);
  const state = useRef({ scopes, handlers, enabled });
  state.current = { scopes, handlers, enabled };

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const s = state.current;
      if (!s.enabled || e.isComposing || e.defaultPrevented) return;
      // A modal dialog (a part-2 overlay such as the reminder picker) owns the keys typed inside it.
      if (e.target instanceof Element && e.target.closest(".ak-dialog")) return;
      // Tab moves focus between controls, as everywhere. Only on the element that owns it (the conversation
      // list, `data-ak-tab-scope`) or with nothing focused does Tab reach the key map (split switching).
      if (e.key === "Tab" && !tabReachesKeymap(document.activeElement)) return;
      const active = new Set<Scope>(["global", ...s.scopes]);
      // Inside any text field single keys type text: treat it as a text scope.
      if (isTextInput(document.activeElement)) active.add("search");
      const r = matcher.handle(e, active);
      if (r.kind === "pending") {
        e.preventDefault();
        return;
      }
      if (r.kind !== "match") return;
      const run = s.handlers[r.id];
      if (!run) return;
      e.preventDefault();
      e.stopPropagation();
      run();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [matcher]);

  useEffect(() => {
    if (!enabled) matcher.reset();
  }, [enabled, matcher]);
}

/** Whether a Tab press goes to the key map: nothing is focused, or the focused element owns Tab. */
export function tabReachesKeymap(active: Element | null): boolean {
  return active === null || active === document.body || active.hasAttribute("data-ak-tab-scope");
}
