// Hardware keyboards (iPad, Android tablets, phones with a Bluetooth keyboard). Key bindings come only from
// packages/shortcuts; this module adapts native key events to its `KeyEventLike` and lists which shortcuts the
// mobile app implements, so the palette never offers an action that does nothing.
import {
  createMatcher,
  detectPlatform,
  type KeyEventLike,
  type Keymap,
  type MatchResult,
  type PaletteCommand,
  type Platform,
  paletteCommands,
  type Scope,
  SHORTCUTS,
} from "@akatsuki/shortcuts";

/** The native event shape (expo-key-event: `key` is a DOM `code`, `character` the produced text). */
export interface NativeKeyEvent {
  key: string;
  character?: string | null;
  shiftKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
}

const NAMED: Record<string, string> = {
  Enter: "Enter",
  NumpadEnter: "Enter",
  Escape: "Escape",
  Tab: "Tab",
  Space: " ",
  Backspace: "Backspace",
  Delete: "Delete",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
};

const SHIFTED_DIGITS = ")!@#$%^&*(";
const PUNCT: Record<string, [string, string]> = {
  Comma: [",", "<"],
  Period: [".", ">"],
  Slash: ["/", "?"],
  Semicolon: [";", ":"],
  Quote: ["'", '"'],
  BracketLeft: ["[", "{"],
  BracketRight: ["]", "}"],
  Backslash: ["\\", "|"],
  Minus: ["-", "_"],
  Equal: ["=", "+"],
  Backquote: ["`", "~"],
};

/** Converts a native key event to the structural DOM-like event `@akatsuki/shortcuts` matches on. */
export function toKeyEventLike(e: NativeKeyEvent): KeyEventLike {
  const code = e.key;
  const shift = !!e.shiftKey;
  let key = NAMED[code];
  if (!key) {
    const ch = e.character;
    // Control characters (ctrl+letter on some keyboards) are not useful as keys; fall back to the code.
    if (ch && ch.length === 1 && ch >= " ") key = ch;
    else if (/^Key[A-Z]$/.test(code)) key = shift ? code.slice(3) : code.slice(3).toLowerCase();
    else if (/^Digit[0-9]$/.test(code)) key = shift ? SHIFTED_DIGITS[Number(code.slice(5))]! : code.slice(5);
    else if (PUNCT[code]) key = PUNCT[code][shift ? 1 : 0];
    else key = code;
  }
  return {
    key,
    code,
    metaKey: !!e.metaKey,
    ctrlKey: !!e.ctrlKey,
    altKey: !!e.altKey,
    shiftKey: shift,
  };
}

/** The key-hint platform: iPadOS uses ⌘ like macOS; Android and others use Ctrl. */
export function keyPlatform(os: string, userAgent?: string): Platform {
  if (os === "ios" || os === "macos") return "mac";
  if (os === "web" && userAgent) return detectPlatform(userAgent);
  return "linux";
}

/** Shortcut ids the mobile app implements (everything else is hidden from the palette and ignored). */
export const MOBILE_SHORTCUTS: ReadonlySet<string> = new Set([
  "palette.open",
  "help.shortcuts",
  "compose.new",
  "search.open",
  "undo",
  "settings.open",
  "go.inbox",
  "go.starred",
  "go.sent",
  "go.drafts",
  "go.snoozed",
  "go.reminders",
  "go.all",
  "go.spam",
  "go.trash",
  "go.label",
  "split.next",
  "split.prev",
  "nav.next",
  "nav.prev",
  "nav.open",
  "nav.back",
  "thread.archive",
  "thread.moveToInbox",
  "thread.snooze",
  "thread.remind",
  "thread.reply",
  "thread.replyAll",
  "thread.forward",
  "thread.trash",
  "thread.spam",
  "thread.toggleRead",
  "thread.star",
  "thread.label",
  "thread.move",
  "thread.unsubscribe",
  "thread.expandAll",
  "thread.showImages",
  "ai.summarize",
  "ai.instantReply",
  "compose.send",
  "compose.sendLater",
  "compose.discard",
  "compose.close",
  "compose.cc",
  "compose.bcc",
  "compose.attach",
  "search.close",
  "search.submit",
  "palette.close",
  "palette.next",
  "palette.prev",
  "palette.run",
]);

/** Every shortcut in the shared map is either implemented here or deliberately left out (tested). */
export const MOBILE_UNSUPPORTED: ReadonlySet<string> = new Set(
  SHORTCUTS.map((s) => s.id).filter((id) => !MOBILE_SHORTCUTS.has(id)),
);

/** Palette commands available on mobile in the current scopes. */
export function mobileCommands(
  platform: Platform,
  keymap: Keymap,
  active: ReadonlySet<Scope>,
  isAvailable: (id: string) => boolean = () => true,
): PaletteCommand[] {
  return paletteCommands(platform, keymap).filter((c) => {
    if (!MOBILE_SHORTCUTS.has(c.id) || !isAvailable(c.id)) return false;
    if (c.scope === "global") return true;
    if (c.scope === "mail") return active.has("list") || active.has("thread");
    return active.has(c.scope);
  });
}

/** A matcher limited to the shortcuts mobile implements. */
export function createMobileMatcher(platform: Platform, keymap: Keymap) {
  const matcher = createMatcher({ platform, keymap });
  return {
    reset: matcher.reset,
    handle(e: KeyEventLike, active: ReadonlySet<Scope>): MatchResult {
      const r = matcher.handle(e, active);
      if (r.kind === "match" && !MOBILE_SHORTCUTS.has(r.id)) return { kind: "none" };
      return r;
    },
  };
}
