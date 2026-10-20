// Synthetic mailbox for the mock backend (tests, Playwright, offline demos). Every name, address and line of
// text here is invented; domains are the reserved example.* names. Deterministic: the same seed always gives
// the same mailbox, so a reload after the simulated OAuth redirect shows the same threads.
import { firstMatchingSplit, type RuleFacts, SPLIT_OTHER, SYS } from "@akatsuki/mail-model";
import type {
  AccountId,
  LabelView,
  MailAccountView,
  MessageBody,
  MessageId,
  MessageMeta,
  SplitView,
  ThreadId,
  ThreadRow,
} from "../types.ts";

export const DEMO_OTP = "123456";

function prng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const PEOPLE = [
  ["Mara Okafor", "mara@example.com"],
  ["Jun Tanaka", "jun@example.net"],
  ["Lena Fischer", "lena.fischer@example.org"],
  ["Ravi Menon", "ravi@example.com"],
  ["Ines Duarte", "ines@example.net"],
  ["Tomas Novak", "tomas@example.org"],
  ["Aiko Sato", "aiko@example.com"],
  ["Noah Becker", "noah.becker@example.net"],
  ["Sofia Rossi", "sofia@example.org"],
  ["Kwame Mensah", "kwame@example.com"],
] as const;

const LISTS = [
  ["The Weekly Ledger", "digest@news.example.com"],
  ["Trailhead Outfitters", "hello@shop.example.net"],
  ["Build Notes", "updates@builds.example.org"],
] as const;

const SUBJECTS = [
  "Draft agenda for Thursday",
  "Quick question about the contract",
  "Notes from the design review",
  "Lunch next week?",
  "Re: launch checklist",
  "Invoice 2026-114",
  "Photos from the offsite",
  "Can you review this by Friday?",
  "Travel plans for November",
  "Feedback on the proposal",
  "Re: hiring loop",
  "Your flight is confirmed",
  "Welcome aboard",
  "Budget for Q1",
  "Re: server migration window",
];

const SNIPPETS = [
  "Sharing the outline before the call so we can skip the warm-up and get straight to decisions.",
  "I went through the draft and left a few comments inline. Nothing blocking, mostly wording.",
  "Here is what we agreed on, plus the two open questions we still need to settle.",
  "Would Tuesday or Wednesday work better for you? I am flexible after two.",
  "Everything on the list is done except the last item, which waits on legal.",
  "Attached is the invoice for October. Payment terms are the usual thirty days.",
  "The album link is below. Grab whatever you like for the recap.",
  "It is short, maybe ten minutes. The second half is the part I am unsure about.",
];

const NEWS = [
  "This week: five short reads on quiet software and slow mornings.",
  "New arrivals for the cold season are in. Members get early access.",
  "Build 4812 passed. Two flaky tests were quarantined.",
];

export interface SeedThread {
  row: ThreadRow;
  messages: MessageMeta[];
  /** The demo failure: archiving this thread fails at the "provider" (§15 toasts). */
  failsOnArchive?: boolean;
}

export interface Seed {
  accounts: MailAccountView[];
  labels: LabelView[];
  splits: SplitView[];
  threads: SeedThread[];
}

const id = <T extends string>(s: string) => s as unknown as T;

const SYSTEM_ROLES = [
  "inbox",
  "sent",
  "drafts",
  "trash",
  "spam",
  "starred",
  "important",
  "unread",
  "snoozed",
] as const;

export function mockAccount(
  n: number,
  provider: "gmail" | "imap",
  email: string,
  now: number,
): MailAccountView {
  return {
    _id: id<AccountId>(`acc${n}`),
    _creationTime: now - 86_400_000 * 30,
    provider,
    providerAccountId: email,
    email,
    sortOrder: n,
    status: "live",
    syncPhase: "complete",
    progress: { threadsDone: 120 },
    windowStartAt: now - 86_400_000 * 90,
    backfillHorizonAt: now - 86_400_000 * 365,
    caps: {
      multiLabel: provider === "gmail",
      categories: provider !== "imap",
      serverSearch: true,
      deleteForever: provider !== "gmail",
    },
    createdAt: now - 86_400_000 * 30,
    lastSyncAt: now - 60_000,
  } as MailAccountView;
}

