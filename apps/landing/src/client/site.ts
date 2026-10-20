// Site chrome: theme, header, menu sheet, palette, shortcuts sheet, global keys, reveals,
// waitlist form and copy buttons.
import { THEME_KEY } from "@/lib/boot";
import { WL_LINE, WL_SUBJECT } from "@/lib/site";
import { addLayer } from "./router";
import { page } from "./state";
import { $, $$, after, comboOf, isMod, reducedMotion, scrollToEl } from "./util";

// ───────────── theme ─────────────

type Theme = "system" | "light" | "dark";
function currentTheme(): Theme {
  const t = document.documentElement.dataset.theme;
  return t === "light" || t === "dark" ? t : "system";
}
export function setTheme(t: Theme) {
  const root = document.documentElement;
  if (t === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", t);
  try {
    if (t === "system") localStorage.removeItem(THEME_KEY);
    else localStorage.setItem(THEME_KEY, t);
  } catch {
    // Storage can be unavailable (private mode); the choice still applies to this page view.
  }
  syncTheme();
}
const THEME_COLOR = { light: "#FFFFFF", dark: "#0A0A0A" } as const;
function syncThemeColor(t: Theme) {
  for (const m of $$<HTMLMetaElement>('meta[name="theme-color"]')) {
    if (!m.dataset.media) m.dataset.media = m.media || "";
    if (!m.dataset.color) m.dataset.color = m.content;
    if (t === "system") {
      m.media = m.dataset.media;
      m.content = m.dataset.color;
    } else {
      m.media = "";
      m.content = THEME_COLOR[t];
    }
  }
}
function syncTheme() {
  const t = currentTheme();
  syncThemeColor(t);
  for (const b of $$<HTMLButtonElement>("[data-theme-set]"))
    b.setAttribute("aria-pressed", String(b.dataset.themeSet === t));
}
function initTheme() {
  syncTheme();
  for (const b of $$<HTMLButtonElement>("[data-theme-set]"))
    b.addEventListener("click", () => setTheme(b.dataset.themeSet as Theme));
}

// ───────────── header ─────────────

function initHeader() {
  const hdr = $("[data-header]");
  if (!hdr) return;
  let on = false;
  const update = () => {
    const next = window.scrollY > 24;
    if (next !== on) {
      on = next;
      hdr.classList.toggle("is-scrolled", on);
    }
  };
  update();
  window.addEventListener("scroll", update, { passive: true });
}

// ───────────── modal helpers ─────────────

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), textarea, [tabindex]:not([tabindex="-1"])';
let openModal: { el: HTMLElement; close: () => void } | null = null;

function trap(el: HTMLElement, e: KeyboardEvent) {
  if (e.key !== "Tab") return;
  const items = $$<HTMLElement>(FOCUSABLE, el).filter(
    (x) => x.tabIndex >= 0 && (x.offsetParent !== null || x === document.activeElement),
  );
  if (!items.length) return;
  const first = items[0]!;
  const last = items[items.length - 1]!;
  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault();
    first.focus();
  }
}

function modal(el: HTMLElement, opts: { focus: () => void; onClose?: () => void; backdrop?: boolean }) {
  const backdrop = $("[data-backdrop]");
  let returnTo: HTMLElement | null = null;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else trap(el, e);
  };
  const open = () => {
    if (openModal && openModal.el !== el) openModal.close();
    returnTo = document.activeElement as HTMLElement | null;
    el.hidden = false;
    el.classList.add("is-open");
    if (opts.backdrop !== false && backdrop) backdrop.hidden = false;
    document.addEventListener("keydown", onKey, true);
    openModal = { el, close };
    page.overlay = true;
    document.documentElement.classList.add("is-locked");
    opts.focus();
  };
  const close = () => {
    if (el.hidden) return;
    if (el.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
    el.hidden = true;
    el.classList.remove("is-open");
    if (backdrop) backdrop.hidden = true;
    document.removeEventListener("keydown", onKey, true);
    if (openModal?.el === el) openModal = null;
    page.overlay = false;
    document.documentElement.classList.remove("is-locked");
    opts.onClose?.();
    returnTo?.focus?.({ preventScroll: true });
  };
  return { open, close, isOpen: () => !el.hidden };
}

// ───────────── palette ─────────────

let palette: ReturnType<typeof modal> | null = null;
export const openPalette = () => palette?.open();

