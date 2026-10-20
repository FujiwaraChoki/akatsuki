// In-memory mail store with Convex-like semantics: visible keys are `applyOps` results, lists follow
// `viewContains` from @akatsuki/mail-model, undo uses `inverseAction`, ops complete after a short simulated
// provider latency, and one seeded thread fails to archive (op failure → toast with Retry / Dismiss).
// Session flags persist in localStorage so a full-page "OAuth redirect" comes back to the same state.
import {
  applyAction,
  applyContextFor,
  diffKeys,
  type Effect,
  inverseAction,
  isFolderProvider,
  parseQuery,
  type QueryAst,
  SYS,
  type ThreadAction,
} from "@akatsuki/mail-model";
import { viewContains } from "@akatsuki/mail-model/optimistic";
import type {
  AccountId,
  LabelId,
  LabelView,
  ListViewArgs,
  MailAccountView,
  MessageMeta,
  MeView,
  OpFailure,
  Prefs,
  SplitView,
  ThreadDetail,
  ThreadId,
  ThreadRow,
  TimerView,
} from "../types.ts";
import { buildSeed, DEMO_OTP, mockAccount, splitFor, VIP_SENDERS } from "./data.ts";
import { MockExtras, type MockPlan } from "./extras.ts";

const SESSION_KEY = "ak:mock-session";

interface Session {
  email: string | null;
  onboarding: "checkout" | "connect" | "done";
  plan?: MockPlan;
  accounts: { provider: "gmail" | "imap"; email: string }[];
  prefs: Partial<Prefs>;
}

const DEFAULT_PREFS: Prefs = {
  undoSendSeconds: 10,
  remoteImages: "block",
  readReceipts: { enabled: false },
  cacheBodiesOnDevice: true,
  density: "comfortable",
  theme: "system",
  keymap: "superhuman",
  unifiedInbox: true,
  notifications: "private",
};

interface MockThread {
  row: ThreadRow;
  messages: MessageMeta[];
  failsOnArchive?: boolean;
}

interface MockOp {
  clientOpId: string;
  threadIds: ThreadId[];
  effects: Map<string, { action: ThreadAction; effect: Effect; before: string[] }>;
  snooze?: boolean;
}

export interface MockOptions {
  /** Start signed in (tests). Default: whatever the persisted session says. */
  signedIn?: boolean;
  /** Start with accounts connected and onboarding done (tests). */
  onboarded?: boolean;
  /** Persist the session in localStorage (default true). */
  persist?: boolean;
  /** Simulated provider latency in ms (default 350). */
  latencyMs?: number;
  now?: () => number;
  /** Billing state (default: a running Pro trial). `none` starts new users at checkout (§13.1). */
  plan?: MockPlan;
}

export class MockStore {
  private listeners = new Set<() => void>();
  version = 0;
  session: Session;
  accounts: MailAccountView[] = [];
  labels: LabelView[] = [];
  splits: SplitView[] = [];
  threads = new Map<string, MockThread>();
  ops = new Map<string, MockOp>();
  failures: OpFailure[] = [];
  timers: TimerView[] = [];
  senderPrefs = new Map<string, { allowRemoteImages: boolean; vip: boolean }>();
  connected = true;
  extras: MockExtras;
  private persist: boolean;
  readonly latencyMs: number;
  readonly now: () => number;
  private seq = 0;

  constructor(opts: MockOptions = {}) {
    this.persist = opts.persist ?? true;
    this.latencyMs = opts.latencyMs ?? 350;
    this.now = opts.now ?? (() => Date.now());
    const saved = this.persist ? this.load() : null;
    this.session = saved ?? { email: null, onboarding: "connect", accounts: [], prefs: {} };
    if (opts.plan) this.session.plan = opts.plan;
    if (
      this.session.plan === "none" &&
      this.session.onboarding === "connect" &&
      !this.session.accounts.length
    )
      this.session.onboarding = "checkout";
    if (opts.signedIn !== undefined) this.session.email = opts.signedIn ? "you@example.com" : null;
    if (opts.onboarded) {
      this.session.onboarding = "done";
      if (this.session.accounts.length === 0)
        this.session.accounts = [
          { provider: "gmail", email: "you@example.com" },
          { provider: "imap", email: "you@example.org" },
        ];
    }
    this.extras = new MockExtras(this, this.session.plan ?? "trialing");
    this.rebuild();
  }

