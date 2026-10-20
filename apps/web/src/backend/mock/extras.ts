// Part-2 state of the mock backend: identities, drafts and sends (with the undo window), snippets, rules,
// devices and sessions, AI settings, billing, and a canned apps/api (AI streams, transcription, checkout).
// Same semantics as the contract (§4.2, §7.2, §8, §10): CAS on draft versions, a `cancelled` tombstone when
// Undo beats `/v1/send`, `too_late` once a send is due. Every value is invented.
import { SYS, type ThreadAction } from "@akatsuki/mail-model";
import type {
  AaspEvent,
  AdoptResponse,
  DraftContent,
  DraftPut,
  DraftSaved,
  SendAccepted,
  SendRequest,
  StreamedAiFeature,
} from "@akatsuki/wire";
import type {
  AccountId,
  AiSettingsPatch,
  AiSettingsView,
  ApiService,
  BillingInfo,
  DeviceView,
  DraftMeta,
  IdentityView,
  MailAccountView,
  MessageId,
  MessageMeta,
  Rule,
  RuleView,
  ScheduledSendView,
  SessionInfo,
  SnippetView,
  ThreadId,
} from "../types.ts";
import type { MockStore } from "./store.ts";

export type MockPlan = "trialing" | "active" | "none" | "selfhost";

const DAY = 86_400_000;
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, extra: Record<string, unknown> = {}) =>
  Object.assign(new Error(code), { code, data: { code, ...extra }, ...extra });

export function billingFor(plan: MockPlan, now: number): BillingInfo {
  const usage = {
    period: new Date(now).toISOString().slice(0, 7),
    usedMicros: 1_240_000,
    limitMicros: 8_000_000,
  };
  switch (plan) {
    case "trialing":
      return {
        entitlement: { state: "trialing", hostedSync: true, managedAi: true, until: now + 12 * DAY },
        billing: {
          status: "trialing",
          trialEnd: now + 12 * DAY,
          cancelAtPeriodEnd: false,
          trialUsedAt: now - 2 * DAY,
        },
        aiUsage: { ...usage, limitMicros: 1_000_000, usedMicros: 210_000 },
      };
    case "active":
      return {
        entitlement: { state: "active", hostedSync: true, managedAi: true, until: now + 33 * DAY },
        billing: {
          status: "active",
          currentPeriodEnd: now + 30 * DAY,
          cancelAtPeriodEnd: false,
          trialUsedAt: now - 60 * DAY,
        },
        aiUsage: usage,
      };
    case "none":
      return {
        entitlement: { state: "none", hostedSync: false, managedAi: false, until: 0 },
        billing: { status: "none", cancelAtPeriodEnd: false },
        aiUsage: { ...usage, usedMicros: 0, limitMicros: 0 },
      };
    case "selfhost":
      return {
        entitlement: {
          state: "selfhost",
          hostedSync: true,
          managedAi: false,
          until: Number.MAX_SAFE_INTEGER,
        },
        billing: { status: "none", cancelAtPeriodEnd: false },
        aiUsage: { ...usage, usedMicros: 0, limitMicros: 0 },
      };
  }
}

interface StoredDraft {
  meta: DraftMeta;
  content: DraftContent;
}

export class MockExtras {
  identities: IdentityView[] = [];
  snippets: SnippetView[] = [];
  rules: RuleView[] = [];
  devices: DeviceView[] = [];
  sessions: SessionInfo[] = [];
  aiSettings: AiSettingsView | null = null;
  billing: BillingInfo;
  plan: MockPlan;
  blocked = new Set<string>();
  drafts = new Map<string, StoredDraft>();
  sends: ScheduledSendView[] = [];
  private sendTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private seq = 0;

