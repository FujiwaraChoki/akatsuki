// The key map on a quiet keyboard (§5.4). Real key presses light their key while the map is in view
// and the inbox is not taking keys; the caption under the board names the command. Exactly one key
// (and on phones one specimen row) stays lit. Keys and rows are buttons too. On first view (motion allowed, and only if the
// map was below the fold when the visitor began scrolling) it plays J, J, E, H and rests on Snooze.
import { addLayer } from "./router";
import { page } from "./state";
import { $, $$, after, isMac, isMod, reducedMotion, watch } from "./util";

const NAMED: Record<string, string> = {
  Escape: "escape",
  Tab: "tab",
  Enter: "enter",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Backspace: "backspace",
  CapsLock: "capslock",
  " ": " ",
};
/** Shifted symbols back to the key that carries them. */
const UNSHIFT: Record<string, string> = {
  "!": "1",
  "#": "3",
  "?": "/",
  "@": "2",
  $: "4",
  "%": "5",
  "^": "6",
  "&": "7",
  "*": "8",
  "(": "9",
  ")": "0",
  _: "-",
  "+": "=",
  "{": "[",
  "}": "]",
  "|": "\\",
  ":": ";",
  '"': "'",
  "<": ",",
  ">": ".",
};
/** Glyph for the readout's kbd. */
const CAP: Record<string, string> = {
  escape: "Esc",
  tab: "Tab",
  enter: "↵",
  up: "↑",
  down: "↓",
  "3": "#",
  "1": "!",
};

export function initKeys(root: HTMLElement) {
  const keys = $$<HTMLElement>(".km-board [data-k]", root);
  const specs = $$<HTMLElement>(".km-spec", root);
  const out = $("[data-km-out]")!;
  const outKeys = $("[data-km-keys]", out)!;
  const outVerb = $("[data-km-verb]", out)!;
  const outSay = $("[data-km-say]", out)!;
  let acts: Record<string, [string, string]> = {};
  try {
    acts = JSON.parse($("[data-km-acts]", root)?.dataset.kmActs ?? "{}");
  } catch {
    // The readout keeps its server-rendered text.
  }
  let ratio = 0;
  let demoed = false;
  /** The auto-demo's pending presses; any real press cancels the rest. */
  let demo: (() => void)[] = [];
  const stopDemo = () => {
    demoed = true;
    for (const c of demo) c();
    demo = [];
  };
  let primed = false;
  // The server render lights E (and the Archive row on phones); start from that.
  const lit = new Set<HTMLElement>($$<HTMLElement>(".is-lit", root));

  const all = (k: string) => keys.filter((x) => x.dataset.k === k);
  const modKey = () => (isMac() ? "meta" : "ctrl");

  const pressAnim = (els: HTMLElement[]) => {
    for (const el of els) {
      for (const cap of el.classList.contains("km-spec") ? $$<HTMLElement>(".km-cap", el) : [el]) {
        cap.classList.remove("is-pressed");
        void cap.offsetWidth;
        cap.classList.add("is-pressed");
        after(140, () => cap.classList.remove("is-pressed"));
      }
    }
  };

  const light = (els: HTMLElement[]) => {
    for (const x of lit) x.classList.remove("is-lit");
    lit.clear();
    for (const x of els) {
      x.classList.add("is-lit");
      lit.add(x);
    }
  };

  const kbd = (t: string) => {
    const k = document.createElement("kbd");
    k.className = "kbd";
    k.textContent = t;
    return k;
  };

  /** Press a key. Returns true when it maps to an action (the readout changed). */
  function press(k: string, shift: boolean, mod: boolean, show = true): boolean {
    const els = all(k);
    const extra: HTMLElement[] = [];
    if (shift) extra.push(...all("shift"));
    if (mod) extra.push(...all(modKey()));
    const first = els[0];
    let act = "";
    if (mod && k === "k") act = "palette";
    else if (shift && first?.dataset.sact) act = first.dataset.sact;
    else if (!shift) act = first?.dataset.act ?? "";
    const rows = specs.filter((x) => (x.dataset.acts ?? "").split(" ").includes(act));
    pressAnim([...els, ...extra, ...rows]);
    const data = act ? acts[act] : undefined;
    if (!data || !show) return !!data;
    light([...els, ...extra, ...rows]);
    const caps: string[] = [];
    if (mod) caps.push(isMac() ? "⌘" : "Ctrl");
    if (shift) caps.push("⇧");
    caps.push(CAP[k] ?? k.toUpperCase());
    outKeys.replaceChildren(...caps.map(kbd));
    outVerb.textContent = data[0];
    outSay.textContent = data[1];
    return true;
  }

  watch(root, [0, 0.1, 0.2, 0.4, 0.5], (en) => {
    ratio = en.intersectionRatio;
    if (!demoed && primed && ratio >= 0.5) {
      demoed = true;
      demo = ["j", "j", "e", "h"].map((k, i) => after(400 + i * 650, () => press(k, false, false)));
    }
  });

  for (const el of [...keys, ...specs]) {
    el.addEventListener("click", () => {
      const k = el.dataset.k ?? "";
      // A key whose only action is shifted (#, !) acts as if Shift were held.
      const shifted = !el.dataset.act && !!el.dataset.sact && !el.classList.contains("km-spec");
      stopDemo();
      press(k, shifted, el.dataset.mod !== undefined);
    });
  }

  if (!reducedMotion()) {
    page.onArm.push(() => {
      if (root.getBoundingClientRect().top >= window.innerHeight) primed = true;
    });
  }

  addLayer(5, (e) => {
    if (ratio < 0.2 || page.inboxLive || page.inboxFocused) return false;
    stopDemo();
    if (isMod(e) && e.key.toLowerCase() === "k") {
      press("k", false, true);
      return false; // the site palette opens as normal (global layer)
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    const raw = NAMED[e.key] ?? (e.key.length === 1 ? e.key.toLowerCase() : "");
    if (!raw) return false;
    const k = UNSHIFT[raw] ?? raw;
    if (raw === "?") {
      press("/", true, false);
      return false; // the shortcuts sheet opens as normal
    }
    // A shifted symbol (#, !) means Shift was held, whatever the event says.
    const shifted = (e.shiftKey || raw in UNSHIFT) && !["escape", "tab", "enter"].includes(k);
    const hit = press(k, shifted, false);
    // Page keys keep working: only plain letters and symbols that map to an action are consumed.
    if (!hit || ["g", "w", " ", "tab", "enter", "escape", "up", "down", "left", "right"].includes(k))
      return false;
    if (k === "/") e.preventDefault();
    return true;
  });
}
