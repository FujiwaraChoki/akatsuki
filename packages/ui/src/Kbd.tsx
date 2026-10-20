// Keyboard hints. Keys always come from packages/shortcuts (by shortcut id) so nothing hard-codes a binding.
import { formatKeys, type ShortcutId } from "@akatsuki/shortcuts";
import type { HTMLAttributes } from "react";
import { cx, firstKeys, useUiConfig } from "./config.tsx";

export interface KbdProps extends HTMLAttributes<HTMLElement> {
  /** A shortcut id from packages/shortcuts (preferred). */
  shortcut?: ShortcutId;
  /** Raw key notation (`"cmd+k"`, `"g i"`) for keys that are not shortcuts, e.g. in help text. */
  keys?: string;
}

/**
 * Renders a key hint, one `<kbd>` per step of a sequence ("G then I"). Renders nothing for a palette-only
 * command without keys.
 */
export function Kbd({ shortcut, keys, className, ...rest }: KbdProps) {
  const config = useUiConfig();
  const notation = keys ?? (shortcut ? firstKeys(shortcut, config) : null);
  if (!notation) return null;
  const steps = notation.trim().split(/ +/);
  return (
    <kbd className={cx("ak-kbd", className)} {...rest}>
      {/* Read once as words ("G then I"); the visible keycaps are hidden from assistive tech. */}
      <span className="ak-kbd__label">{formatKeys(notation, config.platform)}</span>
      {steps.map((step, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: steps are positional and may repeat ("g g")
        <span key={`${i}-${step}`} className="ak-kbd__step" aria-hidden="true">
          {i > 0 && <span className="ak-kbd__then">then</span>}
          <kbd className="ak-kbd__key">{formatKeys(step, config.platform)}</kbd>
        </span>
      ))}
    </kbd>
  );
}