  constructor(
    private store: MockStore,
    plan: MockPlan,
  ) {
    this.plan = plan;
    this.billing = billingFor(plan, store.now());
    this.snippets = [
      {
        _id: "snp1" as never,
        _creationTime: 0,
        name: "Thanks, will review",
        shortcut: "thx",
        bodyHtml: "<p>Thanks {{first_name}}, I will review this and get back to you by {{day}}.</p>",
        variables: ["first_name", "day"],
        useCount: 3,
        updatedAt: 0,
      },
      {
        _id: "snp2" as never,
        _creationTime: 0,
        name: "Schedule a call",
        shortcut: "call",
        subject: "Quick call?",
        bodyHtml: "<p>Hi {{first_name}},</p><p>Do you have 20 minutes this week for a call?</p>",
        variables: ["first_name"],
        useCount: 1,
        updatedAt: 0,
      },
    ];
    const now = store.now();
    this.sessions = [
      {
        id: "ses1",
        token: "tok-current",
        userAgent: "This browser",
        createdAt: now - 3 * DAY,
        expiresAt: now + 27 * DAY,
        current: true,
      },
      {
        id: "ses2",
        token: "tok-desktop",
        userAgent: "Akatsuki desktop on Linux",
        createdAt: now - 9 * DAY,
        expiresAt: now + 21 * DAY,
        current: false,
      },
    ];
  }

  private emit() {
    this.store.emit();
  }

  /** Rebuilds per-account rows after accounts change (connect, sign-out). */
  syncAccounts(accounts: readonly MailAccountView[]) {
    this.identities = accounts.map((a, i) => {
      const existing = this.identities.find((x) => x.accountId === a._id);
      return (
        existing ?? {
          _id: `idn${i + 1}` as never,
          _creationTime: 0,
          accountId: a._id,
          email: a.email,
          name: "You",
          signatureHtml: "<p>You<br>Sent with Akatsuki</p>",
          isDefault: true,
        }
      );
    });
  }

  setPlan(plan: MockPlan) {
    this.plan = plan;
    this.store.session.plan = plan;
    this.store.save();
    this.billing = billingFor(plan, this.store.now());
    this.emit();
  }

  // ───────────── drafts and sends (§10.1–10.3) ─────────────

  putDraft(clientDraftId: string, put: DraftPut): DraftSaved {
    const cur = this.drafts.get(clientDraftId);
    const version = cur?.meta.version ?? 0;
    if (put.baseVersion !== version) throw err("draft_version_conflict", { version });
    if (cur && cur.meta.state !== "editing" && cur.meta.state !== "discarded") throw err("invalid_argument");
    const now = this.store.now();
    const rfcMessageId =
      cur?.meta.rfcMessageId ?? `${clientDraftId}@${put.draft.identityEmail.split("@")[1] ?? "example.com"}`;
    const replyTo = put.draft.inReplyToMessageId ? this.messageById(put.draft.inReplyToMessageId) : null;
    const meta: DraftMeta = {
      _id: (cur?.meta._id ?? `drf${++this.seq}`) as never,
      _creationTime: cur?.meta._creationTime ?? now,
      accountId: put.draft.accountId,
      clientDraftId,
      ...(replyTo && { threadId: replyTo.threadId, replyToMessageId: replyTo._id }),
      providerDraftId: cur?.meta.providerDraftId ?? `pd-${clientDraftId.slice(0, 8)}`,
      rfcMessageId,
      version: version + 1,
      subject: put.draft.subject,
      to: put.draft.to,
      cc: put.draft.cc,
      bcc: put.draft.bcc,
      state: "editing",
      updatedByDeviceId: put.deviceId,
      updatedAt: now,
    };
    this.drafts.set(clientDraftId, { meta, content: put.draft });
    this.emit();
    return { draftId: meta._id, version: meta.version, providerDraftId: meta.providerDraftId!, rfcMessageId };
  }

