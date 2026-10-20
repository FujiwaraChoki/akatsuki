// The working demo inbox (§4). State lives in one object; every action mutates it and re-renders
// synchronously, so the screen changes on the next frame. Key bindings come from shortcuts.json.
import shortcutsData from "@akatsuki/shortcuts/shortcuts.json";
import { glyphHtml } from "@/lib/glyphs";
import { SNOOZE_OPTIONS, SPLITS, type ViewId } from "@/lib/inbox-fixture";
import {
  current,
  emptyHtml,
  esc,
  type InboxState,
  inboxCount,
  initialState,
  paneHtml,
  rowsHtml,
  snoozedCount,
  splitCount,
  visible,
} from "@/lib/inbox-render";
import { addLayer } from "./router";
import { openSheet } from "./site";
import { page } from "./state";
import { $, $$, after, capOf, comboOf, isEditable, isMac, isMod, reducedMotion, watch } from "./util";

interface Def {
  id: string;
  label: string;
  palette: boolean;
  keys: { default: string[]; mac?: string[]; linux?: string[]; windows?: string[] };
}
const DEFS = shortcutsData.shortcuts as Def[];
const LABEL = new Map(DEFS.map((d) => [d.id, d.label]));

/** combo or sequence string → shortcut id, for this platform. */
function keyMap(): Map<string, string> {
  const m = new Map<string, string>();
  const mac = isMac();
  for (const d of DEFS) {
    const list = (mac ? d.keys.mac : (d.keys.linux ?? d.keys.windows)) ?? d.keys.default;
    for (const k of list) if (!m.has(k)) m.set(k, d.id);
  }
  return m;
}

const VIEW_OF: Record<string, ViewId> = {
  "go.inbox": "inbox",
  "go.starred": "starred",
  "go.snoozed": "snoozed",
  "go.reminders": "reminders",
  "go.sent": "sent",
  "go.drafts": "drafts",
};
const VIEW_NAME: Record<ViewId, string> = {
  inbox: "Inbox",
  starred: "Starred",
  snoozed: "Snoozed",
  reminders: "Reminders",
  sent: "Sent",
  drafts: "Drafts",
};

