// Pure string renderers for the demo inbox. The server uses them for the static first paint
// (which is also the no-JS version) and the client uses the same functions after every action,
// so the two can never disagree.
import { glyphHtml } from "./glyphs";
import { EMPTY_VIEWS, type Split, THREADS, type Thread, VIEWS, type ViewId } from "./inbox-fixture";

export type Loc = "inbox" | "archived" | "snoozed" | "trash";
export interface TState extends Thread {
  loc: Loc;
}
export interface InboxState {
  threads: TState[];
  split: Split;
  view: ViewId;
  cursor: string | null;
  picked: string[];
  query: string;
  /** Words of the summary shown while it streams; null means the whole summary. */
  sumWords: number | null;
  /** Inbox-zero tally, filled by the controller. */
  tally?: { keys: number; clicks: number };
}

export function initialState(): InboxState {
  return {
    threads: THREADS.map((t) => ({ ...t, loc: "inbox" as Loc })),
    split: "Important",
    view: "inbox",
    cursor: "t1",
    picked: [],
    query: "",
    sumWords: null,
  };
}

export const esc = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// ───────────── glyphs (12px, 1.25 stroke) ─────────────

const P: Record<string, string> = {
  search: '<circle cx="5.25" cy="5.25" r="3.75"/><path d="M8 8l3 3"/>',
  clip: '<path d="M10 5.5 6 9.5a2.1 2.1 0 0 1-3-3l4.5-4.5a1.4 1.4 0 0 1 2 2L5 8.5a.7.7 0 0 1-1-1L8 3.5"/>',
  star: '<path d="M6 1.2l1.45 3 3.3.45-2.4 2.3.6 3.25L6 8.65 3.05 10.2l.6-3.25-2.4-2.3 3.3-.45z"/>',
  check: '<path d="M2.2 6.3 4.8 8.8 9.8 3.4"/>',
  chevron: '<path d="M3.5 4.75 6 7.25l2.5-2.5"/>',
  plus: '<path d="M6 2v8M2 6h8"/>',
  arrow: '<path d="M2 6h8M7 3l3 3-3 3"/>',
  archive: '<rect x="1.5" y="2" width="9" height="2.5" rx=".5"/><path d="M2.5 4.5v5h7v-5M4.75 6.5h2.5"/>',
  clock: '<circle cx="6" cy="6" r="4.5"/><path d="M6 3.5V6l1.75 1.25"/>',
  replay: '<path d="M2.5 6a3.5 3.5 0 1 0 1.1-2.55"/><path d="M2.3 1.8v2.1h2.1"/>',
  compose: '<path d="M8.25 1.75l2 2L4.5 9.5H2.5v-2z"/>',
};
export function icon(name: keyof typeof P | string, cls = ""): string {
  return `<svg class="ico ${cls}" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P[name] ?? ""}</svg>`;
}

export const kbd = (k: string, cls = ""): string => `<kbd class="kbd ${cls}">${glyphHtml(k)}</kbd>`;

// ───────────── selectors ─────────────

export function visible(s: InboxState): TState[] {
  let list: TState[];
  if (s.view === "inbox") list = s.threads.filter((t) => t.loc === "inbox" && t.split === s.split);
  else if (s.view === "snoozed") list = s.threads.filter((t) => t.loc === "snoozed");
  else if (s.view === "starred") list = s.threads.filter((t) => t.starred && t.loc !== "trash");
  else list = [];
  if (s.query.trim()) list = s.threads.filter((t) => t.loc !== "trash" && matches(t, s.query));
  return list;
}

/** Tiny operator search over the fixture: from:, has:attachment, is:unread, is:starred and words. */
export function matches(t: TState, q: string): boolean {
  return q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((term) => {
      if (term.startsWith("from:")) {
        const v = term.slice(5);
        return !v || t.from.toLowerCase().includes(v) || t.addr.includes(v);
      }
      if (term === "has:attachment") return t.clip;
      if (term.startsWith("has:")) return "has:attachment".startsWith(term);
      if (term === "is:unread") return t.unread;
      if (term === "is:starred") return t.starred;
      if (term.startsWith("is:")) return "is:unread".startsWith(term) || "is:starred".startsWith(term);
      return `${t.from} ${t.subject} ${t.snippet}`.toLowerCase().includes(term);
    });
}

export function splitCount(s: InboxState, split: Split): number {
  return s.threads.filter((t) => t.loc === "inbox" && t.split === split).length;
}
/** The sidebar's Inbox count: conversations left in the Important split. */
export function inboxCount(s: InboxState): number {
  return s.threads.filter((t) => t.loc === "inbox" && t.split === "Important").length;
}
export function snoozedCount(s: InboxState): number {
  return 2 + s.threads.filter((t) => t.loc === "snoozed").length;
}

export function current(s: InboxState): TState | undefined {
  const list = visible(s);
  return list.find((t) => t.id === s.cursor) ?? list[0];
}