  adoptDraft(clientDraftId: string, messageId: MessageId): AdoptResponse {
    const m = this.messageById(messageId);
    if (!m) throw err("not_found");
    const content: DraftContent = {
      accountId: m.accountId,
      identityEmail: this.store.account(m.accountId)?.email ?? "you@example.com",
      to: m.to,
      cc: m.cc,
      bcc: m.bcc,
      subject: m.subject,
      html: null,
      text: m.snippet,
      attachments: [],
    };
    const existing = this.drafts.get(clientDraftId);
    const version = existing?.meta.version ?? 1;
    if (!existing) {
      this.drafts.set(clientDraftId, {
        meta: {
          _id: `drf${++this.seq}` as never,
          _creationTime: this.store.now(),
          accountId: m.accountId,
          clientDraftId,
          threadId: m.threadId,
          rfcMessageId: `${clientDraftId}@example.com`,
          version,
          subject: m.subject,
          to: m.to,
          cc: m.cc,
          bcc: m.bcc,
          state: "editing",
          updatedAt: this.store.now(),
        },
        content,
      });
    }
    return { draft: content, version };
  }

  discardDraft(clientDraftId: string) {
    const d = this.drafts.get(clientDraftId);
    if (d) d.meta = { ...d.meta, state: "discarded" };
    this.emit();
    return { opId: `op-discard-${clientDraftId}` };
  }

  send(req: SendRequest): SendAccepted {
    const now = this.store.now();
    const existing = this.sends.find((s) => s.clientSendId === req.clientSendId);
    if (existing?.state === "cancelled" && !existing.draftId)
      return { sendId: existing._id, sendAt: 0, state: "cancelled" };
    if (existing) return { sendId: existing._id, sendAt: existing.sendAt, state: existing.state as never };
    if (req.draft)
      this.putDraft(req.clientDraftId, {
        baseVersion: req.baseVersion,
        draft: req.draft,
        deviceId: req.deviceId,
      });
    const d = this.drafts.get(req.clientDraftId);
    if (!d) throw err("not_found");
    const kind = req.sendAt ? "later" : "undo";
    const sendAt = req.sendAt ?? now + req.undoSeconds * 1000;
    const due = kind === "undo" && req.undoSeconds === 0;
    const row: ScheduledSendView = {
      _id: `snd${++this.seq}` as never,
      _creationTime: now,
      accountId: d.meta.accountId,
      clientSendId: req.clientSendId,
      draftId: d.meta._id,
      kind,
      sendAt,
      state: due ? "due" : "scheduled",
      preview: { subject: d.meta.subject, to: d.meta.to },
      createdAt: now,
    };
    this.sends = [...this.sends, row];
    d.meta = { ...d.meta, state: due ? "sending" : "scheduled" };
    this.schedule(row, due ? 0 : Math.max(0, sendAt - now));
    this.emit();
    return { sendId: row._id, sendAt, state: row.state as "scheduled" | "due" };
  }

  private schedule(row: ScheduledSendView, ms: number) {
    const prev = this.sendTimers.get(row.clientSendId);
    if (prev) clearTimeout(prev);
    this.sendTimers.set(
      row.clientSendId,
      setTimeout(() => this.deliver(row.clientSendId), ms),
    );
  }

  cancelSend(clientSendId: string): { result: "cancelled" | "too_late" } {
    const row = this.sends.find((s) => s.clientSendId === clientSendId);
    if (!row) {
      // Undo pressed before /v1/send finished: a tombstone that send() will see (§10.3).
      this.sends = [
        ...this.sends,
        {
          _id: `snd${++this.seq}` as never,
          _creationTime: this.store.now(),
          clientSendId,
          kind: "undo",
          sendAt: 0,
          state: "cancelled",
          preview: { subject: "", to: [] },
          createdAt: this.store.now(),
        },
      ];
      this.emit();
      return { result: "cancelled" };
    }
    if (row.state !== "scheduled") return { result: row.state === "cancelled" ? "cancelled" : "too_late" };
    const t = this.sendTimers.get(clientSendId);
    if (t) clearTimeout(t);
    this.patchSend(clientSendId, { state: "cancelled" });
    const d = [...this.drafts.values()].find((x) => x.meta._id === row.draftId);
    if (d) d.meta = { ...d.meta, state: "editing" };
    this.emit();
    return { result: "cancelled" };
  }