  private load(): Session | null {
    try {
      const raw = localStorage.getItem(SESSION_KEY);
      return raw ? (JSON.parse(raw) as Session) : null;
    } catch {
      return null;
    }
  }

  save() {
    if (!this.persist) return;
    try {
      localStorage.setItem(SESSION_KEY, JSON.stringify(this.session));
    } catch {
      /* ignore */
    }
  }

  private rebuild() {
    const now = this.now();
    const accounts = this.session.accounts.map((a, i) => mockAccount(i + 1, a.provider, a.email, now));
    const seed = buildSeed(accounts, now);
    this.accounts = seed.accounts;
    this.labels = seed.labels;
    this.splits = seed.splits;
    this.threads = new Map(seed.threads.map((t) => [t.row._id, { ...t }]));
    this.extras?.syncAccounts(this.accounts);
  }

  subscribe = (cb: () => void) => {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  };

  getVersion = () => this.version;

  emit() {
    this.version++;
    for (const l of [...this.listeners]) l();
  }

  // ───────────── labels (labels.create / rename / remove, §4.2) ─────────────

  createLabel(accountId: AccountId, name: string): { labelId: LabelId; labelKey: string } {
    const n = name.trim();
    const account = this.accounts.find((a) => a._id === accountId);
    if (!account || !n || n.length > 225)
      throw codedError("invalid_argument", "Label names are 1 to 225 characters.");
    if (
      this.labels.some(
        (l) => l.accountId === accountId && !l.pendingDelete && l.name.toLowerCase() === n.toLowerCase(),
      )
    )
      throw codedError("invalid_argument", "A label with that name already exists.");
    const labelId = `${accountId}lblnew${this.labels.length}` as LabelId;
    const labelKey = `l:${labelId}`;
    this.labels = [
      ...this.labels,
      {
        _id: labelId,
        _creationTime: this.now(),
        accountId,
        name: n,
        kind: account.provider === "gmail" ? "user" : account.provider === "graph" ? "category" : "keyword",
        exclusive: false,
        hidden: false,
        pendingDelete: false,
        labelKey,
      } as LabelView,
    ];
    this.emit();
    return { labelId, labelKey };
  }

  renameLabel(labelId: LabelId, name: string) {
    const n = name.trim();
    if (!n || n.length > 225) throw codedError("invalid_argument", "Label names are 1 to 225 characters.");
    this.labels = this.labels.map((l) => (l._id === labelId ? { ...l, name: n } : l));
    this.emit();
    return null;
  }

  removeLabel(labelId: LabelId) {
    const label = this.labels.find((l) => l._id === labelId);
    if (!label) return null;
    this.labels = this.labels.filter((l) => l._id !== labelId);
    for (const t of this.threads.values()) {
      if (t.row.labelKeys.includes(label.labelKey))
        t.row = { ...t.row, labelKeys: t.row.labelKeys.filter((k) => k !== label.labelKey) };
    }
    this.emit();
    return null;
  }

  // ───────────── auth and profile ─────────────

  signIn(email: string) {
    this.session.email = email.trim().toLowerCase();
    this.save();
    this.emit();
  }

  verifyOtp(email: string, otp: string) {
    if (otp.trim() !== DEMO_OTP) {
      const err = new Error("Invalid code") as Error & { code: string };
      err.code = "invalid_argument";
      throw err;
    }
    this.signIn(email);
  }

  signOut() {
    this.session = { email: null, onboarding: "connect", accounts: [], prefs: {}, plan: this.session.plan };
    this.save();
    this.rebuild();
    this.emit();
  }

  me(): MeView | null {
    const email = this.session.email;
    if (!email) return null;
    return {
      userId: "mock-user",
      email,
      name: "You",
      profile: {
        _id: "profile1" as never,
        _creationTime: 0,
        email,
        name: "You",
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
        onboarding: this.session.onboarding,
        prefs: { ...DEFAULT_PREFS, ...this.session.prefs },
        createdAt: 0,
        updatedAt: 0,
      },
      entitlement: this.extras.billing.entitlement,
    };
  }

  updatePrefs(patch: Partial<Prefs>) {
    this.session.prefs = { ...this.session.prefs, ...patch };
    this.save();
    this.emit();
  }

  setOnboarding(state: "checkout" | "connect" | "done") {
    this.session.onboarding = state;
    this.save();
    this.emit();
  }

