// The mail client shell: sidebar, list (with split tabs and search) and reading pane, driven from the keyboard.
// Every action is a shortcut from packages/shortcuts and a command palette entry (BRIEF quality bar).
import { SPLIT_OTHER, SYS, type ThreadActions } from "@akatsuki/mail-model";
import {
  CommandPalette,
  Eyebrow,
  errorMessage,
  IconButton,
  type PaletteItem,
  paletteItemsFromShortcuts,
  useToast,
  useUiConfig,
} from "@akatsuki/ui";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type CommandContext, extensions, scopeOverride, useExtensionsVersion } from "../app/extensions.ts";
import { navigate, useLocation } from "../app/router.tsx";
import {
  formatRoute,
  labelKeyOfView,
  type MailboxView,
  type Route,
  sameView,
  VIEW_TITLES,
  viewKey,
} from "../app/routes.ts";
import { useBackend, useBodies, useSyncOrigin } from "../backend/context.tsx";
import type {
  AccountId,
  LabelView,
  ListViewArgs,
  MailAccountView,
  MeView,
  SplitView,
  ThreadDetail,
  ThreadId,
  ThreadRow,
} from "../backend/types.ts";
import { ErrorBoundary, ErrorState } from "../components/ErrorState.tsx";
import {
  IconArchive,
  IconBack,
  IconBell,
  IconClock,
  IconCommand,
  IconMail,
  IconMailOpen,
  IconMenu,
  IconPlus,
  IconStar,
  IconTag,
  IconTrash,
} from "../components/Icons.tsx";
import { useShortcuts } from "../keyboard/useShortcuts.ts";
import { fullDate, minuteNow } from "../lib/format.ts";
import { deviceId, deviceName, uuidv7 } from "../lib/ids.ts";
import {
  type AppearancePatch,
  nextTheme,
  setAppearance,
  type ThemePref,
  useDensity,
  useSyncServerTheme,
  useTheme,
} from "../lib/theme.ts";
import { Banners } from "./Banners.tsx";
import { EmptyView, NoResults, ReadingEmpty } from "./EmptyStates.tsx";
import { HelpDialog } from "./HelpDialog.tsx";
import { ListPicker, type PickerItem } from "./ListPicker.tsx";
import { openOriginalTab, originalPage } from "./original.ts";
import { SearchBar } from "./SearchBar.tsx";
import { countKeys, Sidebar, visibleLabels } from "./Sidebar.tsx";
import { ThreadList } from "./ThreadList.tsx";
import { ThreadView } from "./ThreadView.tsx";
import { TimePicker } from "./TimePicker.tsx";
import { type BuildAction, useMailActions } from "./useMailActions.ts";

type MailRoute = Extract<Route, { name: "mail" | "search" }>;

const NO_ACCOUNTS: MailAccountView[] = [];
const NO_LABELS: LabelView[] = [];
const NO_SPLITS: SplitView[] = [];
const NO_IDS: ThreadId[] = [];
const RECENT_KEY = "ak:palette-recent";
const APP_VERSION = "0.0.0";

function readRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string").slice(0, 5) : [];
  } catch {
    return [];
  }
}

type Picker = null | { kind: "label" | "move" | "goLabel" } | { kind: "snooze" | "remind" };

export interface MailAppProps {
  route: MailRoute;
  me: MeView;
}