  rescheduleSend(clientSendId: string, sendAt: number) {
    const row = this.sends.find((s) => s.clientSendId === clientSendId);
    if (row?.state !== "scheduled") throw err("invalid_argument");
    this.patchSend(clientSendId, { sendAt });
    this.schedule({ ...row!, sendAt }, Math.max(0, sendAt - this.store.now()));
    this.emit();
  }

  resolveUnknown(clientSendId: string, resolution: "was_sent" | "discard") {
    this.patchSend(clientSendId, { state: resolution === "was_sent" ? "sent" : "cancelled" });
    this.emit();
  }

  private patchSend(clientSendId: string, patch: Partial<ScheduledSendView>) {
    this.sends = this.sends.map((s) => (s.clientSendId === clientSendId ? { ...s, ...patch } : s));
  }

  /** The send leaves: the draft becomes a sent message in its thread (or a new thread). */
  private deliver(clientSendId: string) {
    const row = this.sends.find((s) => s.clientSendId === clientSendId);
    if (!row || (row.state !== "scheduled" && row.state !== "due")) return;
    const d = [...this.drafts.values()].find((x) => x.meta._id === row.draftId);
    if (!d)
      return this.patchSend(clientSendId, {
        state: "failed",
        error: { code: "draft_deleted", message: "The draft was deleted.", retryable: false },
      });
    const now = this.store.now();
    const account = this.store.account(d.meta.accountId);
    const from = { name: "You", email: d.content.identityEmail || account?.email || "you@example.com" };
    const reply = d.content.inReplyToMessageId ? this.messageById(d.content.inReplyToMessageId) : null;
    let thread = reply ? this.store.threads.get(reply.threadId) : undefined;
    if (!thread) {
      const tid = `thrs${++this.seq}` as ThreadId;
      thread = {
        row: {
          _id: tid,
          accountId: d.meta.accountId,
          subject: d.meta.subject,
          snippet: "",
          participants: [...d.meta.to, from],
          lastMessageAt: now,
          sortAt: now,
          messageCount: 0,
          unreadCount: 0,
          draftCount: 0,
          hasAttachments: false,
          isBulk: false,
          labelKeys: [],
          pendingOpCount: 0,
        },
        messages: [],
      };
      this.store.threads.set(tid, thread);
    }
    const t = thread;
    const msg: MessageMeta = {
      _id: `${t.row._id}m${t.messages.length}s` as MessageId,
      _creationTime: now,
      accountId: t.row.accountId,
      threadId: t.row._id,
      providerMessageId: `sent-${clientSendId}`,
      rfcMessageId: d.meta.rfcMessageId,
      references: [],
      from,
      to: d.content.to,
      cc: d.content.cc,
      bcc: d.content.bcc,
      replyTo: [],
      subject: d.content.subject,
      snippet: d.content.text.replace(/\s+/g, " ").trim().slice(0, 200),
      date: now,
      labelKeys: [SYS.sent],
      isUnread: false,
      isDraft: false,
      isOutbound: true,
      hasAttachments: d.content.attachments.length > 0,
    } as MessageMeta;
    t.messages = [...t.messages, msg];
    const keys = new Set([...t.row.labelKeys, SYS.sent]);
    t.row = {
      ...t.row,
      snippet: msg.snippet,
      lastFrom: from,
      lastMessageAt: now,
      sortAt: Math.max(t.row.sortAt, now),
      messageCount: t.messages.length,
      labelKeys: [...keys].sort(),
      hasAttachments: t.row.hasAttachments || msg.hasAttachments,
    };
    d.meta = { ...d.meta, state: "sent", threadId: t.row._id };
    this.patchSend(clientSendId, { state: "sent", sentProviderMessageId: msg.providerMessageId });
    this.emit();
  }