export function mockLabels(account: MailAccountView): LabelView[] {
  const a = account._id;
  const labels: LabelView[] = SYSTEM_ROLES.map((role) => ({
    _id: id(`${a}-${role}`),
    _creationTime: 0,
    accountId: a,
    providerLabelId: role.toUpperCase(),
    name: role[0]!.toUpperCase() + role.slice(1),
    kind: "system",
    role,
    exclusive:
      account.provider !== "gmail" && ["inbox", "sent", "drafts", "trash", "spam", "snoozed"].includes(role),
    hidden: false,
    pendingDelete: false,
    labelKey: `sys:${role}`,
  }));
  for (const [n, name] of ["Clients", "Receipts", "Travel"].entries()) {
    const lid = `${a}lbl${n}`;
    labels.push({
      _id: id(lid),
      _creationTime: 0,
      accountId: a,
      providerLabelId: `Label_${n}`,
      name,
      kind: account.provider === "gmail" ? "user" : "keyword",
      exclusive: false,
      hidden: false,
      pendingDelete: false,
      labelKey: `l:${lid}`,
    });
  }
  if (account.provider !== "gmail") {
    const lid = `${a}archive`;
    labels.push({
      _id: id(lid),
      _creationTime: 0,
      accountId: a,
      providerLabelId: "Archive",
      name: "Archive",
      kind: "folder",
      role: "archive",
      exclusive: true,
      hidden: false,
      pendingDelete: false,
      labelKey: `l:${lid}`,
    });
  }
  return labels;
}

export function mockSplits(): SplitView[] {
  return [
    {
      _id: id("splitvip"),
      _creationTime: 0,
      name: "VIP",
      order: 0,
      rule: { match: "all", conds: [{ f: "vip", eq: true }] },
      showCount: true,
      labelKey: "split:splitvip",
    },
    {
      _id: id("splitnews"),
      _creationTime: 0,
      name: "News",
      order: 1,
      rule: { match: "all", conds: [{ f: "is_bulk", eq: true }] },
      showCount: false,
      labelKey: "split:splitnews",
    },
  ] as SplitView[];
}

export const VIP_SENDERS = new Set(["mara@example.com", "jun@example.net"]);

export function splitFor(
  row: Pick<ThreadRow, "labelKeys" | "lastFrom" | "participants" | "isBulk" | "subject" | "accountId">,
  splits: readonly SplitView[],
): string | undefined {
  if (!row.labelKeys.includes(SYS.inbox)) return undefined;
  const from = row.lastFrom?.email ?? "";
  const facts: RuleFacts = {
    fromEmail: from,
    participants: row.participants.map((p) => p.email),
    labelKeys: row.labelKeys,
    isBulk: row.isBulk,
    subject: row.subject,
    accountId: row.accountId,
    vip: VIP_SENDERS.has(from),
  };
  const ordered = [...splits].sort((a, b) => a.order - b.order);
  return firstMatchingSplit(ordered, facts) ?? SPLIT_OTHER;
}