export function initInbox(root: HTMLElement) {
  const stage = $("[data-stage]", root)!;
  const frame = $("[data-frame]", root)!;
  const rowsEl = $("[data-rows]", root)!;
  const emptyEl = $("[data-empty]", root)!;
  const listEl = $("[data-list]", root)!;
  const paneEl = $("[data-pane]", root)!;
  const statusEl = $("[data-status]", root)!;
  const liveEl = $("[data-live]", root)!;
  const chipEl = $("[data-live-chip]", root)!;
  const hudEl = $("[data-hud]", root)!;
  const tallyEl = $("[data-tally]", root);
  const popEl = $("[data-pop]", root)!;
  const popHead = $("[data-pop-head]", root)!;
  const sheetEl = $("[data-isheet]", root)!;
  const sheetPane = $("[data-isheet-pane]", root)!;
  const palEl = $("[data-ipal]", root)!;
  const palIn = $<HTMLInputElement>("[data-ipal-in]", root)!;
  const palRows = $$<HTMLButtonElement>("[data-cmd-id]", root);
  const composeEl = $<HTMLFormElement>("[data-compose]", root)!;
  const searchBox = $("[data-searchbox]", root)!;
  const searchIn = $<HTMLInputElement>("[data-search-in]", root)!;
  const searchOps = $("[data-search-ops]", root)!;
  const replyEl = $("[data-reply]", root)!;
  const replyText = $("[data-reply-text]", root)!;
  const map = keyMap();
  const motion = !reducedMotion();

  let s: InboxState = initialState();
  let history: InboxState[] = [];
  const tally = { keys: 0, clicks: 0 };
  let timedSeq = 0;
  let gPending = false;
  let clearG: (() => void) | null = null;
  let pop: null | { kind: "snooze" | "remind"; hi: number; ids: string[] } = null;
  let palSel = 0;
  let replying = false;
  let pendingSend: (() => void) | null = null;
  let lastEmpty = "";
  let ratio = 0;
  let topVisible = false;
  let visibleH = 0;
  const ghost = { running: false, done: false, stepped: false, startY: 0, cancel: [] as (() => void)[] };
  const idleStatus = statusEl.innerHTML;
  let streamCancel: (() => void) | null = null;

  const snapshot = (): InboxState => ({
    ...s,
    threads: s.threads.map((t) => ({ ...t })),
    picked: [...s.picked],
  });
  const push = () => {
    history.push(snapshot());
    if (history.length > 50) history.shift();
  };
  const narrow = () => window.innerWidth < 1100;
  const phone = () => window.innerWidth < 768;

  // ───────────── render ─────────────

  function render() {
    s.tally = { ...tally };
    const list = visible(s);
    if (!list.find((t) => t.id === s.cursor)) s.cursor = list[0]?.id ?? null;
    rowsEl.innerHTML = rowsHtml(s);
    rowsEl.setAttribute(
      "aria-label",
      `Demo inbox, ${s.view === "inbox" ? `${s.split} split` : VIEW_NAME[s.view]}`,
    );
    if (s.cursor) rowsEl.setAttribute("aria-activedescendant", `ir-${s.cursor}`);
    else rowsEl.removeAttribute("aria-activedescendant");
    if (list.length === 0) {
      const html = emptyHtml(s);
      const kind = html.slice(0, 40) + s.view + s.split + s.query;
      if (kind !== lastEmpty || emptyEl.hidden) {
        emptyEl.innerHTML = html;
        lastEmpty = kind;
      } else {
        const tl = $(".iz-tally", emptyEl);
        const fresh = /<p class="iz-tally">(.*?)<\/p>/.exec(html)?.[1];
        if (tl && fresh) tl.textContent = fresh;
      }
      emptyEl.hidden = false;
    } else {
      emptyEl.hidden = true;
      lastEmpty = "";
    }
    paneEl.innerHTML = paneHtml(s);
    if (!sheetEl.hidden) sheetPane.innerHTML = paneHtml(s);
    for (const sp of SPLITS) {
      const n = $(`[data-split-count="${sp}"]`, root);
      if (n) n.textContent = String(splitCount(s, sp));
      const tab = $(`[data-split="${sp}"]`, root);
      tab?.setAttribute("aria-selected", String(s.view === "inbox" && s.split === sp));
      if (tab) tab.tabIndex = s.split === sp ? 0 : -1;
    }
    const inboxN = $('[data-count="inbox"]', root);
    if (inboxN) inboxN.textContent = String(inboxCount(s));
    const snN = $('[data-count="snoozed"]', root);
    if (snN) snN.textContent = String(snoozedCount(s));
    for (const b of $$("[data-view]", root)) b.classList.toggle("is-active", b.dataset.view === s.view);
  }

  function status(text: string) {
    statusEl.textContent = text;
    statusEl.classList.add("is-echo");
    liveEl.textContent = text;
  }

  // ───────────── measured timing, shown in the status bar ─────────────
  // Measured, never claimed: the time from handling the input to the first animation frame after
  // the DOM changed. "next frame" only appears when that is what happened.
  function log(_cap: string, _action: string, _thread: string, started: number) {
    const text = statusEl.textContent ?? "";
    const token = ++timedSeq;
    requestAnimationFrame(() => {
      if (token !== timedSeq || statusEl.textContent !== text) return;
      const dt = performance.now() - started;
      const frames = dt <= 17 ? 1 : Math.ceil(dt / 16.7);
      const label = frames === 1 ? "next frame" : `${frames} frames`;
      const [head, ...tail] = text.split(" · ");
      statusEl.innerHTML = `${esc(head ?? "")} · on screen <span class="ib-frame" title="${dt.toFixed(1)} ms after the key">${label}</span>${tail.length ? ` · ${esc(tail.join(" · "))}` : ""}`;
    });
  }

  // ───────────── actions ─────────────

  const subjOf = (id: string | null) => s.threads.find((t) => t.id === id);
  const targets = () => (s.picked.length ? [...s.picked] : s.cursor ? [s.cursor] : []);

  function moveCursorAfterRemoval(ids: string[]) {
    const list = visible(s);
    const idx = list.findIndex((t) => t.id === s.cursor);
    const rest = list.filter((t) => !ids.includes(t.id));
    const after = list.slice(idx + 1).find((t) => !ids.includes(t.id));
    const before = list
      .slice(0, Math.max(0, idx))
      .reverse()
      .find((t) => !ids.includes(t.id));
    s.cursor = (after ?? before ?? rest[0])?.id ?? null;
  }

  function relocate(ids: string[], loc: "archived" | "snoozed" | "trash") {
    if (!ids.length) return false;
    push();
    moveCursorAfterRemoval(ids);
    for (const t of s.threads) if (ids.includes(t.id)) t.loc = loc;
    s.picked = [];
    return true;
  }

  function describe(ids: string[]): string {
    if (ids.length > 1) return `${ids.length} conversations`;
    return `“${subjOf(ids[0] ?? null)?.subject ?? ""}”`;
  }

  /** Runs a shortcut id. Returns a log entry when the action changed the inbox. */
  function run(id: string): { action: string; thread: string } | null {
    const cur = current(s);
    const thread = cur ? `${cur.from} — ${cur.subject}` : "";
    replying && id !== "thread.reply" && stopReply();
    switch (id) {
      case "nav.next":
      case "nav.prev": {
        const list = visible(s);
        if (!list.length) return null;
        const i = list.findIndex((t) => t.id === s.cursor);
        const n = Math.max(0, Math.min(list.length - 1, i + (id === "nav.next" ? 1 : -1)));
        s.cursor = list[n]?.id ?? null;
        s.sumWords = null;
        render();
        return null;
      }
      case "select.extendDown":
      case "select.extendUp": {
        if (s.cursor && !s.picked.includes(s.cursor)) s.picked.push(s.cursor);
        run(id === "select.extendDown" ? "nav.next" : "nav.prev");
        if (s.cursor && !s.picked.includes(s.cursor)) s.picked.push(s.cursor);
        render();
        status(`${s.picked.length} selected`);
        return null;
      }
      case "nav.open": {
        if (!cur) return null;
        cur.unread = false;
        if (narrow()) openSheetPane();
        render();
        status(`Opened “${cur.subject}”`);
        return { action: "Open", thread };
      }
      case "nav.back":
        return back() ? null : null;
      case "thread.archive": {
        const ids = targets();
        const what = describe(ids);
        if (!relocate(ids, "archived")) return null;
        closeSheetPane();
        render();
        status(`Archived ${what} · Z to undo`);
        return { action: "Archive", thread: ids.length > 1 ? what : thread };
      }
      case "thread.trash": {
        const ids = targets();
        const what = describe(ids);
        if (!relocate(ids, "trash")) return null;
        closeSheetPane();
        render();
        status(`Moved ${what} to Trash · Z to undo`);
        return { action: "Trash", thread: ids.length > 1 ? what : thread };
      }
      case "thread.snooze":
      case "thread.remind":
        if (!cur && !s.picked.length) return null;
        openPop(id === "thread.snooze" ? "snooze" : "remind");
        return null;
      case "thread.star": {
        const ids = targets();
        if (!ids.length) return null;
        push();
        const on = !subjOf(ids[0] ?? null)?.starred;
        for (const t of s.threads) if (ids.includes(t.id)) t.starred = on;
        render();
        status(`${on ? "Starred" : "Unstarred"} ${describe(ids)}`);
        return { action: on ? "Star" : "Unstar", thread };
      }
      case "thread.toggleRead": {
        const ids = targets();
        if (!ids.length) return null;
        push();
        const unread = !subjOf(ids[0] ?? null)?.unread;
        for (const t of s.threads) if (ids.includes(t.id)) t.unread = unread;
        render();
        status(`Marked ${describe(ids)} as ${unread ? "unread" : "read"}`);
        return { action: unread ? "Unread" : "Read", thread };
      }
      case "select.toggle": {
        if (!s.cursor) return null;
        s.picked = s.picked.includes(s.cursor)
          ? s.picked.filter((x) => x !== s.cursor)
          : [...s.picked, s.cursor];
        render();
        status(
          s.picked.length ? `${s.picked.length} selected · E and H act on all of them` : "Selection cleared",
        );
        return null;
      }
      case "select.all": {
        s.picked = visible(s).map((t) => t.id);
        render();
        status(`${s.picked.length} selected`);
        return null;
      }
      case "split.next":
      case "split.prev": {
        const i = SPLITS.indexOf(s.split);
        s.split = SPLITS[(i + (id === "split.next" ? 1 : SPLITS.length - 1)) % SPLITS.length]!;
        s.view = "inbox";
        s.cursor = visible(s)[0]?.id ?? null;
        s.picked = [];
        render();
        status(`${s.split} split`);
        return null;
      }
      case "ai.summarize": {
        if (!cur) return null;
        stream();
        status(`Summarising “${cur.subject}” with llama3.3 on this device`);
        return { action: "Summarise", thread };
      }
      case "thread.reply":
      case "thread.replyAll":
      case "ai.instantReply": {
        if (!cur) return null;
        if (narrow()) openSheetPane();
        startReply(cur.from, id === "ai.instantReply");
        return null;
      }
      case "undo": {
        if (pendingSend) {
          pendingSend();
          pendingSend = null;
          status("Send undone. The draft is back.");
          openCompose();
          return { action: "Undo send", thread: "" };
        }
        const prev = history.pop();
        if (!prev) {
          status("Nothing to undo");
          return null;
        }
        // Name what came back: the threads whose place or flags differ between now and then.
        const changed = prev.threads.filter((t) => {
          const now = s.threads.find((x) => x.id === t.id);
          return now && (now.loc !== t.loc || now.starred !== t.starred || now.unread !== t.unread);
        });
        s = prev;
        lastEmpty = "";
        render();
        const back = changed.length === 1 ? `“${changed[0]!.subject}”` : `${changed.length} conversations`;
        const moved = changed.some((t) => t.loc === "inbox");
        status(
          changed.length
            ? `Undone. ${back} ${moved ? (changed.length === 1 ? "is back in the inbox" : "are back in the inbox") : "is as it was"}.`
            : "Undone.",
        );
        return { action: "Undo", thread: back };
      }
      case "search.open":
        openSearch();
        return null;
      case "compose.new":
        openCompose();
        return null;
      case "help.shortcuts":
        openSheet();
        return null;
      case "palette.open":
        openPal();
        return null;
      case "thread.moveToInbox": {
        status("Already in the inbox");
        return null;
      }
      default: {
        const v = VIEW_OF[id];
        if (v) {
          s.view = v;
          s.query = "";
          s.picked = [];
          s.cursor = visible(s)[0]?.id ?? null;
          render();
          status(`${VIEW_NAME[v]}`);
          return null;
        }
        const label = LABEL.get(id);
        if (label) status(`${label} works in the real app; this demo keeps to the basics`);
        return null;
      }
    }
  }

  function back(): boolean {
    if (!popEl.hidden) {
      closePop();
      return true;
    }
    if (!sheetEl.hidden) {
      closeSheetPane();
      return true;
    }
    if (replying) {
      stopReply();
      return true;
    }
    if (s.query) {
      s.query = "";
      render();
      status("Search cleared");
      return true;
    }
    if (s.picked.length) {
      s.picked = [];
      render();
      return true;
    }
    return false;
  }

  // ───────────── reading sheet (narrow layouts) ─────────────

  function openSheetPane() {
    sheetEl.hidden = false;
    sheetPane.innerHTML = paneHtml(s);
  }
  function closeSheetPane() {
    sheetEl.hidden = true;
  }
  $("[data-isheet-back]", root)?.addEventListener("click", () => {
    tally.clicks++;
    closeSheetPane();
    render();
  });

  // ───────────── summary stream ─────────────

  function stream() {
    streamCancel?.();
    const cur = current(s);
    if (!cur) return;
    const total = cur.summary.join(" ").split(" ").length;
    if (!motion) {
      s.sumWords = null;
      render();
      return;
    }
    s.sumWords = 0;
    render();
    let n = 0;
    let stop = false;
    streamCancel = () => (stop = true);
    const tick = () => {
      if (stop) return;
      n++;
      s.sumWords = n >= total ? null : n;
      paneEl.innerHTML = paneHtml(s);
      if (!sheetEl.hidden) sheetPane.innerHTML = paneHtml(s);
      if (n < total) after(18, tick);
    };
    after(18, tick);
  }

  // ───────────── reply ─────────────

  function startReply(name: string, suggest: boolean) {
    replying = true;
    replyEl.classList.add("is-active");
    replyText.textContent = suggest ? "Three suggested replies are in the real app" : `Reply to ${name}`;
    status(suggest ? "Suggested replies: shift R in the real app" : `Replying to ${name} · Esc to leave`);
  }
  function stopReply() {
    replying = false;
    replyEl.classList.remove("is-active");
    replyText.textContent = "Reply…";
  }
  replyEl.addEventListener("click", () => {
    tally.clicks++;
    const cur = current(s);
    if (cur) startReply(cur.from, false);
  });

  // ───────────── snooze popover ─────────────

  function openPop(kind: "snooze" | "remind") {
    const ids = targets();
    pop = { kind, hi: 1, ids };
    popHead.textContent = kind === "snooze" ? "Snooze until" : "Remind if no reply";
    const row = $(`#ir-${s.cursor}`, rowsEl);
    const top = row ? row.offsetTop + row.offsetHeight + 4 : 48;
    const maxTop = listEl.clientHeight - 214;
    popEl.style.top = `${Math.max(8, Math.min(top, maxTop))}px`;
    popEl.style.left = phone() ? "12px" : "180px";
    popEl.hidden = false;
    markPop();
  }
  function markPop() {
    $$(".ib-pop-row", popEl).forEach((r, i) => {
      r.classList.toggle("is-hi", i === pop?.hi);
    });
  }
  function closePop() {
    popEl.hidden = true;
    pop = null;
  }
  function choose(i: number): { action: string; thread: string } | null {
    if (!pop) return null;
    const o = SNOOZE_OPTIONS[i];
    if (!o) return null;
    if (o.key === "P") {
      closePop();
      status("Date picker is in the real app");
      return null;
    }
    const { kind, ids } = pop;
    closePop();
    const when = `${o.label}${o.when ? `, ${o.when}` : ""}`;
    const what = describe(ids);
    const thread =
      ids.length > 1
        ? what
        : (() => {
            const t = subjOf(ids[0] ?? null);
            return t ? `${t.from} — ${t.subject}` : "";
          })();
    if (kind === "snooze") {
      relocate(ids, "snoozed");
      closeSheetPane();
      render();
      status(`Snoozed ${what} until ${when} · Z to undo`);
      return { action: "Snooze", thread };
    }
    status(`Reminder set: ${what} comes back ${when} if nobody replies`);
    return { action: "Remind", thread };
  }
  for (const [i, b] of $$<HTMLButtonElement>(".ib-pop-row", popEl).entries()) {
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      tally.clicks++;
      const r = choose(i);
      if (r) log("↵", r.action, r.thread, performance.now());
      render();
    });
  }

  // ───────────── search ─────────────

  const OPS = ["from:", "has:attachment", "is:unread", "is:starred"];
  function openSearch() {
    searchBox.hidden = false;
    searchIn.value = s.query;
    searchIn.focus();
    suggest();
  }
  function closeSearch(clear: boolean) {
    searchBox.hidden = true;
    if (clear) s.query = "";
    render();
    rowsEl.focus({ preventScroll: true });
  }
  function suggest() {
    const tok = searchIn.value.split(/\s+/).pop() ?? "";
    const list = tok
      ? OPS.filter((o) => o.startsWith(tok.toLowerCase()) && o !== tok.toLowerCase())
      : OPS.slice(0, 3);
    searchOps.innerHTML = list
      .map((o) => `<button type="button" tabindex="-1" data-op="${o}">${o}</button>`)
      .join("");
  }
  searchIn.addEventListener("input", () => {
    s.query = searchIn.value;
    render();
    suggest();
  });
  searchIn.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeSearch(true);
      status("Search closed");
    } else if (e.key === "Enter") {
      e.preventDefault();
      closeSearch(false);
      status(s.query ? `${visible(s).length} results for ${s.query} · Esc clears` : "Search closed");
    } else if (e.key === "Tab") {
      e.preventDefault();
      const first = $<HTMLButtonElement>("[data-op]", searchOps);
      if (first) applyOp(first.dataset.op ?? "");
    }
  });
  const applyOp = (op: string) => {
    const parts = searchIn.value.split(/\s+/);
    parts[parts.length - 1] = op;
    searchIn.value = parts.join(" ") + (op.endsWith(":") ? "" : " ");
    s.query = searchIn.value;
    render();
    suggest();
    searchIn.focus();
  };
  searchOps.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-op]");
    if (b) applyOp(b.dataset.op ?? "");
  });

  // ───────────── compose ─────────────

  function openCompose() {
    if (phone()) {
      status("Compose with C on a keyboard");
      return;
    }
    composeEl.hidden = false;
    $<HTMLInputElement>("[data-compose-to]", composeEl)?.focus();
  }
  function closeCompose() {
    composeEl.hidden = true;
    rowsEl.focus({ preventScroll: true });
  }
  function send(later: boolean) {
    closeCompose();
    if (later) {
      status("Scheduled for tomorrow, 08:00. The server sends it even if your laptop is closed.");

      return;
    }
    status("Sending… Undo Z");

    const cancel = after(5000, () => {
      pendingSend = null;
      status("Sent “Thursday” to Lukas Brenner");
    });
    pendingSend = cancel;
  }
  composeEl.addEventListener("submit", (e) => {
    e.preventDefault();
    send(false);
  });
  $("[data-send-later]", composeEl)?.addEventListener("click", () => send(true));
  composeEl.addEventListener("keydown", (e) => {
    const c = comboOf(e);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeCompose();
      status("Draft kept");
    } else if (isMod(e) && e.key === "Enter") {
      e.preventDefault();
      send(false);
    } else if (isMod(e) && e.shiftKey && e.key.toLowerCase() === "l") {
      e.preventDefault();
      send(true);
    } else if (c === "tab" || c === "shift+tab") {
      // keep focus inside the composer
      const f = $$<HTMLElement>("input, textarea, button", composeEl);
      const i = f.indexOf(document.activeElement as HTMLElement);
      e.preventDefault();
      f[(i + (e.shiftKey ? f.length - 1 : 1)) % f.length]?.focus();
    }
  });

  // ───────────── in-mock palette ─────────────

  function palShown() {
    return palRows.filter((r) => !r.hidden);
  }
  function palMark() {
    const list = palShown();
    palSel = Math.max(0, Math.min(palSel, list.length - 1));
    for (const r of palRows) r.setAttribute("aria-selected", "false");
    const cur = list[palSel];
    cur?.setAttribute("aria-selected", "true");
    cur?.scrollIntoView({ block: "nearest" });
  }
  function openPal() {
    if (phone()) return;
    palEl.hidden = false;
    palIn.value = "";
    for (const r of palRows) r.hidden = false;
    const pe = $("[data-ipal-empty]", root);
    if (pe) pe.hidden = true;
    palSel = 0;
    palMark();
    palIn.focus({ preventScroll: true });
    const r = palEl.getBoundingClientRect();
    if (r.top < 64 || r.bottom > window.innerHeight)
      palEl.scrollIntoView({ block: "center", behavior: motion ? "smooth" : "auto" });
  }
  function closePal() {
    palEl.hidden = true;
    rowsEl.focus({ preventScroll: true });
  }
  const palEmpty = $("[data-ipal-empty]", root);
  palIn.addEventListener("input", () => {
    const q = palIn.value.toLowerCase().trim();
    for (const r of palRows) {
      const label = r.dataset.label ?? "";
      r.hidden =
        !!q && !label.split(/\s+/).some((w) => w.startsWith(q)) && !label.startsWith(q) && !label.includes(q);
    }
    if (palEmpty) palEmpty.hidden = palShown().length > 0;
    palSel = 0;
    palMark();
  });
  function palRun(r: HTMLButtonElement | undefined, started: number) {
    if (!r) return;
    closePal();
    const id = r.dataset.cmdId ?? "";
    const res = run(id);
    if (res) log("⌘K", res.action, res.thread, started);
  }
  palIn.addEventListener("keydown", (e) => {
    const c = comboOf(e);
    if (c === "escape") {
      e.preventDefault();
      e.stopPropagation();
      closePal();
    } else if (c === "down" || c === "ctrl+n") {
      e.preventDefault();
      palSel = (palSel + 1) % Math.max(1, palShown().length);
      palMark();
    } else if (c === "up" || c === "ctrl+p") {
      e.preventDefault();
      palSel = (palSel - 1 + palShown().length) % Math.max(1, palShown().length);
      palMark();
    } else if (c === "enter") {
      e.preventDefault();
      palRun(palShown()[palSel], performance.now());
    } else if (c === "tab" || c === "shift+tab") e.preventDefault();
  });
  for (const r of palRows) r.addEventListener("click", () => palRun(r, performance.now()));

  // ───────────── HUD ─────────────

  function hud(cap: string) {
    const k = document.createElement("kbd");
    k.className = "kbd";
    k.innerHTML = glyphHtml(cap);
    hudEl.append(k);
    while (hudEl.children.length > 5) hudEl.firstElementChild?.remove();
    after(1200, () => {
      if (!motion) return k.remove();
      k.classList.add("is-out");
      after(300, () => k.remove());
    });
  }

  // ───────────── key handling ─────────────

  function handle(e: KeyboardEvent | null, combo: string, cap: string, fromGhost = false): boolean {
    const started = performance.now();
    // popover keys
    if (pop) {
      let r: { action: string; thread: string } | null = null;
      if (combo === "escape") closePop();
      else if (combo === "down" || combo === "j") {
        pop.hi = (pop.hi + 1) % SNOOZE_OPTIONS.length;
        markPop();
      } else if (combo === "up" || combo === "k") {
        pop.hi = (pop.hi + SNOOZE_OPTIONS.length - 1) % SNOOZE_OPTIONS.length;
        markPop();
      } else if (combo === "enter") r = choose(pop.hi);
      else {
        const i = SNOOZE_OPTIONS.findIndex((o) => o.key.toLowerCase() === combo);
        if (i >= 0) r = choose(i);
        else return false;
      }
      e?.preventDefault();
      if (!fromGhost) count(cap);
      if (r && !fromGhost) log(cap, r.action, r.thread, started);
      return true;
    }
    let id: string | undefined;
    if (gPending) {
      gPending = false;
      clearG?.();
      id = map.get(`g ${combo}`);
      if (!id) return false;
    } else if (combo === "g") {
      gPending = true;
      clearG = after(1200, () => (gPending = false));
      e?.preventDefault();
      if (!fromGhost) count(cap);
      return true;
    } else if (combo === "escape" && back()) {
      e?.preventDefault();
      if (!fromGhost) count(cap);
      return true;
    } else id = map.get(combo);
    if (!id) return false;
    if (id === "palette.open" && !e) return false;
    e?.preventDefault();
    const res = run(id);
    if (!fromGhost) {
      count(cap);
      if (res) log(cap, res.action, res.thread, started);
    }
    return true;
  }

  function count(cap: string) {
    tally.keys++;
    hud(cap);
    setChip();
    const tl = $(".iz-tally", emptyEl);
    if (tl)
      tl.textContent = `You pressed ${tally.keys} ${tally.keys === 1 ? "key" : "keys"} and clicked ${tally.clicks} ${tally.clicks === 1 ? "time" : "times"}.`;
  }

  // ───────────── modes: live, focused ─────────────

  // Letter keys, and ⌘K, reach the mock while enough of it is on screen (the "Live" chip says so).
  // The chip only appears once the visitor has actually used a key or the pointer here.
  function setChip() {
    const focused = page.inboxFocused;
    const live = !focused && page.inboxLive;
    const used = tally.keys + tally.clicks > 0;
    chipEl.hidden = !used || (!focused && !live);
    chipEl.classList.toggle("is-live", live);
    chipEl.textContent = focused ? "Esc to leave" : "Live · keys go to this inbox";
    stage.classList.toggle("is-focused", focused);
    if (tallyEl)
      tallyEl.textContent = `${tally.keys} ${tally.keys === 1 ? "key" : "keys"} · ${tally.clicks} ${tally.clicks === 1 ? "click" : "clicks"}`;
  }
  function leaveFocus() {
    if (frame.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
    page.inboxFocused = false;
    setChip();
  }
  function updateLive() {
    const live = ratio >= 0.5 || (topVisible && visibleH >= 160);
    page.inboxLive = live && !phone();
    if (page.inboxFocused && ratio < 0.25) leaveFocus();
    setChip();
  }
  watch(stage, [0, 0.1, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.75, 1], (en) => {
    ratio = en.intersectionRatio;
    visibleH = en.intersectionRect.height;
    topVisible =
      en.isIntersecting && en.boundingClientRect.top >= 0 && en.boundingClientRect.top < window.innerHeight;
    if (ratio >= 0.6) maybeGhost();
    updateLive();
  });
  frame.addEventListener("focusin", (e) => {
    const fromOutside = !frame.contains(e.relatedTarget as Node | null);
    page.inboxFocused = true;
    setChip();
    if (fromOutside)
      liveEl.textContent =
        "Demo inbox. J and K move, E archives, Z undoes. Once you have pressed a key here, Tab switches splits. Esc leaves.";
  });
  frame.addEventListener("focusout", (e) => {
    if (!frame.contains(e.relatedTarget as Node | null)) {
      page.inboxFocused = false;
      setChip();
    }
  });

  addLayer(3, (e) => {
    if (!page.inboxFocused || isEditable(e.target)) return false;
    const combo = comboOf(e);
    if (isMod(e) && e.key.toLowerCase() === "k") {
      stopGhost();
      e.preventDefault();
      count(capOf(e));
      openPal();
      return true;
    }
    // Tab belongs to the page until the visitor has used the mock, and always lets them out past
    // the last split (or before the first with Shift Tab).
    if (combo === "tab" || combo === "shift+tab") {
      if (!tally.keys || pop) return false;
      const i = SPLITS.indexOf(s.split);
      if (combo === "tab" && i === SPLITS.length - 1) return false;
      if (combo === "shift+tab" && i === 0) return false;
    }
    stopGhost();
    if (combo === "escape" && popEl.hidden && sheetEl.hidden && !replying && !s.query && !s.picked.length) {
      e.preventDefault();
      leaveFocus();
      status("Left the inbox. Click it, or Tab to it, to come back.");
      return true;
    }
    return handle(e, combo, capOf(e));
  });
  addLayer(4, (e) => {
    if (!page.inboxLive || page.inboxFocused) return false;
    // Ctrl K / ⌘K belongs to the site until the visitor has actually used the inbox.
    if (isMod(e) && e.key.toLowerCase() === "k" && tally.keys > 0) {
      stopGhost();
      e.preventDefault();
      count(capOf(e));
      openPal();
      return true;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return false;
    const combo = comboOf(e);
    const plain = /^(shift\+)?[a-z]$/.test(combo) || combo === "#" || combo === "/";
    const popKeys = !!pop && (/^[1-4]$/.test(combo) || combo === "enter" || combo === "escape");
    if (!plain && !popKeys) return false;
    if (!pop && !gPending && combo !== "g" && !map.has(combo)) return false;
    stopGhost();
    return handle(e, combo, capOf(e));
  });

  // ───────────── pointer ─────────────

  frame.addEventListener("click", (e) => {
    const t = e.target as HTMLElement;
    if (t.closest("input, textarea, form")) return;
    stopGhost();
    tally.clicks++;
    const act = t.closest<HTMLButtonElement>("[data-act]");
    const row = t.closest<HTMLElement>(".ir");
    const started = performance.now();
    if (row?.dataset.id) {
      s.cursor = row.dataset.id;
      s.sumWords = null;
    }
    if (act) {
      const id =
        { archive: "thread.archive", snooze: "thread.snooze", star: "thread.star" }[act.dataset.act ?? ""] ??
        "";
      const r = run(id);
      if (r) log("click", r.action, r.thread, started);
    } else if (row) {
      if (phone()) {
        const r = run("nav.open");
        if (r) log("tap", r.action, r.thread, started);
      } else render();
    }
    const tab = t.closest<HTMLElement>("[data-split]");
    if (tab) {
      s.split = tab.dataset.split as InboxState["split"];
      s.view = "inbox";
      s.picked = [];
      s.cursor = visible(s)[0]?.id ?? null;
      render();
    }
    const view = t.closest<HTMLElement>("[data-view]");
    if (view) run(`go.${view.dataset.view}`);
    if (t.closest("[data-ib-search]")) openSearch();
    if (t.closest("[data-ib-compose]")) openCompose();
    if (!t.closest("input, textarea, [data-ipal], [data-compose], [data-searchbox]"))
      rowsEl.focus({ preventScroll: true });
    setChip();
  });

  // swipe (phones)
  let sw: null | {
    row: HTMLElement;
    x: number;
    y: number;
    dx: number;
    bg: HTMLElement | null;
    locked: boolean | null;
    id: number;
  } = null;
  rowsEl.addEventListener("pointerdown", (e) => {
    if (!phone() || e.pointerType === "mouse") return;
    const row = (e.target as HTMLElement).closest<HTMLElement>(".ir");
    if (!row) return;
    stopGhost();
    sw = { row, x: e.clientX, y: e.clientY, dx: 0, bg: null, locked: null, id: e.pointerId };
  });
  rowsEl.addEventListener("pointermove", (e) => {
    if (!sw || e.pointerId !== sw.id) return;
    const dx = e.clientX - sw.x;
    const dy = e.clientY - sw.y;
    if (sw.locked === null && Math.abs(dx) + Math.abs(dy) > 8) sw.locked = Math.abs(dx) > Math.abs(dy);
    if (!sw.locked) return;
    sw.dx = dx;
    if (!sw.bg) {
      sw.bg = document.createElement("div");
      sw.bg.className = "ir-swipe-bg";
      sw.bg.style.top = `${sw.row.offsetTop}px`;
      sw.bg.style.height = `${sw.row.offsetHeight}px`;
      sw.bg.style.bottom = "auto";
      listEl.append(sw.bg);
      sw.row.classList.add("is-swiping");
      sw.row.style.background = "var(--bg)";
    }
    const left = dx < 0;
    sw.bg.className = `ir-swipe-bg ${left ? "is-archive" : "is-snooze"}`;
    sw.bg.textContent = left ? "Archive" : "Snooze · tomorrow 08:00";
    sw.row.style.transform = `translateX(${dx}px)`;
  });
  const endSwipe = () => {
    if (!sw) return;
    const { row, dx, bg } = sw;
    sw = null;
    const w = row.offsetWidth;
    if (Math.abs(dx) > w * 0.35) {
      tally.clicks++;
      s.cursor = row.dataset.id ?? s.cursor;
      bg?.remove();
      const started = performance.now();
      let r: { action: string; thread: string } | null;
      if (dx < 0) r = run("thread.archive");
      else {
        pop = { kind: "snooze", hi: 1, ids: [s.cursor ?? ""] };
        r = choose(1);
      }
      if (r) log(dx < 0 ? "swipe ←" : "swipe →", r.action, r.thread, started);
      return;
    }
    row.style.transition = "transform 200ms var(--ease-out)";
    row.style.transform = "";
    after(220, () => {
      bg?.remove();
      row.style.transition = "";
      row.style.background = "";
      row.classList.remove("is-swiping");
    });
  };
  rowsEl.addEventListener("pointerup", endSwipe);
  rowsEl.addEventListener("pointercancel", endSwipe);

  // ───────────── idle ghost ─────────────

  function maybeGhost() {
    if (ghost.done || ghost.running || !motion || document.hidden || tally.keys || tally.clicks) return;
    ghost.running = true;
    ghost.startY = window.scrollY;
    const seq: [string, string][] = [
      ["j", "J"],
      ["j", "J"],
      ["e", "E"],
      ["k", "K"],
      ["s", "S"],
    ];
    let i = 0;
    const step = () => {
      if (!ghost.running) return;
      const k = seq[i++];
      if (!k) {
        ghost.running = false;
        ghost.done = true;
        ghost.cancel = [];
        return;
      }
      ghost.stepped = true;
      hud(k[1]);
      handle(null, k[0], k[1], true);
      ghost.cancel.push(after(650, step));
    };
    ghost.cancel.push(after(4000, step));
  }
  function stopGhost() {
    for (const c of ghost.cancel) c();
    ghost.cancel = [];
    const touched = ghost.stepped;
    ghost.running = false;
    ghost.done = true;
    ghost.stepped = false;
    if (touched) {
      s = initialState();
      history = [];
      lastEmpty = "";
      render();
      statusEl.innerHTML = idleStatus;
      statusEl.classList.remove("is-echo");
    }
  }
  // Phones: the touch bar acts on the selected row, like the keys it shows.
  for (const b of $$<HTMLButtonElement>("[data-touch]", root)) {
    b.addEventListener("click", () => {
      stopGhost();
      tally.clicks++;
      const k = b.dataset.touch ?? "";
      const started = performance.now();
      const id = map.get(k);
      const res = id ? run(id) : null;
      if (res) log(k.toUpperCase(), res.action, res.thread, started);
      setChip();
    });
  }
  frame.addEventListener("pointerenter", stopGhost);
  frame.addEventListener("touchstart", stopGhost, { passive: true });
  window.addEventListener(
    "scroll",
    () => {
      if (ghost.running && Math.abs(window.scrollY - ghost.startY) > 40) stopGhost();
    },
    { passive: true },
  );

  render();
  setChip();
}