  private messageById(id: string): MessageMeta | null {
    for (const t of this.store.threads.values()) {
      const m = t.messages.find((x) => x._id === id);
      if (m) return m;
    }
    return null;
  }

  // ───────────── settings rows ─────────────

  saveSnippet(
    a: Omit<SnippetView, "_id" | "_creationTime" | "useCount" | "updatedAt"> & { snippetId?: string },
  ) {
    const now = this.store.now();
    const { snippetId, ...fields } = a;
    if (snippetId)
      this.snippets = this.snippets.map((s) =>
        s._id === snippetId ? { ...s, ...fields, updatedAt: now } : s,
      );
    else
      this.snippets = [
        ...this.snippets,
        { _id: `snp${++this.seq}` as never, _creationTime: now, useCount: 0, updatedAt: now, ...fields },
      ];
    this.emit();
    return { snippetId: (snippetId ?? this.snippets[this.snippets.length - 1]!._id) as never };
  }

  saveRule(a: {
    ruleId?: string;
    accountId?: AccountId;
    name: string;
    enabled: boolean;
    order: number;
    rule: Rule;
    actions: ThreadAction[];
  }) {
    const { ruleId, ...fields } = a;
    if (ruleId)
      this.rules = this.rules.map((r) => (r._id === ruleId ? ({ ...r, ...fields } as RuleView) : r));
    else
      this.rules = [
        ...this.rules,
        { _id: `rul${++this.seq}` as never, _creationTime: this.store.now(), ...fields } as RuleView,
      ];
    this.emit();
    return { ruleId: (ruleId ?? this.rules[this.rules.length - 1]!._id) as never };
  }