/** Builds the synthetic mailbox for the given accounts. */
export function buildSeed(accounts: MailAccountView[], now: number, seed = 7): Seed {
  const rand = prng(seed);
  const pick = <T>(xs: readonly T[]) => xs[Math.floor(rand() * xs.length)]!;
  const splits = mockSplits();
  const labels = accounts.flatMap(mockLabels);
  const threads: SeedThread[] = [];
  const me = (a: MailAccountView) => ({ name: "You", email: a.email });
  let n = 0;
  for (const account of accounts) {
    const count = account.provider === "gmail" ? 90 : 30;
    const userLabels = labels.filter(
      (l) => l.accountId === account._id && l.kind !== "system" && l.role !== "archive",
    );
    for (let i = 0; i < count; i++, n++) {
      const bulk = rand() < 0.22;
      const [name, email] = bulk ? pick(LISTS) : pick(PEOPLE);
      const age = 300_000 + Math.floor(i * 3.1 * 3_600_000 + rand() * 3_000_000);
      const at = now - age;
      const inbox = i < (account.provider === "gmail" ? 40 : 12) || rand() < 0.1;
      const keys = new Set<string>();
      if (inbox) keys.add(SYS.inbox);
      if (rand() < (inbox ? 0.45 : 0.08)) keys.add(SYS.unread);
      if (rand() < 0.12) keys.add(SYS.starred);
      if (!inbox && rand() < 0.15) keys.add(SYS.sent);
      if (!inbox && !keys.has(SYS.sent) && account.provider !== "gmail") {
        const archive = labels.find((l) => l.accountId === account._id && l.role === "archive");
        if (archive) keys.add(archive.labelKey);
      }
      if (rand() < 0.2 && userLabels.length > 0) keys.add(pick(userLabels).labelKey);
      const messageCount = bulk ? 1 : 1 + Math.floor(rand() * 4);
      const subject = bulk
        ? pick(["The Weekly Ledger, issue 212", "Early access starts today", "Build report"])
        : pick(SUBJECTS);
      const snippet = bulk ? pick(NEWS) : pick(SNIPPETS);
      const tid = id<ThreadId>(`thr${n}`);
      const from = { name, email };
      const messages: MessageMeta[] = [];
      for (let m = 0; m < messageCount; m++) {
        const outbound = m % 2 === 1;
        const mdate = at - (messageCount - 1 - m) * 5_400_000;
        messages.push({
          _id: id<MessageId>(`${tid}m${m}`),
          _creationTime: mdate,
          accountId: account._id,
          threadId: tid,
          providerMessageId: `${tid}-p${m}`,
          references: [],
          from: outbound ? me(account) : from,
          to: outbound ? [from] : [me(account)],
          cc: [],
          bcc: [],
          replyTo: [],
          subject: m === 0 ? subject : `Re: ${subject}`,
          snippet: m === messageCount - 1 ? snippet : pick(SNIPPETS),
          date: mdate,
          labelKeys: [...keys].filter((k) => k !== SYS.unread || m === messageCount - 1),
          isUnread: keys.has(SYS.unread) && m === messageCount - 1,
          isDraft: false,
          isOutbound: outbound,
          hasAttachments: m === 0 && rand() < 0.15,
          ...(bulk && { listId: `<${email.split("@")[1]}>` }),
        } as MessageMeta);
      }
      const row: ThreadRow = {
        _id: tid,
        accountId: account._id,
        subject,
        snippet,
        participants: [from, me(account)],
        lastFrom: messages[messages.length - 1]!.from,
        lastMessageAt: at,
        sortAt: at,
        messageCount,
        unreadCount: keys.has(SYS.unread) ? 1 : 0,
        draftCount: 0,
        hasAttachments: messages.some((m) => m.hasAttachments),
        isBulk: bulk,
        labelKeys: [...keys].sort(),
        pendingOpCount: 0,
      };
      row.split = splitFor(row, splits);
      threads.push({ row, messages });
    }
  }
  // The demo failure: a fixed, recognisable inbox thread at the top of the first account.
  const first = accounts[0];
  if (first) {
    const t = threads.find(
      (x) => x.row.accountId === first._id && x.row.labelKeys.includes(SYS.inbox) && !x.row.isBulk,
    );
    if (t) {
      t.row.subject = "Shared folder moved (archiving this fails in the demo)";
      t.row.sortAt = t.row.lastMessageAt = now - 60_000; // newest, so it sits at the top of the inbox
      t.failsOnArchive = true;
    }
  }
  return { accounts, labels, splits, threads };
}

const TINY_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGklEQVQ4T2NkYGD4z0ABYBzVMKoBRgNGNQAAqkgBEUqN0g4AAAAASUVORK5CYII=";

/** A sanitized body like the worker's (§7.3): remote images deferred, quote offset in UTF-8 bytes. */
export function mockBody(m: MessageMeta, opts: { remoteImages: boolean }): MessageBody {
  const greeting = `<p>Hi,</p><p>${escapeHtml(m.snippet)}</p>`;
  const image = opts.remoteImages
    ? `<p><img alt="Header image" width="16" height="16" data-akatsuki-src="${TINY_PNG}"></p>`
    : "";
  const sign = `<p>Best,<br>${escapeHtml(m.from?.name ?? m.from?.email ?? "")}</p>`;
  const visible = `${greeting}${image}${sign}`;
  const quoted = `<blockquote><p>Earlier message in this conversation.</p></blockquote>`;
  const html = `${visible}${quoted}`;
  return {
    messageId: m._id,
    providerMessageId: m.providerMessageId,
    sanitizerVersion: 1,
    html,
    text: `Hi,\n\n${m.snippet}\n\nBest,\n${m.from?.name ?? ""}`,
    quoted: { htmlStart: new TextEncoder().encode(visible).length, textStart: null },
    attachments: m.hasAttachments
      ? [{ partId: "2", filename: "notes.pdf", mimeType: "application/pdf", size: 48_213, inline: false }]
      : [],
    remoteImageCount: opts.remoteImages ? 1 : 0,
    trackersRemoved: opts.remoteImages ? 1 : 0,
    headers: {
      to: m.to,
      cc: m.cc,
      replyTo: m.replyTo,
      listUnsubscribe: m.listId ? { http: "https://news.example.com/unsubscribe", oneClick: true } : null,
      auth: null,
    },
  };
}

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}
