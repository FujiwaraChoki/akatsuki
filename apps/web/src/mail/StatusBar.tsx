// The status bar (First Light, SPEC §3.1.3): 28px under all three panes. Left, where you are ("Inbox · 12
// conversations"), or for four seconds what just happened. Right, the last keys you pressed (fading after a
// second) and the sync state. It replaces the persistent "connected" toast and never covers content.
import { maskAccount } from "@akatsuki/ui";
import { useEffect, useRef, useState } from "react";
import type { MailAccountView } from "../backend/types.ts";
import { useOffline } from "./Banners.tsx";
import { useEcho } from "./statusEcho.ts";

const HUD_MAX = 3;
const HUD_FADE_MS = 1000;
const HUD_GONE_MS = 1300;

interface HudKey {
  id: number;
  keys: string[];
  out: boolean;
}

const NAMED: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Enter: "↵",
  Escape: "Esc",
  " ": "Space",
  Backspace: "⌫",
};

/** The keycaps for one key press, or null for presses the HUD ignores (typing, lone modifiers). */
export function hudKeys(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">) {
  if (["Shift", "Control", "Meta", "Alt", "CapsLock", "Tab"].includes(e.key)) return null;
  const main =
    NAMED[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key.startsWith("F") ? e.key : null);
  if (!main) return null;
  const out: string[] = [];
  if (e.ctrlKey) out.push("Ctrl");
  if (e.metaKey) out.push("⌘");
  if (e.altKey) out.push("Alt");
  if (e.shiftKey && e.key.length !== 1) out.push("⇧");
  out.push(main);
  return out;
}

function typingIn(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null;
  return !!t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
}

function useKeyHud(): HudKey[] {
  const [keys, setKeys] = useState<HudKey[]>([]);
  const seq = useRef(0);
  useEffect(() => {
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const later = (ms: number, f: () => void) => {
      const t = setTimeout(() => {
        timers.delete(t);
        f();
      }, ms);
      timers.add(t);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.repeat || typingIn(e.target)) return;
      const k = hudKeys(e);
      if (!k) return;
      const id = ++seq.current;
      setKeys((cur) => [...cur, { id, keys: k, out: false }].slice(-HUD_MAX));
      later(HUD_FADE_MS, () => setKeys((cur) => cur.map((x) => (x.id === id ? { ...x, out: true } : x))));
      later(HUD_GONE_MS, () => setKeys((cur) => cur.filter((x) => x.id !== id)));
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      for (const t of timers) clearTimeout(t);
    };
  }, []);
  return keys;
}

export interface StatusBarProps {
  /** "Inbox", "Search", a label name. */
  title: string;
  count: number;
  /** The list is still loading or has more pages. */
  partial: boolean;
  accounts: readonly MailAccountView[];
  onReconnect: (a: MailAccountView) => void;
}

export function StatusBar({ title, count, partial, accounts, onReconnect }: StatusBarProps) {
  const echoText = useEcho();
  const hud = useKeyHud();
  const offline = useOffline();
  const reauth = accounts.find((a) => a.status === "reauth_required");
  const syncing = accounts.filter((a) => a.status === "initial_sync" || a.status === "connecting");
  const n = `${count}${partial && count > 0 ? "+" : ""}`;
  return (
    <footer className="ak-statusbar">
      <p className="ak-statusbar__left" data-echo={echoText ? "" : undefined} aria-live="polite">
        {echoText ?? `${title} · ${n} conversation${count === 1 ? "" : "s"}`}
      </p>
      <div className="ak-statusbar__hud" aria-hidden="true">
        {hud.map((k) => (
          <kbd key={k.id} className="ak-kbd" data-out={k.out || undefined}>
            {k.keys.map((x) => (
              <span key={x} className="ak-kbd__key">
                {x}
              </span>
            ))}
          </kbd>
        ))}
      </div>
      <p className="ak-statusbar__sync">
        {offline ? (
          <>
            <i className="ak-statusbar__ring" aria-hidden="true" />
            Offline · changes wait on this device
          </>
        ) : reauth ? (
          <>
            <b className="ak-statusbar__alert" aria-hidden="true">
              !
            </b>
            Sync paused for {maskAccount(reauth.email)} ·{" "}
            <button type="button" className="ak-textbutton" onClick={() => onReconnect(reauth)}>
              Reconnect
            </button>
          </>
        ) : syncing.length > 0 ? (
          <>
            <i className="ak-statusbar__dot" aria-hidden="true" />
            Syncing {syncing.length === 1 ? maskAccount(syncing[0]!.email) : `${syncing.length} accounts`}
          </>
        ) : (
          <>
            <i className="ak-statusbar__dot" aria-hidden="true" />
            {accounts.length === 0
              ? "No accounts"
              : `Synced · ${accounts.length} account${accounts.length === 1 ? "" : "s"}`}
          </>
        )}
      </p>
    </footer>
  );
}