// ───────────── rows ─────────────

export function rowLabel(t: TState): string {
  return [t.from, t.subject, t.time, t.unread ? "unread" : "read", t.starred ? "starred" : ""]
    .filter(Boolean)
    .join(", ");
}

function row(t: TState, sel: boolean, picked: boolean): string {
  const cls = ["ir", t.unread ? "is-unread" : "", sel ? "is-sel" : "", picked ? "is-picked" : ""]
    .join(" ")
    .trim();
  const meta = [
    t.chip ? `<span class="chip">${esc(t.chip)}</span>` : "",
    t.clip ? icon("clip", "ir-clip") : "",
    t.starred ? icon("star", "ir-star") : "",
  ].join("");
  const hints = sel
    ? `<span class="ir-hints" aria-hidden="true" title="E archive · H snooze · R reply">${kbd("E")}${kbd("H")}${kbd("R")}</span>`
    : "";
  return `<li role="option" id="ir-${t.id}" class="${cls}" data-id="${t.id}" aria-selected="${sel}" aria-label="${esc(rowLabel(t))}">
<span class="ir-dot" aria-hidden="true">${picked ? icon("check") : ""}</span>
<span class="ir-from">${esc(t.from)}</span>
<span class="ir-text"><span class="ir-subj">${esc(t.subject)}</span><span class="ir-sep"> — </span><span class="ir-snip">${esc(t.snippet)}</span></span>
<span class="ir-meta">${meta}</span>
<span class="ir-time">${sel ? hints : ""}<span class="ir-clock">${esc(t.time)}</span></span>
<span class="ir-acts"><button type="button" class="ir-act" data-act="archive" aria-label="Archive ${esc(t.subject)}" tabindex="-1">${icon("archive")}</button><button type="button" class="ir-act" data-act="snooze" aria-label="Snooze ${esc(t.subject)}" tabindex="-1">${icon("clock")}</button><button type="button" class="ir-act" data-act="star" aria-label="${t.starred ? "Unstar" : "Star"} ${esc(t.subject)}" tabindex="-1">${icon("star")}</button></span>
</li>`;
}

export function rowsHtml(s: InboxState): string {
  const list = visible(s);
  const cur = current(s);
  return list.map((t) => row(t, t.id === cur?.id, s.picked.includes(t.id))).join("");
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

export function emptyHtml(s: InboxState): string {
  if (s.query.trim()) {
    return `<div class="inbox-empty"><p class="inbox-empty-title">No matches.</p><p class="inbox-empty-sub">Try from:hana, has:attachment or is:unread.</p></div>`;
  }
  if (s.view === "inbox" && s.split === "Important") {
    const k = s.tally?.keys ?? 0;
    const c = s.tally?.clicks ?? 0;
    return `<div class="inbox-zero">
<div class="iz-dawn" aria-hidden="true"><span class="dawn-wash"></span><span class="dawn-grain"></span><span class="dawn-line"></span></div>
<div class="iz-copy"><p class="iz-title">Inbox zero</p>
<p class="iz-tally">You pressed ${plural(k, "key", "keys")} and clicked ${plural(c, "time", "times")}.</p>
<p class="iz-hint">Press ${kbd("Z")} to bring the last one back.</p>
<a class="tlink iz-link" href="#waitlist" data-waitlist>Join the waitlist <span class="arr">→</span></a></div></div>`;
  }
  const msg = s.view === "inbox" ? `Nothing in ${s.split}.` : (EMPTY_VIEWS[s.view] ?? "Nothing here.");
  return `<div class="inbox-empty"><p class="inbox-empty-title">${esc(msg)}</p><p class="inbox-empty-sub">Press G then I to go back to the inbox.</p></div>`;
}

// ───────────── reading pane ─────────────

export function paneHtml(s: InboxState): string {
  const t = current(s);
  if (!t) return `<div class="rp-none"><p>No conversation selected.</p></div>`;
  const words = t.summary.join(" ").split(" ");
  const shown = s.sumWords == null ? words.join(" ") : words.slice(0, s.sumWords).join(" ");
  const streaming = s.sumWords != null && s.sumWords < words.length;
  const crumb =
    s.view === "inbox" ? `Inbox · ${t.split}` : (VIEWS.find((v) => v.id === s.view)?.label ?? s.view);
  return `<p class="rp-crumb">${esc(crumb)}</p>
<p class="rp-subj">${esc(t.subject)}</p>
<p class="rp-from">${esc(t.from)} · to me · ${esc(t.time)}</p>
<div class="rp-sum"><p class="rp-sum-label">Summary · llama3.3<span class="rp-sum-where"> · on this device</span></p><p class="rp-sum-text${streaming ? " is-streaming" : ""}">${esc(shown)}</p></div>
<div class="rp-body">${t.body.map((p) => `<p>${esc(p)}</p>`).join("")}</div>`;
}