function initPalette() {
  const el = $("[data-palette]");
  const input = $<HTMLInputElement>("[data-pal-input]");
  if (!el || !input) return;
  const items = $$<HTMLButtonElement>("[data-pal-item]", el);
  const groups = $$("[data-pal-group]", el);
  const empty = $("[data-pal-empty]", el);
  let sel = 0;
  const shown = () => items.filter((i) => !i.hidden);
  const mark = () => {
    const list = shown();
    sel = Math.max(0, Math.min(sel, list.length - 1));
    for (const i of items) i.setAttribute("aria-selected", "false");
    const cur = list[sel];
    if (cur) {
      cur.setAttribute("aria-selected", "true");
      input.setAttribute("aria-activedescendant", cur.id);
      cur.scrollIntoView({ block: "nearest" });
    } else input.removeAttribute("aria-activedescendant");
  };
  const filter = () => {
    const words = input.value.toLowerCase().trim().split(/\s+/).filter(Boolean);
    for (const i of items) i.hidden = !words.every((w) => (i.dataset.search ?? "").includes(w));
    for (const g of groups) g.hidden = !$$("[data-pal-item]", g).some((i) => !i.hidden);
    if (empty) empty.hidden = shown().length > 0;
    sel = 0;
    mark();
  };
  palette = modal(el, {
    focus: () => {
      input.value = "";
      filter();
      input.focus();
    },
  });
  input.addEventListener("input", filter);
  input.addEventListener("keydown", (e) => {
    const c = comboOf(e);
    if (c === "down" || c === "ctrl+n") {
      e.preventDefault();
      sel = (sel + 1) % Math.max(1, shown().length);
      mark();
    } else if (c === "up" || c === "ctrl+p") {
      e.preventDefault();
      sel = (sel - 1 + shown().length) % Math.max(1, shown().length);
      mark();
    } else if (c === "enter") {
      e.preventDefault();
      shown()[sel]?.click();
    } else if (c === "tab" || c === "shift+tab") {
      e.preventDefault();
    }
  });
  for (const i of items) {
    i.addEventListener("click", () => run(i));
    i.addEventListener("mousemove", () => {
      const idx = shown().indexOf(i);
      if (idx !== sel) {
        sel = idx;
        mark();
      }
    });
  }
  $("[data-backdrop]")?.addEventListener("click", () => {
    palette?.close();
    sheet?.close();
  });
  const run = (i: HTMLButtonElement) => {
    const href = i.dataset.href;
    const cmd = i.dataset.cmd ?? "";
    palette?.close();
    if (href) {
      location.href = href;
      return;
    }
    if (cmd === "waitlist") goWaitlist();
    else if (cmd === "inbox") tryInbox();
    else if (cmd === "shortcuts") openSheet();
    else if (cmd === "copy") copy(i.querySelector(".pal-sub")?.textContent ?? "", null);
    else if (cmd.startsWith("theme:")) setTheme(cmd.slice(6) as Theme);
  };
  for (const b of $$("[data-open-palette]")) b.addEventListener("click", () => palette?.open());
}

// ───────────── shortcuts sheet ─────────────

let sheet: ReturnType<typeof modal> | null = null;
export const openSheet = () => sheet?.open();

function initSheet() {
  const el = $("[data-shortcuts]");
  if (!el) return;
  sheet = modal(el, {
    focus: () => {
      const tpl = $<HTMLTemplateElement>("template[data-sheet-tpl]", el);
      if (tpl) tpl.replaceWith(tpl.content.cloneNode(true));
      // The list scrolls inside the sheet: focus it, so arrows and Page Down move the list, not the page.
      $<HTMLElement>("[data-sheet-body]", el)?.focus();
    },
  });
  $("[data-close-sheet]", el)?.addEventListener("click", () => sheet?.close());
  for (const b of $$("[data-open-shortcuts]")) b.addEventListener("click", () => sheet?.open());
}

// ───────────── mobile menu ─────────────

function initMenu() {
  const el = $("#msheet");
  const btn = $("[data-open-menu]");
  if (!el || !btn) return;
  const m = modal(el, {
    backdrop: false,
    focus: () => {
      btn.setAttribute("aria-expanded", "true");
      $<HTMLElement>("[data-close-menu]", el)?.focus();
    },
    onClose: () => btn.setAttribute("aria-expanded", "false"),
  });
  btn.addEventListener("click", () => m.open());
  for (const c of $$("[data-close-menu]", el)) c.addEventListener("click", () => m.close());
}

// ───────────── waitlist, inbox links ─────────────

export function goWaitlist() {
  const target = $("#waitlist");
  const input = $<HTMLInputElement>("[data-wl-input]");
  if (!target || !input) {
    location.href = "/#waitlist";
    return;
  }
  scrollToEl(target, "center");
  after(reducedMotion() ? 0 : 700, () => input.focus({ preventScroll: true }));
}

export function tryInbox() {
  const stage = $("[data-stage]");
  const list = $<HTMLElement>("[data-rows]");
  if (!stage || !list) {
    location.href = "/#inbox";
    return;
  }
  scrollToEl(stage, "center");
  list.focus({ preventScroll: true });
}

