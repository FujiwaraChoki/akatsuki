// Keyboard glyphs. Platform-specific labels render both variants; CSS shows the right one
// (the boot script sets html[data-platform] before first paint, so nothing flickers).
// ⌘ ⌥ ⌃ are not in the brand fonts, so they render as inline SVG (lib/glyphs.ts).
import { Fragment, type ReactNode } from "react";
import { glyphHtml, hasGlyph } from "@/lib/glyphs";
import { keysFor, type Plat, type ShortcutDef, sequenceCaps, shortcut } from "@/lib/shortcuts";

export function Kbd({ children, className = "" }: { children: ReactNode; className?: string }) {
  const cls = `kbd ${className}`.trim();
  if (typeof children === "string" && hasGlyph(children)) {
    // biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG glyphs from lib/glyphs.ts, no user input
    return <kbd className={cls} dangerouslySetInnerHTML={{ __html: glyphHtml(children) }} />;
  }
  return <kbd className={cls}>{children}</kbd>;
}

/** Inline ⌘ glyph (SVG), for places that are not a kbd. */
export function Cmd() {
  // biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG glyphs from lib/glyphs.ts, no user input
  return <span className="gl-wrap" dangerouslySetInnerHTML={{ __html: glyphHtml("⌘") }} />;
}

/** ⌘ on macOS, Ctrl elsewhere. */
export function Mod({ className = "" }: { className?: string }) {
  return (
    <kbd className={`kbd ${className}`.trim()}>
      <span className="p-mac">
        <Cmd />
      </span>
      <span className="p-other">Ctrl</span>
    </kbd>
  );
}

/** "⌘K" / "Ctrl K" as plain text, no keycap. */
export function ModText({ k }: { k: string }) {
  return (
    <>
      <span className="p-mac">
        <Cmd />
        {k}
      </span>
      <span className="p-other">Ctrl {k}</span>
    </>
  );
}

function Steps({ keys, plat }: { keys: string; plat: Plat }) {
  const steps = sequenceCaps(keys, plat);
  return (
    <span className="kseq">
      {steps.map((caps, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: keycaps of one shortcut, a fixed sequence
        <Fragment key={i}>
          {i > 0 && <span className="sc-then">then</span>}
          {caps.map((c, j) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: keycaps of one shortcut, a fixed sequence
            <Kbd key={j}>{c}</Kbd>
          ))}
        </Fragment>
      ))}
    </span>
  );
}

function Alternatives({ s, plat }: { s: ShortcutDef; plat: Plat }) {
  const list = keysFor(s, plat);
  return (
    <span className="kseq">
      {list.map((k, i) => (
        <Fragment key={k}>
          {i > 0 && <span className="sc-or">or</span>}
          <Steps keys={k} plat={plat} />
        </Fragment>
      ))}
    </span>
  );
}

/** Every key binding of one shortcut, from shortcuts.json, platform-aware. */
export function Keys({ id, def }: { id?: string; def?: ShortcutDef }) {
  const s = def ?? shortcut(id ?? "");
  const mac = keysFor(s, "mac").join("|");
  const other = keysFor(s, "other").join("|");
  const same =
    sequenceCaps(mac.replace(/\|/g, " "), "mac").flat().join() ===
    sequenceCaps(other.replace(/\|/g, " "), "other").flat().join();
  if (same) return <Alternatives s={s} plat="other" />;
  return (
    <>
      <span className="p-mac">
        <Alternatives s={s} plat="mac" />
      </span>
      <span className="p-other">
        <Alternatives s={s} plat="other" />
      </span>
    </>
  );
}