  /** Simulates a completed mailbox connection (the worker would call `worker.createAccount`). */
  connect(provider: "gmail" | "imap", email: string): AccountId {
    const existing = this.session.accounts.findIndex((a) => a.email === email);
    if (existing === -1) this.session.accounts.push({ provider, email });
    this.save();
    this.rebuild();
    this.emit();
    const i = existing === -1 ? this.session.accounts.length : existing + 1;
    return `acc${i}` as AccountId;
  }

  // ───────────── reads ─────────────

  account(id: string) {
    return this.accounts.find((a) => a._id === id);
  }

  private ctxFor(accountId: string) {
    const a = this.account(accountId);
    const exclusive = this.labels
      .filter((l) => l.accountId === accountId && l.exclusive && l.kind !== "system")
      .map((l) => l._id);
    return applyContextFor(a?.provider ?? "gmail", exclusive);
  }

  list(view: ListViewArgs): ThreadRow[] {
    const out: ThreadRow[] = [];
    for (const t of this.threads.values()) {
      const v = { keys: t.row.labelKeys, deleted: false };
      if (viewContains(view, v, t.row)) out.push(t.row);
    }
    const bySnooze = view.labelKey === SYS.snoozed;
    return out.sort((a, b) =>
      bySnooze ? (a.snoozedUntil ?? 0) - (b.snoozedUntil ?? 0) : b.sortAt - a.sortAt,
    );
  }

  thread(id: string): ThreadDetail | null {
    const t = this.threads.get(id);
    if (!t) return null;
    const a = this.account(t.row.accountId)!;
    return {
      thread: t.row,
      account: { _id: a._id, provider: a.provider, email: a.email, caps: a.caps },
      messages: t.messages,
      truncated: false,
      pendingOps: [],
      timers: this.timers.filter((x) => x.threadId === id),
    };
  }

  unreadCounts(keys: readonly string[], accountId?: string): Record<string, number> {
    const out: Record<string, number> = {};
    for (const k of keys)
      out[k] = Math.min(
        100,
        this.list({ labelKey: k, unreadOnly: true, ...(accountId && { accountId: accountId as AccountId }) })
          .length,
      );
    return out;
  }

  search(q: string, nowAt: number, accountId?: string): { rows: ThreadRow[]; unsupported: string[] } {
    const tz = this.me()?.profile.timezone ?? "UTC";
    const { ast } = parseQuery(q, { nowAt, timezone: tz });
    const unsupported = new Set<string>();
    const rows = [...this.threads.values()]
      .map((t) => t.row)
      .filter((r) => (accountId ? r.accountId === accountId : true))
      .filter((r) => this.matches(ast, r, nowAt, unsupported))
      .sort((a, b) => b.sortAt - a.sortAt);
    return { rows, unsupported: [...unsupported] };
  }

  private matches(ast: QueryAst, r: ThreadRow, nowAt: number, unsupported: Set<string>): boolean {
    if ("and" in ast) return ast.and.every((x) => this.matches(x, r, nowAt, unsupported));
    if ("or" in ast) {
      unsupported.add("OR");
      return ast.or.some((x) => this.matches(x, r, nowAt, unsupported));
    }
    if ("not" in ast) return !this.matches(ast.not, r, nowAt, unsupported);
    const people = [...r.participants, ...(r.lastFrom ? [r.lastFrom] : [])].map((p) =>
      `${p.name ?? ""} ${p.email}`.toLowerCase(),
    );
    const hay = `${r.subject} ${r.snippet} ${people.join(" ")}`.toLowerCase();
    if ("text" in ast) return hay.includes(ast.text.toLowerCase());
    if ("at" in ast) return ast.field === "before" ? r.lastMessageAt < ast.at : r.lastMessageAt >= ast.at;
    if ("days" in ast) {
      const edge = nowAt - ast.days * 86_400_000;
      return ast.field === "older_than" ? r.lastMessageAt < edge : r.lastMessageAt >= edge;
    }
    const value = ast.value.toLowerCase();
    switch (ast.field) {
      case "from":
        return (r.lastFrom ? `${r.lastFrom.name ?? ""} ${r.lastFrom.email}` : "")
          .toLowerCase()
          .includes(value);
      case "to":
      case "cc":
        return people.some((p) => p.includes(value));
      case "subject":
        return r.subject.toLowerCase().includes(value);
      case "is":
        if (value === "unread") return r.labelKeys.includes(SYS.unread);
        if (value === "read") return !r.labelKeys.includes(SYS.unread);
        if (value === "starred") return r.labelKeys.includes(SYS.starred);
        if (value === "snoozed") return r.labelKeys.includes(SYS.snoozed);
        if (value === "important") return r.labelKeys.includes(SYS.important);
        return false;
      case "has":
        return value === "attachment" && r.hasAttachments;
      case "in":
        if (value === "anywhere") return true;
        if (value === "archive")
          return !r.labelKeys.some((k) => [SYS.inbox, SYS.trash, SYS.spam].includes(k as never));
        return r.labelKeys.includes(`sys:${value}`);
      case "label": {
        const l = this.labels.find((x) => x.name.toLowerCase() === value);
        return l ? r.labelKeys.includes(l.labelKey) : false;
      }
      case "account":
        return this.account(r.accountId)?.email === value;
      case "filename":
        unsupported.add("filename:");
        return false;
      default:
        return false;
    }
  }