function initLinks() {
  for (const a of $$<HTMLAnchorElement>("[data-waitlist]")) {
    a.addEventListener("click", (e) => {
      if (!$("#waitlist")) return;
      e.preventDefault();
      goWaitlist();
    });
  }
  // The inbox-zero link is rendered later; delegate.
  document.addEventListener("click", (e) => {
    const a = (e.target as HTMLElement).closest?.(".iz-link");
    if (a) {
      e.preventDefault();
      goWaitlist();
    }
  });
  for (const a of $$<HTMLAnchorElement>("[data-try-inbox]")) {
    a.addEventListener("click", (e) => {
      e.preventDefault();
      tryInbox();
    });
  }
  if (location.hash === "#waitlist")
    after(300, () => $<HTMLInputElement>("[data-wl-input]")?.focus({ preventScroll: true }));
}

function copy(text: string, btn: HTMLElement | null) {
  const done = () => {
    if (!btn) return;
    const old = btn.textContent;
    btn.textContent = "Copied";
    after(1500, () => (btn.textContent = old));
  };
  navigator.clipboard?.writeText(text).then(done, done);
}

function initWaitlist() {
  const wrap = $("[data-wl]");
  const form = $<HTMLFormElement>("[data-wl-form]");
  const input = $<HTMLInputElement>("[data-wl-input]");
  const err = $("[data-wl-err]");
  const doneEl = $("[data-wl-done]");
  if (!wrap || !form || !input || !err || !doneEl) return;
  form.noValidate = true;
  const valid = (v: string) => /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(v);
  const setInvalid = (on: boolean) => {
    wrap.classList.toggle("is-invalid", on);
    err.hidden = !on;
    if (on) input.setAttribute("aria-invalid", "true");
    else input.removeAttribute("aria-invalid");
  };
  const echo = $("[data-wl-echo]");
  const placeholder = echo?.textContent ?? "";
  input.addEventListener("input", () => {
    if (echo) echo.textContent = input.value.trim() || placeholder;
    echo?.classList.toggle("is-typed", !!input.value.trim());
    if (wrap.classList.contains("is-invalid") && valid(input.value.trim())) setInvalid(false);
  });
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const email = input.value.trim();
    if (!valid(email)) {
      setInvalid(true);
      input.focus();
      return;
    }
    setInvalid(false);
    const body = [WL_LINE, "", `Email: ${email}`].join("\n");
    const href = `mailto:${form.action.replace(/^mailto:/, "").split("?")[0]}?subject=${encodeURIComponent(WL_SUBJECT)}&body=${encodeURIComponent(body)}`;
    window.location.href = href;
    form.hidden = true;
    doneEl.hidden = false;
  });
  for (const b of $$<HTMLButtonElement>("[data-copy]"))
    b.addEventListener("click", () => copy(b.dataset.copy ?? "", b));
}

// ───────────── reveals ─────────────

function initReveal() {
  const blocks = $$("[data-reveal], [data-sunrise]");
  if (!blocks.length) return;
  const io = new IntersectionObserver(
    (entries) => {
      for (const en of entries) {
        const need = (en.target as HTMLElement).dataset.sunrise !== undefined ? 0.3 : 0.15;
        if (
          en.intersectionRatio >= need ||
          (en.isIntersecting && en.intersectionRect.height > window.innerHeight * 0.5)
        ) {
          en.target.classList.add("is-in");
          io.unobserve(en.target);
        }
      }
    },
    { threshold: [0, 0.15, 0.3] },
  );
  for (const b of blocks) io.observe(b);
  // Arm the hidden state only once the visitor scrolls, and only below the fold, so nothing a
  // full-page capture or a crawler sees is ever at opacity 0.
  const arm = () => {
    window.removeEventListener("scroll", arm);
    for (const b of blocks) {
      const r = b.getBoundingClientRect();
      if (r.top < window.innerHeight && r.bottom > 0) b.classList.add("is-in");
    }
    document.documentElement.classList.add("armed");
    page.armed = true;
    for (const fn of page.onArm) fn();
  };
  window.addEventListener("scroll", arm, { passive: true });
}

// ───────────── global keys (priority 6) ─────────────

function initGlobalKeys() {
  let g = false;
  let clearG: (() => void) | null = null;
  addLayer(2, () => {
    // An open sheet or palette owns the keyboard (its own capture listener handles Esc and Tab).
    return page.overlay;
  });
  addLayer(6, (e) => {
    const c = comboOf(e);
    if (isMod(e) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      palette?.open();
      return true;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    if (g) {
      g = false;
      clearG?.();
      if (c === "h") {
        e.preventDefault();
        location.href = "/";
        return true;
      }
    }
    if (c === "g") {
      g = true;
      clearG = after(1200, () => (g = false));
      return true;
    }
    if (c === "w") {
      e.preventDefault();
      goWaitlist();
      return true;
    }
    if (c === "?") {
      e.preventDefault();
      sheet?.open();
      return true;
    }
    return false;
  });
}

export function initSite() {
  initTheme();
  initHeader();
  initPalette();
  initSheet();
  initMenu();
  initLinks();
  initWaitlist();
  initReveal();
  initGlobalKeys();
}