export function MailApp({ route, me }: MailAppProps) {
  const backend = useBackend();
  const bodies = useBodies();
  const m = backend.useMutations();
  useExtensionsVersion();
  const { toast } = useToast();
  const ui = useUiConfig();
  const location = useLocation();
  const accounts = backend.useAccounts() ?? NO_ACCOUNTS;
  const labels = backend.useLabels() ?? NO_LABELS;
  const splitsRaw = backend.useSplits();
  const splits = useMemo(() => [...(splitsRaw ?? NO_SPLITS)].sort((a, b) => a.order - b.order), [splitsRaw]);
  const prefs = me.profile.prefs;
  const tz = me.profile.timezone;
  const syncOrigin = useSyncOrigin();
  const theme = useTheme();
  const density = useDensity();
  const savePrefs = useCallback((patch: AppearancePatch) => m.updatePrefs({ patch }), [m]);
  useSyncServerTheme(prefs.theme, prefs.density, savePrefs);
  useEffect(() => bodies.cache.setPersist(prefs.cacheBodiesOnDevice), [bodies, prefs.cacheBodiesOnDevice]);

  // Register this browser once per session (devices.register, §4.2).
  useEffect(() => {
    void m
      .registerDevice({ deviceId: deviceId(), kind: "web", name: deviceName(), appVersion: APP_VERSION })
      .catch(() => undefined);
  }, [m]);

  const isSearch = route.name === "search";
  const routeQ = route.name === "search" ? route.q : "";
  const view: MailboxView = route.name === "mail" ? route.view : { kind: "inbox" };
  const accountId = route.accountId as AccountId | undefined;
  const openId = (route.threadId ?? null) as ThreadId | null;
  const vkey = isSearch ? `search:${route.q}` : viewKey(view, accountId);

  // ───────────── data ─────────────
  const labelKey = route.name === "mail" ? labelKeyOfView(route.view) : null;
  const listArgs: ListViewArgs | null = useMemo(
    () => (labelKey ? { labelKey, ...(accountId && { accountId }) } : null),
    [labelKey, accountId],
  );
  const list = backend.useThreadList(listArgs);
  const isReminders = route.name === "mail" && route.view.kind === "reminders";
  const reminderTimers = backend.useTimers("reminder");
  const reminderIds = useMemo(
    () =>
      isReminders
        ? [...(reminderTimers ?? [])].sort((a, b) => a.fireAt - b.fireAt).map((t) => t.threadId as ThreadId)
        : NO_IDS,
    [isReminders, reminderTimers],
  );
  const reminderRows = backend.useThreadRows(reminderIds);
  const searchArgs = useMemo(
    () => (routeQ.trim() ? { q: routeQ.trim(), nowAt: minuteNow(), ...(accountId && { accountId }) } : null),
    [routeQ, accountId],
  );
  const search = backend.useSearch(searchArgs);
  const [fullSearch, setFullSearch] = useState<{ q: string; ids: ThreadId[]; busy: boolean } | null>(null);
  const fullIds = fullSearch && searchArgs && fullSearch.q === searchArgs.q ? fullSearch.ids : NO_IDS;
  const fullRows = backend.useThreadRows(fullIds);

  const rows: ThreadRow[] = useMemo(() => {
    if (isSearch) {
      const seen = new Set(search.rows.map((r) => r._id));
      return [...search.rows, ...(fullRows ?? []).filter((r) => !seen.has(r._id))];
    }
    if (isReminders) return reminderRows ?? [];
    return list.rows;
  }, [isSearch, isReminders, search.rows, fullRows, reminderRows, list.rows]);
  const status = isSearch ? search.status : isReminders ? (reminderRows ? "done" : "loading") : list.status;
  const loadMore = isSearch ? search.loadMore : list.loadMore;

  const keys = useMemo(() => countKeys(splits, labels), [splits, labels]);
  const counts = backend.useUnreadCounts(keys, accountId);
  const detail = backend.useThread(openId);
  const myEmails = useMemo(() => new Set(accounts.map((a) => a.email.toLowerCase())), [accounts]);
  const labelsByKey = useMemo(() => new Map(labels.map((l) => [l.labelKey, l])), [labels]);
  const actions = useMailActions({ accounts, labels });

  // ───────────── cursor, selection, scopes ─────────────
  const cursors = useRef(new Map<string, string>());
  const lastIndex = useRef(0);
  const [cursorState, setCursorState] = useState<string | null>(null);
  const [selection, setSelection] = useState<Set<string>>(new Set());
  const storedCursor = cursors.current.get(vkey) ?? cursorState;
  const cursorIndex = storedCursor ? rows.findIndex((r) => r._id === storedCursor) : -1;
  const cursorId =
    cursorIndex >= 0
      ? storedCursor
      : (rows[Math.min(lastIndex.current, Math.max(0, rows.length - 1))]?._id ?? null);
  if (cursorIndex >= 0) lastIndex.current = cursorIndex;
  const setCursor = useCallback(
    (id: string | null) => {
      if (id) cursors.current.set(vkey, id);
      else cursors.current.delete(vkey);
      setCursorState(id);
    },
    [vkey],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: clear the selection when the view changes
  useEffect(() => setSelection(new Set()), [vkey]);
  useEffect(() => {
    if (openId) setCursor(openId);
  }, [openId, setCursor]);

  const [paletteOpen, setPaletteOpen] = useState(false);
  // Narrow screens: the sidebar is a drawer behind the menu button.
  const [navOpen, setNavOpen] = useState(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: any navigation closes the drawer
  useEffect(() => setNavOpen(false), [vkey, openId]);
  const [helpOpen, setHelpOpen] = useState(false);
  const [picker, setPicker] = useState<Picker>(null);
  const [searchFocused, setSearchFocused] = useState(false);
  const [query, setQuery] = useState(routeQ);
  const [contactQ, setContactQ] = useState<string | null>(null);
  const contacts = backend.useContacts(contactQ);
  const [showImagesTick, setShowImagesTick] = useState(0);
  const [expandAllTick, setExpandAllTick] = useState(0);
  const [recent, setRecent] = useState(readRecent);
  const searchInput = useRef<HTMLInputElement>(null);
  const listEl = useRef<HTMLDivElement>(null);
  const listFocused = () => document.activeElement === listEl.current;
  const focusList = useCallback(() => listEl.current?.focus({ preventScroll: true }), []);
  // The list has the keyboard from the start (j/k, Enter, and its aria-activedescendant for screen readers).
  useEffect(() => {
    const t = setTimeout(() => {
      if (document.activeElement === document.body || document.activeElement === null) focusList();
    }, 0);
    return () => clearTimeout(t);
  }, [focusList]);
  // The field shows the query of the list on screen: the search's own on /search, empty anywhere else, so an
  // unfiltered list never sits under an old query. The last query stays one keystroke away as a suggestion.
  const [lastSearch, setLastSearch] = useState(routeQ);
  const shownList = JSON.stringify([
    route.name,
    route.name === "mail" ? route.view : null,
    routeQ,
    accountId ?? null,
  ]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset only when the list on screen changes
  useEffect(() => {
    setQuery(routeQ);
    if (routeQ) setLastSearch(routeQ);
  }, [shownList]);

  // ───────────── navigation ─────────────
  const routeTo = useCallback((r: Route) => navigate(formatRoute(r)), []);
  const go = useCallback(
    (v: MailboxView) => routeTo({ name: "mail", view: v, ...(accountId && { accountId }) }),
    [routeTo, accountId],
  );
  const openThread = useCallback(
    (id: string) => {
      setCursor(id);
      routeTo({ ...route, threadId: id });
    },
    [route, routeTo, setCursor],
  );
  const closeThread = useCallback(() => {
    const { threadId: _t, ...rest } = route;
    routeTo(rest as Route);
    queueMicrotask(() => listEl.current?.focus());
  }, [route, routeTo]);

  // Mark a thread read when it opens (once per open).
  const markedRead = useRef<string | null>(null);
  useEffect(() => {
    if (!detail || markedRead.current === detail.thread._id) return;
    markedRead.current = detail.thread._id;
    if (detail.thread.labelKeys.includes(SYS.unread))
      actions.perform([detail.thread], (a) => a.markRead(), null);
  }, [detail, actions]);

  // ───────────── targets and actions ─────────────
  const rowById = useMemo(() => new Map(rows.map((r) => [r._id, r])), [rows]);
  const targets = useCallback((): ThreadRow[] => {
    if (selection.size > 0) return rows.filter((r) => selection.has(r._id));
    if (openId) return detail ? [detail.thread] : [];
    const r = cursorId ? rowById.get(cursorId as ThreadId) : undefined;
    return r ? [r] : [];
  }, [selection, rows, openId, detail, cursorId, rowById]);

  /** After rows leave the view: move the cursor past them and auto-advance an open thread (Superhuman). */
  const advancePast = useCallback(
    (leave: Set<string>) => {
      if (leave.size === 0) return;
      const anchor = openId ?? cursorId;
      const idx = rows.findIndex((r) => r._id === anchor);
      let next: string | null = null;
      for (let i = idx + 1; i < rows.length && !next; i++) if (!leave.has(rows[i]!._id)) next = rows[i]!._id;
      for (let i = idx - 1; i >= 0 && !next; i--) if (!leave.has(rows[i]!._id)) next = rows[i]!._id;
      setSelection(new Set());
      if (cursorId && leave.has(cursorId)) setCursor(next);
      if (openId && leave.has(openId)) {
        if (next) openThread(next);
        else closeThread();
      }
    },
    [openId, cursorId, rows, setCursor, openThread, closeThread],
  );

  const act = useCallback(
    (build: BuildAction, title: ((n: number) => string) | null, rowsIn?: ThreadRow[]) => {
      const t = rowsIn ?? targets();
      if (t.length === 0) return;
      const leave = actions.leaving(t, build, isSearch || isReminders ? null : listArgs);
      const restore = openId && leave.has(openId) ? location : undefined;
      actions.perform(t, build, title, restore ? { restore } : {});
      advancePast(leave);
    },
    [targets, actions, isSearch, isReminders, listArgs, openId, location, advancePast],
  );

  const plural = (n: number, one: string, many: string) =>
    n === 1 ? one : `${many.replace("{n}", String(n))}`;
  const archive = () =>
    act(
      (a) => a.archive(),
      (n) => plural(n, "Archived.", "Archived {n} conversations."),
    );
  const trash = () =>
    act(
      (a) => a.trash(),
      (n) => plural(n, "Moved to Trash.", "Moved {n} conversations to Trash."),
    );
  const spam = () =>
    act(
      (a) => a.spam(),
      (n) => plural(n, "Reported as spam.", "Reported {n} conversations as spam."),
    );
  const moveToInbox = () =>
    act(
      (a) => a.moveToInbox(),
      (n) => plural(n, "Moved to Inbox.", "Moved {n} conversations to Inbox."),
    );
  const toggleRead = () => {
    const t = targets();
    const anyUnread = t.some((r) => r.labelKeys.includes(SYS.unread));
    act(
      (a) => (anyUnread ? a.markRead() : a.markUnread()),
      (n) =>
        anyUnread
          ? plural(n, "Marked as read.", "Marked {n} as read.")
          : plural(n, "Marked as unread.", "Marked {n} as unread."),
      t,
    );
  };
  const toggleStar = () => {
    const t = targets();
    const allStarred = t.length > 0 && t.every((r) => r.labelKeys.includes(SYS.starred));
    act(
      (a) => (allStarred ? a.unstar() : a.star()),
      (n) => (allStarred ? plural(n, "Unstarred.", "Unstarred {n}.") : plural(n, "Starred.", "Starred {n}.")),
      t,
    );
  };
  const undo = async () => {
    const g = await actions.undoLast();
    if (!g) return;
    if (g.restore) navigate(g.restore);
    if (g.threadIds[0]) setCursor(g.threadIds[0]);
  };

  const onPickTime = (at: number, returnUnread: boolean) => {
    const t = targets();
    if (t.length === 0) return;
    if (picker?.kind === "remind") {
      void actions.setReminder(t[0]!, at);
      return;
    }
    const leave = actions.leaving(t, (a) => a.snooze(), isSearch || isReminders ? null : listArgs);
    const restore = openId && leave.has(openId) ? location : undefined;
    actions.snooze(
      t,
      at,
      returnUnread,
      t.length === 1 ? "Snoozed." : `Snoozed ${t.length} conversations.`,
      restore ? { restore } : {},
    );
    advancePast(leave);
  };

  // ───────────── pickers ─────────────
  const pickerItems = useMemo((): PickerItem[] => {
    if (!picker) return [];
    if (picker.kind === "goLabel")
      return visibleLabels(labels).map((l) => ({
        id: l._id,
        label: l.name,
        detail: accounts.length > 1 ? accounts.find((a) => a._id === l.accountId)?.email : undefined,
      }));
    const t = targets();
    const accountIds = new Set(t.map((r) => r.accountId));
    if (accountIds.size !== 1) return [];
    const acc = [...accountIds][0]!;
    if (picker.kind === "label")
      return visibleLabels(labels.filter((l) => l.accountId === acc)).map((l) => ({
        id: l._id,
        label: l.name,
        checked: t.every((r) => r.labelKeys.includes(l.labelKey)),
      }));
    if (picker.kind === "move") {
      const folders = labels.filter(
        (l) => l.accountId === acc && l.exclusive && l.role !== "drafts" && l.role !== "sent",
      );
      return folders.map((l) => ({ id: l.labelKey, label: l.name }));
    }
    return [];
  }, [picker, labels, accounts, targets]);

  const onPickItem = (item: PickerItem) => {
    if (!picker) return;
    if (picker.kind === "goLabel") return go({ kind: "label", labelId: item.id });
    if (picker.kind === "label") {
      const remove = item.checked === true;
      act(
        (a: ThreadActions) => (remove ? a.removeLabel(item.id) : a.addLabel(item.id)),
        (n) => `${remove ? "Removed" : "Added"} ${item.label}${n > 1 ? ` (${n})` : ""}.`,
      );
    }
    if (picker.kind === "move")
      act(
        (a) => a.moveTo(item.id),
        () => `Moved to ${item.label}.`,
      );
  };

  // "Create label “…”" in the label picker: labels.create, then the label goes on the targets right away (the
  // thread op waits for the label op on the worker, §5.4).
  const createLabelAndApply = (name: string) => {
    const t = targets();
    const accountIds = [...new Set(t.map((r) => r.accountId))];
    if (accountIds.length !== 1) return;
    void m
      .createLabel({ clientOpId: uuidv7(), accountId: accountIds[0]!, name })
      .then(({ labelId }) =>
        act(
          (a: ThreadActions) => a.addLabel(labelId),
          (n) => `Created and added ${name}${n > 1 ? ` (${n})` : ""}.`,
          t,
        ),
      )
      .catch((e) => toast({ title: errorMessage(e), tone: "error" }));
  };

  // ───────────── theme, search, account helpers ─────────────
  const setTheme = (t: ThemePref) => {
    void setAppearance({ theme: t }, savePrefs).catch(() => undefined);
  };
  const setDensity = (d: "compact" | "comfortable") => {
    void setAppearance({ density: d }, savePrefs).catch(() => undefined);
  };
  const submitSearch = (q: string) => {
    routeTo({ name: "search", q, ...(accountId && { accountId }) });
    searchInput.current?.blur();
    queueMicrotask(() => listEl.current?.focus());
  };
  const closeSearch = () => {
    searchInput.current?.blur();
    if (isSearch) go({ kind: "inbox" });
    else setQuery("");
    queueMicrotask(() => listEl.current?.focus());
  };
  const runFullSearch = async () => {
    if (!searchArgs) return;
    const q = searchArgs.q;
    setFullSearch({ q, ids: [], busy: true });
    const targetsAcc = accountId ? accounts.filter((a) => a._id === accountId) : accounts;
    const ids: ThreadId[] = [];
    try {
      const results = await Promise.allSettled(targetsAcc.map((a) => backend.worker.search(a._id, q)));
      for (const r of results) if (r.status === "fulfilled") ids.push(...r.value);
      const failed = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
      if (failed) toast({ title: errorMessage(failed.reason), tone: "error" });
    } finally {
      setFullSearch({ q, ids, busy: false });
    }
  };
  const reconnect = async (a: MailAccountView) => {
    const provider = a.provider === "gmail" ? "google" : a.provider === "graph" ? "microsoft" : null;
    if (!provider) return navigate("/onboarding/imap");
    try {
      window.location.assign(
        await backend.worker.connectStart(provider, {
          returnTo: `${window.location.origin}/onboarding`,
          loginHint: a.email,
          accountId: a._id,
        }),
      );
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
    }
  };

  // ───────────── split tabs ─────────────
  const tabs = useMemo(
    () => [
      { key: SYS.inbox, name: "Inbox", view: { kind: "inbox" } as MailboxView },
      ...splits.map((s) => ({
        key: s.labelKey,
        name: s.name,
        view: { kind: "inbox", split: s.labelKey } as MailboxView,
      })),
      { key: SPLIT_OTHER, name: "Other", view: { kind: "inbox", split: SPLIT_OTHER } as MailboxView },
    ],
    [splits],
  );
  const inInbox = route.name === "mail" && route.view.kind === "inbox";
  const cycleTab = (d: 1 | -1) => {
    if (!inInbox) return go({ kind: "inbox" });
    const i = tabs.findIndex((t) => sameView(t.view, view));
    go(tabs[(i + d + tabs.length) % tabs.length]!.view);
  };

  // ───────────── shortcuts ─────────────
  const move = (d: 1 | -1) => {
    if (rows.length === 0) return;
    const anchor = openId ?? cursorId;
    const i = rows.findIndex((r) => r._id === anchor);
    const j = Math.max(0, Math.min(rows.length - 1, (i === -1 ? 0 : i) + d));
    const id = rows[j]!._id;
    if (openId) {
      if (id !== openId) openThread(id);
    } else setCursor(id);
    if (j >= rows.length - 5 && status === "more") loadMore(50);
  };
  const extend = (d: 1 | -1) => {
    if (!cursorId) return;
    const i = rows.findIndex((r) => r._id === cursorId);
    const j = Math.max(0, Math.min(rows.length - 1, i + d));
    setSelection((s) => new Set([...s, cursorId, rows[j]!._id]));
    setCursor(rows[j]!._id);
  };
  const t0 = targets();
  const canArchive = actions.available("archive", t0);
  const canMove = t0.length > 0 && t0.every((r) => actions.isFolder(r.accountId));

  const commandCtx: CommandContext = { targets: t0.map((r) => r._id), openThread: detail ?? null, navigate };
  const handlers: Record<string, (() => void) | undefined> = {
    "palette.open": () => setPaletteOpen(true),
    "help.shortcuts": () => setHelpOpen(true),
    "search.open": () => searchInput.current?.focus(),
    undo: () => void undo(),
    "go.inbox": () => go({ kind: "inbox" }),
    "go.starred": () => go({ kind: "sys", name: "starred" }),
    "go.sent": () => go({ kind: "sys", name: "sent" }),
    "go.drafts": () => go({ kind: "sys", name: "drafts" }),
    "go.snoozed": () => go({ kind: "sys", name: "snoozed" }),
    "go.reminders": () => go({ kind: "reminders" }),
    "go.all": () => go({ kind: "sys", name: "all" }),
    "go.spam": () => go({ kind: "sys", name: "spam" }),
    "go.trash": () => go({ kind: "sys", name: "trash" }),
    "go.label": () => setPicker({ kind: "goLabel" }),
    // With nothing focused, Tab first puts the keyboard on the list; on the list it switches splits.
    "split.next": () => (listFocused() ? cycleTab(1) : focusList()),
    "split.prev": () => (listFocused() ? cycleTab(-1) : focusList()),
    "nav.next": () => move(1),
    "nav.prev": () => move(-1),
    "nav.open": () => cursorId && openThread(cursorId),
    "nav.back": () => {
      if (openId) closeThread();
      else if (selection.size > 0) setSelection(new Set());
      else if (isSearch) closeSearch();
    },
    "select.toggle": () =>
      cursorId &&
      setSelection((s) => {
        const n = new Set(s);
        if (n.has(cursorId)) n.delete(cursorId);
        else n.add(cursorId);
        return n;
      }),
    "select.extendDown": () => extend(1),
    "select.extendUp": () => extend(-1),
    "select.all": () => setSelection(new Set(rows.map((r) => r._id))),
    ...(t0.length > 0 && {
      "thread.archive": canArchive ? archive : undefined,
      "thread.moveToInbox": moveToInbox,
      "thread.snooze": () => setPicker({ kind: "snooze" }),
      "thread.remind": t0.length === 1 ? () => setPicker({ kind: "remind" }) : undefined,
      "thread.trash": trash,
      "thread.spam": spam,
      "thread.toggleRead": toggleRead,
      "thread.star": toggleStar,
      "thread.label": () => setPicker({ kind: "label" }),
      "thread.move": canMove ? () => setPicker({ kind: "move" }) : undefined,
    }),
    ...(openId && {
      "thread.expandAll": () => setExpandAllTick((n) => n + 1),
      "thread.showImages": () => setShowImagesTick((n) => n + 1),
      "thread.openOriginal": () => openOriginal(),
    }),
  };
  for (const e of extensions())
    for (const [id, run] of Object.entries(e.commands?.(commandCtx) ?? {})) if (run) handlers[id] = run;

  // Synchronous (bodies already in memory), so the new tab opens inside the key press or click.
  function openOriginal() {
    if (!detail) return;
    for (const m of [...detail.messages].reverse()) {
      const body = bodies.cache.peek(m._id);
      if (!body) continue;
      openOriginalTab(
        originalPage({
          subject: detail.thread.subject,
          meta: m,
          body,
          date: fullDate(m.date, tz),
          showImages: prefs.remoteImages === "proxy",
          syncOrigin,
        }),
      );
      return;
    }
    toast({ title: "The message is still loading." });
  }

  const anyDialog = paletteOpen || helpOpen || picker !== null;
  // When a dialog closes and focus fell back to the page, the list takes it again, so keys and screen readers
  // keep working (the list announces its active row through aria-activedescendant).
  // Two ticks: a closing dialog gives focus back first (Radix does it in a timeout after the content unmounts,
  // which is a render after this one), e.g. the palette returns it to the draft it was opened from.
  useEffect(() => {
    if (anyDialog) return;
    let inner: ReturnType<typeof setTimeout> | undefined;
    const t = setTimeout(() => {
      inner = setTimeout(() => {
        if (document.activeElement === document.body || document.activeElement === null) focusList();
      }, 0);
    }, 0);
    return () => {
      clearTimeout(t);
      if (inner) clearTimeout(inner);
    };
  }, [anyDialog, focusList]);
  // After moving to another mailbox, label or search (keys, sidebar click, palette), focus goes to the list
  // once its rows are there, or to the empty state, so screen readers keep the list context and focus is
  // never left on <body> or the clicked sidebar link. A field or dialog the user moved to meanwhile keeps it.
  const listKey = JSON.stringify([
    route.name,
    route.name === "mail" ? route.view : routeQ,
    accountId ?? null,
  ]);
  const lastListKey = useRef(listKey);
  const focusAfterNav = useRef(false);
  if (lastListKey.current !== listKey) {
    lastListKey.current = listKey;
    focusAfterNav.current = true;
  }
  useEffect(() => {
    if (!focusAfterNav.current || anyDialog) return;
    if (rows.length === 0 && status !== "done") return;
    focusAfterNav.current = false;
    const a = document.activeElement as HTMLElement | null;
    const movable =
      !a ||
      a === document.body ||
      a === listEl.current ||
      a.closest(".ak-sidebar, .ak-listpane__empty") !== null;
    if (!movable) return;
    if (rows.length > 0) focusList();
    else document.querySelector<HTMLElement>(".ak-listpane__empty")?.focus({ preventScroll: true });
  });

  // The drawer takes focus when it opens and gives it back to the list on Escape.
  useEffect(() => {
    if (!navOpen) return;
    document.querySelector<HTMLElement>(".ak-sidebar a, .ak-sidebar button")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      setNavOpen(false);
      focusList();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [navOpen, focusList]);
  // F6 / Shift+F6 cycle between the landmarks: mailboxes, search, conversations, reading pane.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F6" || e.altKey || e.ctrlKey || e.metaKey) return;
      if (document.activeElement?.closest(".ak-dialog")) return;
      e.preventDefault();
      cycleLandmark(e.shiftKey ? -1 : 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const overrideScopes = scopeOverride();
  useShortcuts({
    platform: ui.platform,
    keymap: ui.keymap,
    scopes: overrideScopes ?? (searchFocused ? ["search"] : openId ? ["mail", "thread"] : ["mail", "list"]),
    handlers,
    enabled: !anyDialog,
  });

  // ───────────── palette ─────────────
  // Rebuilt every render (a few dozen items), so it always offers exactly what this screen can do.
  const paletteItems = ((): PaletteItem[] => {
    const defined = Object.fromEntries(Object.entries(handlers).filter(([, v]) => v !== undefined)) as Record<
      string,
      () => void
    >;
    const items = paletteItemsFromShortcuts(defined, { platform: ui.platform, keymap: ui.keymap });
    for (const s of splits)
      items.push({
        id: `split:${s._id}`,
        label: `Go to ${s.name}`,
        section: "Splits",
        run: () => go({ kind: "inbox", split: s.labelKey }),
      });
    items.push({
      id: "split:other",
      label: "Go to Other",
      section: "Splits",
      run: () => go({ kind: "inbox", split: SPLIT_OTHER }),
    });
    for (const l of visibleLabels(labels))
      items.push({
        id: `label:${l._id}`,
        label: `Go to ${l.name}`,
        section: "Labels",
        keywords: ["label"],
        run: () => go({ kind: "label", labelId: l._id }),
      });
    if (accounts.length > 1) {
      items.push({
        id: "account:all",
        label: "Show all accounts",
        section: "Accounts",
        run: () => routeTo({ ...route, accountId: undefined, threadId: undefined } as Route),
      });
      for (const a of accounts)
        items.push({
          id: `account:${a._id}`,
          label: `Show only ${a.email}`,
          section: "Accounts",
          run: () => routeTo({ ...route, accountId: a._id, threadId: undefined } as Route),
        });
    }
    items.push({
      id: "account:connect",
      label: "Connect a mailbox",
      section: "Accounts",
      keywords: ["add", "gmail", "outlook", "imap"],
      run: () => navigate("/onboarding"),
    });
    for (const a of accounts.filter((x) => x.status === "reauth_required"))
      items.push({
        id: `account:reconnect:${a._id}`,
        label: `Reconnect ${a.email}`,
        section: "Accounts",
        run: () => void reconnect(a),
      });
    for (const t of ["light", "dark", "system"] as const)
      items.push({
        id: `theme:${t}`,
        label: `Use ${t === "system" ? "the system" : t} theme`,
        section: "Appearance",
        keywords: ["theme", "appearance", `${t} mode`],
        disabled: theme.pref === t,
        run: () => setTheme(t),
      });
    items.push(
      {
        id: "density:compact",
        label: "Use compact rows",
        section: "Appearance",
        keywords: ["density"],
        disabled: density === "compact",
        run: () => setDensity("compact"),
      },
      {
        id: "density:comfortable",
        label: "Use comfortable rows",
        section: "Appearance",
        keywords: ["density"],
        disabled: density === "comfortable",
        run: () => setDensity("comfortable"),
      },
    );
    if (isSearch && searchArgs)
      items.push({
        id: "search:full",
        label: "Search the full mailbox",
        section: "Search",
        run: () => void runFullSearch(),
      });
    items.push({
      id: "session:signout",
      label: "Sign out",
      section: "Account",
      keywords: ["log out", "logout", "sign off"],
      run: () => void backend.auth.signOut(),
    });
    for (const e of extensions()) items.push(...(e.paletteItems?.(commandCtx) ?? []));
    return items;
  })();

  const rememberRun = (item: PaletteItem) => {
    const next = [item.id, ...recent.filter((x) => x !== item.id)].slice(0, 5);
    setRecent(next);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch {
      /* ignore */
    }
  };

  // ───────────── render ─────────────
  const title = isSearch
    ? "Search"
    : view.kind === "label"
      ? (labels.find((l) => l._id === view.labelId)?.name ?? "Label")
      : view.kind === "sys"
        ? VIEW_TITLES[view.name]
        : view.kind === "reminders"
          ? "Reminders"
          : "Inbox";
  const empty = isSearch ? (
    <NoResults q={searchArgs?.q ?? ""} />
  ) : (
    <EmptyView view={view} noAccounts={accounts.length === 0} />
  );
  const toolbar = detail ? (
    <ThreadToolbar
      canArchive={canArchive}
      starred={detail.thread.labelKeys.includes(SYS.starred)}
      unread={detail.thread.labelKeys.includes(SYS.unread)}
      onArchive={archive}
      onTrash={trash}
      onSnooze={() => setPicker({ kind: "snooze" })}
      onRemind={() => (handlers["thread.remind"] ?? (() => setPicker({ kind: "remind" })))()}
      onStar={toggleStar}
      onRead={toggleRead}
      onLabel={() => setPicker({ kind: "label" })}
    />
  ) : null;

  return (
    <div className="ak-shell" data-pane={openId ? "open" : "closed"} data-nav={navOpen ? "open" : undefined}>
      <button type="button" className="ak-skiplink" onClick={focusList}>
        Skip to conversations
      </button>
      <Sidebar
        view={isSearch ? null : view}
        accountId={accountId}
        accounts={accounts}
        labels={labels}
        splits={splits}
        counts={counts}
        theme={theme.pref}
        onToggleTheme={() => setTheme(nextTheme(theme.pref))}
        onOpenPalette={() => setPaletteOpen(true)}
        onSignOut={() => void backend.auth.signOut()}
        onConnect={() => navigate("/onboarding")}
        onOpenSettings={() => navigate("/settings")}
      />
      {navOpen && (
        <button
          type="button"
          className="ak-navscrim"
          aria-label="Close menu"
          tabIndex={-1}
          onClick={() => setNavOpen(false)}
        />
      )}
      <main className="ak-main" id="conversations">
        <section className="ak-listpane" aria-label={title}>
          <header className="ak-listpane__header">
            <div className="ak-compactbar">
              <IconButton
                label="Menu"
                icon={<IconMenu />}
                aria-expanded={navOpen}
                onClick={() => setNavOpen((o) => !o)}
              />
              <IconButton
                label="Command palette"
                icon={<IconCommand />}
                onClick={() => setPaletteOpen(true)}
              />
              <IconButton
                label="Compose"
                icon={<IconPlus />}
                shortcut="compose.new"
                onClick={() => handlers["compose.new"]?.()}
              />
            </div>
            <div className="ak-listpane__titlebar">
              <h1 className="ak-listpane__title">
                <Eyebrow>{title}</Eyebrow>
              </h1>
              {selection.size > 0 && <span className="ak-listpane__selected">{selection.size} selected</span>}
            </div>
            <SearchBar
              value={query}
              onChange={setQuery}
              onSubmit={submitSearch}
              onClose={closeSearch}
              onFocusChange={setSearchFocused}
              inputRef={searchInput}
              timezone={tz}
              onContactQuery={setContactQ}
              context={{
                contacts: contacts ?? [],
                labels: visibleLabels(labels).map((l) => l.name),
                splits: splits.map((s) => s.name),
                recent: lastSearch,
              }}
            />
            {inInbox && (
              <div className="ak-tabs" role="tablist" aria-label="Splits">
                {tabs.map((t) => {
                  const selected = sameView(t.view, view);
                  const n = t.key === SYS.inbox ? undefined : counts?.[t.key];
                  return (
                    <button
                      key={t.key}
                      type="button"
                      role="tab"
                      aria-selected={selected}
                      tabIndex={-1}
                      className="ak-tab"
                      onClick={() => go(t.view)}
                    >
                      {t.name}
                      {n ? <span className="ak-tab__count">{n >= 100 ? "99+" : n}</span> : null}
                    </button>
                  );
                })}
              </div>
            )}
            {isSearch && searchArgs && (search.unsupported.length > 0 || search.status === "done") && (
              <div className="ak-searchnote">
                {search.unsupported.length > 0 && (
                  <span>Some of this query ({search.unsupported.join(", ")}) needs the full mailbox.</span>
                )}
                <button
                  type="button"
                  className="ak-link"
                  onClick={() => void runFullSearch()}
                  disabled={fullSearch?.busy}
                >
                  {fullSearch?.busy ? "Searching the full mailbox…" : "Search the full mailbox"}
                </button>
              </div>
            )}
          </header>
          <Banners accounts={accounts} onReconnect={(a) => void reconnect(a)} />
          <ThreadList
            rows={rows}
            status={status}
            loadMore={loadMore}
            cursorId={cursorId}
            openId={openId}
            selected={selection}
            density={density}
            timezone={tz}
            now={Date.now()}
            myEmails={myEmails}
            labelsByKey={labelsByKey}
            showSnoozedUntil={
              route.name === "mail" && route.view.kind === "sys" && route.view.name === "snoozed"
            }
            ariaLabel={`${title} conversations`}
            empty={empty}
            onActivate={openThread}
            onToggleSelect={(id) =>
              setSelection((s) => {
                const n = new Set(s);
                if (n.has(id)) n.delete(id);
                else n.add(id);
                return n;
              })
            }
            listRef={listEl}
          />
        </section>
        <section className="ak-reading" aria-label="Reading pane">
          {openId && (
            <div className="ak-backbar">
              <button type="button" className="ak-backbar__button" onClick={closeThread}>
                <IconBack />
                <span>Back to {title}</span>
              </button>
            </div>
          )}
          <ErrorBoundary
            resetKey={openId ?? ""}
            fallback={(e, reset) => <ErrorState error={e} onRetry={reset} />}
          >
            {openId ? (
              detail === undefined ? (
                <ThreadSkeleton />
              ) : detail === null ? (
                <ErrorState error={{ code: "not_found" }} onRetry={closeThread} />
              ) : (
                <ThreadView
                  detail={detail}
                  labelsByKey={labelsByKey}
                  timezone={tz}
                  prefs={prefs}
                  multiAccount={accounts.length > 1}
                  showImagesTick={showImagesTick}
                  expandAllTick={expandAllTick}
                  toolbar={toolbar}
                />
              )
            ) : (
              <ReadingEmpty count={rows.length} />
            )}
          </ErrorBoundary>
        </section>
      </main>
      <ErrorBoundary resetKey={openId ?? cursorId ?? ""} fallback={() => null}>
        <Prefetch rows={rows} anchor={openId ?? cursorId} />
      </ErrorBoundary>
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        items={paletteItems}
        recent={recent}
        onRun={rememberRun}
      />
      <HelpDialog open={helpOpen} onOpenChange={setHelpOpen} />
      {extensions()
        .filter((e) => e.overlay)
        .map((e) => {
          const Overlay = e.overlay!;
          return (
            <ErrorBoundary key={e.id} resetKey={e.id} fallback={(err) => <ErrorState error={err} inline />}>
              <Overlay me={me} />
            </ErrorBoundary>
          );
        })}
      <TimePicker
        open={picker?.kind === "snooze" || picker?.kind === "remind"}
        mode={picker?.kind === "remind" ? "remind" : "snooze"}
        count={t0.length}
        timezone={tz}
        onOpenChange={(o) => !o && setPicker(null)}
        onPick={onPickTime}
      />
      <ListPicker
        open={picker?.kind === "label" || picker?.kind === "move" || picker?.kind === "goLabel"}
        title={
          picker?.kind === "move" ? "Move to folder" : picker?.kind === "goLabel" ? "Go to label" : "Labels"
        }
        placeholder={picker?.kind === "move" ? "Folder" : "Label"}
        emptyText={
          picker?.kind === "label" && new Set(t0.map((r) => r.accountId)).size > 1
            ? "Pick conversations from one account to label them."
            : "Nothing matches."
        }
        items={pickerItems}
        onOpenChange={(o) => !o && setPicker(null)}
        onPick={onPickItem}
        onCreate={
          picker?.kind === "label" && new Set(t0.map((r) => r.accountId)).size === 1
            ? createLabelAndApply
            : undefined
        }
      />
    </div>
  );
}

function ThreadSkeleton() {
  return (
    <div
      className="ak-thread ak-thread--skeleton"
      role="status"
      aria-busy="true"
      aria-label="Loading conversation"
    >
      <span className="ak-skeleton ak-skeleton--title" />
      <span className="ak-skeleton" />
      <span className="ak-skeleton ak-skeleton--short" />
    </div>
  );
}

interface ToolbarProps {
  canArchive: boolean;
  starred: boolean;
  unread: boolean;
  onArchive: () => void;
  onTrash: () => void;
  onSnooze: () => void;
  onRemind: () => void;
  onStar: () => void;
  onRead: () => void;
  onLabel: () => void;
}

function ThreadToolbar(p: ToolbarProps): ReactNode {
  return (
    <div className="ak-toolbar" role="toolbar" aria-label="Conversation actions">
      {p.canArchive && (
        <IconButton label="Archive" icon={<IconArchive />} shortcut="thread.archive" onClick={p.onArchive} />
      )}
      <IconButton label="Move to Trash" icon={<IconTrash />} shortcut="thread.trash" onClick={p.onTrash} />
      <IconButton label="Snooze" icon={<IconClock />} shortcut="thread.snooze" onClick={p.onSnooze} />
      <IconButton
        label="Remind me if no reply"
        icon={<IconBell />}
        shortcut="thread.remind"
        onClick={p.onRemind}
      />
      <IconButton
        label={p.starred ? "Unstar" : "Star"}
        icon={<IconStar filled={p.starred} />}
        shortcut="thread.star"
        pressed={p.starred}
        onClick={p.onStar}
      />
      <IconButton
        label={p.unread ? "Mark as read" : "Mark as unread"}
        icon={p.unread ? <IconMailOpen /> : <IconMail />}
        shortcut="thread.toggleRead"
        onClick={p.onRead}
      />
      <IconButton label="Labels" icon={<IconTag />} shortcut="thread.label" onClick={p.onLabel} />
    </div>
  );
}

const LANDMARKS = [".ak-sidebar", ".ak-search__input", ".ak-threadlist", ".ak-reading"] as const;

/** Moves focus to the next (or previous) landmark that is on screen (F6). */
function cycleLandmark(d: 1 | -1) {
  const targets = LANDMARKS.map((sel) => {
    const el = document.querySelector<HTMLElement>(sel);
    if (!el || el.getClientRects().length === 0) return null;
    if (sel === ".ak-sidebar" || sel === ".ak-reading")
      return el.querySelector<HTMLElement>("button, a[href], input, [tabindex='0']") ?? el;
    return el;
  }).filter((x): x is HTMLElement => x !== null);
  if (targets.length === 0) return;
  const at = targets.findIndex((t) => t === document.activeElement || t.contains(document.activeElement));
  const roots = LANDMARKS.map((sel) => document.querySelector<HTMLElement>(sel));
  const inRoot = roots.findIndex((r) => r?.contains(document.activeElement ?? null));
  const from = at >= 0 ? at : inRoot >= 0 ? Math.min(inRoot, targets.length - 1) : -1;
  const next = targets[(from + d + targets.length) % targets.length]!;
  if (next.matches(".ak-reading") && !next.hasAttribute("tabindex")) next.setAttribute("tabindex", "-1");
  next.focus({ preventScroll: true });
}

/** Keeps the next two conversations warm: their live query subscribed and their bodies cached (instant j/k). */
function Prefetch({ rows, anchor }: { rows: readonly ThreadRow[]; anchor: string | null }) {
  const i = anchor ? rows.findIndex((r) => r._id === anchor) : -1;
  const next = rows.slice(i + 1, i + 3).map((r) => r._id);
  return (
    <>
      <PrefetchOne id={next[0] ?? null} />
      <PrefetchOne id={next[1] ?? null} />
    </>
  );
}

function PrefetchOne({ id }: { id: ThreadId | null }) {
  const backend = useBackend();
  const bodies = useBodies();
  const detail = backend.useThread(id);
  const thread = detail?.thread._id;
  const ids = detail?.messages.map((m) => m._id).join(",") ?? "";
  useEffect(() => {
    if (!thread || !ids) return;
    const t = setTimeout(() => void bodies.loadThread(thread, ids.split(",") as never), 150);
    return () => clearTimeout(t);
  }, [bodies, thread, ids]);
  return null;
}

export type { ThreadDetail };