  // ───────────── writes ─────────────

  private setKeys(t: MockThread, keys: string[]) {
    const unread = keys.includes(SYS.unread);
    t.row = {
      ...t.row,
      labelKeys: keys,
      unreadCount: unread ? Math.max(1, t.row.unreadCount) : 0,
    };
    t.row.split = splitFor(t.row, this.splits);
  }

  private settleLater(t: MockThread, fail: (() => void) | null) {
    t.row = { ...t.row, pendingOpCount: t.row.pendingOpCount + 1 };
    setTimeout(() => {
      const cur = this.threads.get(t.row._id);
      if (!cur) return;
      fail?.();
      cur.row = { ...cur.row, pendingOpCount: Math.max(0, cur.row.pendingOpCount - 1) };
      this.emit();
    }, this.latencyMs);
  }

  apply(clientOpId: string, threadIds: ThreadId[], action: ThreadAction) {
    if (this.ops.has(clientOpId)) return { opIds: [] as string[] };
    const op: MockOp = { clientOpId, threadIds, effects: new Map() };
    const opIds: string[] = [];
    for (const id of threadIds) {
      const t = this.threads.get(id);
      if (!t) continue;
      const opId = `op${++this.seq}`;
      opIds.push(opId);
      const before = t.row.labelKeys;
      const after = applyAction({ keys: before, deleted: false }, action, this.ctxFor(t.row.accountId));
      op.effects.set(id, { action, effect: diffKeys(before, after.keys), before });
      this.setKeys(t, after.keys);
      if (action.kind === "delete_forever") this.threads.delete(id);
      const failing = t.failsOnArchive && !after.keys.includes(SYS.inbox) && before.includes(SYS.inbox);
      this.settleLater(
        t,
        failing
          ? () => {
              const cur = this.threads.get(id);
              if (!cur) return;
              this.setKeys(cur, before); // rollback "by construction": back to provider truth
              this.failures = [
                {
                  _id: opId as never,
                  threadId: id,
                  subject: cur.row.subject,
                  accountEmail: this.account(cur.row.accountId)?.email ?? "",
                  payloadKind: "thread",
                  error: {
                    code: "provider_invalid",
                    message: "The provider rejected the change.",
                    retryable: false,
                  },
                  state: "failed",
                  at: this.now(),
                },
                ...this.failures,
              ];
            }
          : null,
      );
    }
    this.ops.set(clientOpId, op);
    this.emit();
    return { opIds };
  }

  undo(clientOpId: string, undoClientOpId: string): { mode: "cancelled" | "inverse" | "noop" } {
    const op = this.ops.get(undoClientOpId);
    if (!op) {
      const err = new Error("not_found") as Error & { code: string };
      err.code = "not_found";
      throw err;
    }
    if (op.snooze) {
      this.unsnooze(clientOpId, op.threadIds);
      return { mode: "inverse" };
    }
    let any = false;
    for (const [id, e] of op.effects) {
      const t = this.threads.get(id);
      if (!t) continue;
      const folder = isFolderProvider(this.account(t.row.accountId)?.provider ?? "gmail");
      const inv = inverseAction(e.action, e.effect, folder);
      if (!inv) continue;
      const after = applyAction({ keys: t.row.labelKeys, deleted: false }, inv, this.ctxFor(t.row.accountId));
      this.setKeys(t, after.keys);
      this.settleLater(t, null);
      any = true;
    }
    this.ops.set(clientOpId, { clientOpId, threadIds: op.threadIds, effects: new Map() });
    this.emit();
    return { mode: any ? "inverse" : "noop" };
  }