  saveAi(patch: AiSettingsPatch) {
    const cur: AiSettingsView = this.aiSettings ?? {
      _id: "ais1" as never,
      _creationTime: 0,
      profiles: [],
      routes: [],
      consent: { autoTriage: false, syncVoiceProfile: false },
      updatedAt: 0,
    };
    const { voiceProfile, ...rest } = patch;
    const next: AiSettingsView = { ...cur, ...rest, updatedAt: this.store.now() };
    if (voiceProfile === null) delete next.voiceProfile;
    else if (voiceProfile !== undefined) {
      if (!next.consent.syncVoiceProfile) throw err("invalid_argument");
      next.voiceProfile = voiceProfile;
    }
    for (const p of next.profiles)
      if (p.baseUrl && (/[?#]/.test(p.baseUrl) || /\/\/[^/]*@/.test(p.baseUrl)))
        throw err("invalid_argument");
    this.aiSettings = next;
    this.emit();
  }

  // ───────────── apps/api (canned) ─────────────

  api(): ApiService {
    return {
      configured: true,
      streamFeature: (feature, body, signal) => this.stream(feature, body.vars, signal),
      transcribe: async () => {
        await delay(120);
        return {
          text: "Thanks for sending this over. I will take a look tomorrow morning.",
          durationSec: 4,
          language: "en",
        };
      },
      quota: async () => ({
        period: this.billing.aiUsage.period,
        usedMicros: this.billing.aiUsage.usedMicros,
        limitMicros: this.billing.aiUsage.limitMicros,
        resetsAt: Date.UTC(new Date().getUTCFullYear(), new Date().getUTCMonth() + 1, 1),
      }),
      checkout: async () => {
        await delay(80);
        if (this.plan === "trialing" || this.plan === "active") throw err("already_subscribed");
        this.setPlan("trialing");
        if (this.store.session.onboarding === "checkout") this.store.setOnboarding("connect");
        return { url: `${window.location.origin}/billing/done?session_id=cs_mock` };
      },
      portal: async () => ({ url: `${window.location.origin}/settings/billing` }),
      cancel: async () => {
        this.setPlan("none");
      },
    };
  }

  private async *stream(
    feature: StreamedAiFeature,
    vars: Record<string, unknown>,
    signal?: AbortSignal,
  ): AsyncGenerator<AaspEvent> {
    const model =
      feature === "triage" || feature === "nl_search"
        ? "anthropic/claude-haiku-5.5"
        : "anthropic/claude-sonnet-5.5";
    if (!this.billing.entitlement.managedAi) throw err("not_entitled");
    yield { event: "meta", data: { feature, model, promptVersion: 1 } };
    const usage = { inputTokens: 420, outputTokens: 80 };
    const text = (s: string) => s;
    const textOut = cannedText(feature, vars);
    if (textOut !== null) {
      const words = textOut.split(/(?<= )/);
      for (const w of words) {
        if (signal?.aborted) return;
        await delay(8);
        yield { event: "delta", data: { text: text(w) } };
      }
      yield { event: "done", data: { text: textOut, object: null, usage, finishReason: "stop" } };
      return;
    }
    const obj = cannedObject(feature, vars);
    await delay(30);
    yield { event: "partial", data: { object: obj } };
    yield { event: "done", data: { text: null, object: obj, usage, finishReason: "stop" } };
  }
}

function cannedText(feature: StreamedAiFeature, vars: Record<string, unknown>): string | null {
  if (feature === "compose") {
    const instruction = String(vars.instruction ?? "").toLowerCase();
    if (instruction.includes("decline"))
      return "Thanks for thinking of me. I can't take this on right now, but I'd be glad to help next quarter.";
    return "Thanks for the update. That works for me; let's go ahead as planned and check in on Friday.";
  }
  if (feature === "rewrite") {
    const t = String(vars.text ?? "");
    switch (vars.style) {
      case "shorter":
        return t.split(/(?<=[.!?])\s+/)[0] ?? t;
      case "formal":
        return `Dear colleague,\n\n${t}\n\nKind regards`;
      case "friendlier":
        return `${t} Hope you're having a good week!`;
      default:
        return t.replace(/\bi\b/g, "I").replace(/\s+([,.])/g, "$1");
    }
  }
  return null;
}

function cannedObject(feature: StreamedAiFeature, vars: Record<string, unknown>): unknown {
  switch (feature) {
    case "summarize":
      return {
        tldr: "They want your review of the attached notes before Friday's meeting.",
        bullets: [
          "The draft agenda is ready.",
          "Two open questions about the budget.",
          "Meeting moved to 10:00.",
        ],
        asks: ["Review the notes and reply with comments."],
        deadlines: [{ text: "Comments before the meeting", due: "Friday" }],
      };
    case "instant_reply":
      return {
        replies: [
          { label: "Sounds good", body: "Sounds good, thanks. I'll take a look before Friday." },
          {
            label: "Need more time",
            body: "Thanks. Could we push this to early next week? I need a bit more time.",
          },
          { label: "Call?", body: "Happy to discuss. Do you have 15 minutes tomorrow for a quick call?" },
        ],
      };
    case "extract_actions":
      return {
        actions: [
          { kind: "todo", title: "Review the meeting notes", due: null, quote: "could you review the notes" },
          { kind: "deadline", title: "Send comments", due: "Friday", quote: "before Friday" },
        ],
      };
    case "nl_search": {
      const q = String(vars.query ?? "").toLowerCase();
      const parts: string[] = [];
      if (q.includes("unread")) parts.push("is:unread");
      if (q.includes("attachment") || q.includes("pdf")) parts.push("has:attachment");
      const from = /from (\w+)/.exec(q);
      if (from) parts.push(`from:${from[1]}`);
      if (q.includes("last week")) parts.push("newer_than:7d");
      return {
        query: parts.join(" ") || q,
        explanation: "Turned your sentence into search operators. Edit them before running.",
      };
    }
    case "triage":
      return { items: [] };
    case "voice_profile":
      return { profile: "Short, warm sentences. Greets by first name. Signs off with 'Best'." };
  }
  return null;
}