  snooze(clientOpId: string, threadIds: ThreadId[], until: number, returnUnread: boolean) {
    const op: MockOp = { clientOpId, threadIds, effects: new Map(), snooze: true };
    for (const id of threadIds) {
      const t = this.threads.get(id);
      if (!t) continue;
      const a = this.account(t.row.accountId);
      const action: ThreadAction = isFolderProvider(a?.provider ?? "gmail")
        ? { kind: "move", to: SYS.snoozed }
        : { kind: "modify", add: [SYS.snoozed], remove: [SYS.inbox] };
      const after = applyAction(
        { keys: t.row.labelKeys, deleted: false },
        action,
        this.ctxFor(t.row.accountId),
      );
      this.setKeys(t, after.keys);
      t.row = { ...t.row, snoozedUntil: until };
      this.timers = this.timers.filter((x) => !(x.threadId === id && x.kind === "snooze"));
      this.timers.push({
        _id: `tmr${++this.seq}` as never,
        _creationTime: this.now(),
        accountId: t.row.accountId,
        threadId: id,
        kind: "snooze",
        fireAt: until,
        condition: "always",
        anchorAt: this.now(),
        returnUnread,
        state: "active",
        createdAt: this.now(),
      });
      this.settleLater(t, null);
    }
    this.ops.set(clientOpId, op);
    this.emit();
  }

  unsnooze(clientOpId: string, threadIds: ThreadId[]) {
    for (const id of threadIds) {
      const t = this.threads.get(id);
      if (!t) continue;
      const a = this.account(t.row.accountId);
      const action: ThreadAction = isFolderProvider(a?.provider ?? "gmail")
        ? { kind: "move", to: SYS.inbox }
        : { kind: "modify", add: [SYS.inbox], remove: [SYS.snoozed] };
      const after = applyAction(
        { keys: t.row.labelKeys, deleted: false },
        action,
        this.ctxFor(t.row.accountId),
      );
      this.setKeys(t, after.keys);
      const { snoozedUntil: _s, ...rest } = t.row;
      t.row = { ...rest, returnedAt: this.now() };
      this.timers = this.timers.filter((x) => !(x.threadId === id && x.kind === "snooze"));
      this.settleLater(t, null);
    }
    this.ops.set(clientOpId, { clientOpId, threadIds, effects: new Map() });
    this.emit();
  }

  setReminder(threadId: ThreadId, at: number, condition: "always" | "if_no_reply", note?: string) {
    const t = this.threads.get(threadId);
    if (!t) return;
    this.timers = this.timers.filter((x) => !(x.threadId === threadId && x.kind === "reminder"));
    this.timers.push({
      _id: `tmr${++this.seq}` as never,
      _creationTime: this.now(),
      accountId: t.row.accountId,
      threadId,
      kind: "reminder",
      fireAt: at,
      condition,
      anchorAt: this.now(),
      returnUnread: false,
      ...(note && { note }),
      state: "active",
      createdAt: this.now(),
    });
    t.row = { ...t.row, reminderAt: at };
    this.emit();
  }

  ackFailure(opIds: string[]) {
    this.failures = this.failures.filter((f) => !opIds.includes(f._id));
    this.emit();
  }

  retry(opId: string, clientOpId: string) {
    const f = this.failures.find((x) => x._id === opId);
    this.ackFailure([opId]);
    if (!f?.threadId) return;
    const t = this.threads.get(f.threadId);
    if (!t) return;
    t.failsOnArchive = false; // the demo failure succeeds on retry
    this.apply(clientOpId, [f.threadId], { kind: "modify", add: [], remove: [SYS.inbox] });
  }

  /** Re-evaluates every inbox thread's split after the split list changed (`internal.splits.recompute`). */
  resplit() {
    for (const t of this.threads.values()) t.row = { ...t.row, split: splitFor(t.row, this.splits) };
    this.emit();
  }

  setSenderPref(sender: string, patch: { allowRemoteImages?: boolean; vip?: boolean }) {
    const cur = this.senderPrefs.get(sender) ?? { allowRemoteImages: false, vip: VIP_SENDERS.has(sender) };
    this.senderPrefs.set(sender, { ...cur, ...patch });
    this.emit();
  }
}

function codedError(code: string, message: string): Error & { code: string } {
  const err = new Error(message) as Error & { code: string };
  err.code = code;
  return err;
}
