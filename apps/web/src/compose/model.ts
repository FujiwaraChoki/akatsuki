// The composer's local draft (the device copy of §10.1) and how it is built for new mail, replies and forwards.
// Pure: no React, no network. The provider draft is written from `toDraftContent` by the autosaver.
import type { DraftContent } from "@akatsuki/wire";
import type {
  AccountId,
  IdentityView,
  MailAccountView,
  MessageId,
  MessageMeta,
  ThreadDetail,
} from "../backend/types.ts";
import { fullDate } from "../lib/format.ts";
import { uuidv7 } from "../lib/ids.ts";
import { type Addr, forwardSubject, quoteLines, replySubject } from "./text.ts";

export type ComposeMode = "new" | "reply" | "reply_all" | "forward";

export type FollowUp = "1d" | "3d" | "1w";
export const FOLLOW_UP_MS: Record<FollowUp, number> = {
  "1d": 86_400_000,
  "3d": 3 * 86_400_000,
  "1w": 7 * 86_400_000,
};

export interface ComposeAttachment {
  /** Local key (stable across renders). */
  key: string;
  filename: string;
  size: number;
  mimeType: string;
  status: "uploading" | "ready" | "error";
  uploadId?: string;
  /** A part of an existing message (forwarding keeps the original attachments). */
  fromMessageId?: MessageId;
  partId?: string;
  error?: string;
}

export interface LocalDraft {
  clientDraftId: string;
  mode: ComposeMode;
  accountId: AccountId | null;
  identityEmail: string | null;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  showCc: boolean;
  showBcc: boolean;
  subject: string;
  /** What the user typed (plain text, D-280). */
  body: string;
  /** Appended under the body when saved and sent: the reply quote or the forwarded message. */
  quoted: string | null;
  /** The message `quoted` is waiting for (filled once its body is on the device). */
  quoteOf?: MessageId;
  threadId?: string;
  inReplyToMessageId?: MessageId;
  forwardOfMessageId?: MessageId;
  attachments: ComposeAttachment[];
  /** Read receipt for this message (only offered when prefs.readReceipts.enabled, decision 8). */
  readReceipt: boolean;
  /** "Remind me if no reply" (§10 reminders, `if_no_reply`), counted from the moment the message is sent. */
  followUp: FollowUp | null;
  /** The provider draft version (`baseVersion`); 0 = never saved. */
  version: number;
  createdAt: number;
  updatedAt: number;
  savedAt: number | null;
}

/** §1.6: 25 MB of attachments per message. */
export const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024;

export function emptyDraft(over: Partial<LocalDraft> = {}, now = Date.now()): LocalDraft {
  return {
    clientDraftId: uuidv7(now),
    mode: "new",
    accountId: null,
    identityEmail: null,
    to: [],
    cc: [],
    bcc: [],
    showCc: false,
    showBcc: false,
    subject: "",
    body: "",
    quoted: null,
    attachments: [],
    readReceipt: false,
    followUp: null,
    version: 0,
    createdAt: now,
    updatedAt: now,
    savedAt: null,
    ...over,
  };
}

/** The identity to send from: one the message was addressed to, else the account's default, else its address. */
export function pickIdentity(
  account: Pick<MailAccountView, "_id" | "email"> | undefined,
  identities: readonly IdentityView[],
  addressedTo: readonly Addr[] = [],
): string | null {
  if (!account) return null;
  const mine = identities.filter((i) => i.accountId === account._id);
  const hit = mine.find((i) => addressedTo.some((a) => a.email === i.email.toLowerCase()));
  return (hit ?? mine.find((i) => i.isDefault) ?? mine[0])?.email ?? account.email;
}

/** The newest message that is not a draft (what a reply answers). */
export function replyTarget(messages: readonly MessageMeta[]): MessageMeta | undefined {
  for (let i = messages.length - 1; i >= 0; i--) if (!messages[i]!.isDraft) return messages[i];
  return messages[messages.length - 1];
}

const dedupe = (list: readonly Addr[], drop: ReadonlySet<string>): Addr[] => {
  const seen = new Set<string>();
  const out: Addr[] = [];
  for (const a of list) {
    const e = a.email.toLowerCase();
    if (drop.has(e) || seen.has(e)) continue;
    seen.add(e);
    out.push(a.name ? { name: a.name, email: e } : { email: e });
  }
  return out;
};

/**
 * Reply recipients. Reply: the Reply-To (else the sender); when the newest message is our own, its original
 * recipients. Reply all adds every other To/Cc address, never our own addresses.
 */
export function replyRecipients(
  m: MessageMeta,
  mode: "reply" | "reply_all",
  myEmails: ReadonlySet<string>,
): { to: Addr[]; cc: Addr[] } {
  const own = m.isOutbound || (m.from && myEmails.has(m.from.email.toLowerCase()));
  const primary: Addr[] = own ? m.to : m.replyTo.length > 0 ? m.replyTo : m.from ? [m.from] : [];
  const to = dedupe(primary, own ? new Set() : myEmails);
  if (mode === "reply") return { to: to.length ? to : dedupe(primary, new Set()), cc: [] };
  const toSet = new Set(to.map((a) => a.email));
  const others = own ? m.cc : [...m.to, ...m.cc];
  const cc = dedupe(others, new Set([...myEmails, ...toSet]));
  return { to, cc };
}

export function quoteHeader(m: MessageMeta, tz: string): string {
  const who = m.from ? (m.from.name ? `${m.from.name} <${m.from.email}>` : m.from.email) : "someone";
  return `On ${fullDate(m.date, tz)}, ${who} wrote:`;
}

/** The plain-text quote of a reply. */
export function replyQuote(m: MessageMeta, bodyText: string, tz: string): string {
  return `${quoteHeader(m, tz)}\n${quoteLines(bodyText)}`;
}

/** The forwarded-message block (headers, then the original text). */
export function forwardBlock(m: MessageMeta, bodyText: string, tz: string): string {
  const fmt = (list: readonly Addr[]) =>
    list.map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(", ");
  const lines = [
    "---------- Forwarded message ----------",
    `From: ${m.from ? fmt([m.from]) : ""}`,
    `Date: ${fullDate(m.date, tz)}`,
    `Subject: ${m.subject}`,
    `To: ${fmt(m.to)}`,
  ];
  if (m.cc.length) lines.push(`Cc: ${fmt(m.cc)}`);
  return `${lines.join("\n")}\n\n${bodyText.trim()}`;
}

export interface BuildContext {
  accounts: readonly MailAccountView[];
  identities: readonly IdentityView[];
  myEmails: ReadonlySet<string>;
  now?: number;
}

/** A reply, reply-all or forward of `detail`. `quoted` stays null until the quoted body is on the device. */
export function draftFor(
  detail: ThreadDetail,
  mode: Exclude<ComposeMode, "new">,
  ctx: BuildContext,
): LocalDraft {
  const m = replyTarget(detail.messages);
  const account = ctx.accounts.find((a) => a._id === detail.account._id);
  const base = emptyDraft({ mode, accountId: detail.account._id, threadId: detail.thread._id }, ctx.now);
  if (!m) return { ...base, subject: replySubject(detail.thread.subject) };
  const identityEmail = pickIdentity(account ?? detail.account, ctx.identities, [...m.to, ...m.cc]);
  if (mode === "forward")
    return {
      ...base,
      identityEmail,
      subject: forwardSubject(m.subject || detail.thread.subject),
      forwardOfMessageId: m._id,
      quoteOf: m._id,
    };
  const { to, cc } = replyRecipients(m, mode, ctx.myEmails);
  return {
    ...base,
    identityEmail,
    to,
    cc,
    showCc: cc.length > 0,
    subject: replySubject(m.subject || detail.thread.subject),
    inReplyToMessageId: m._id,
    quoteOf: m._id,
  };
}

export function hasContent(d: LocalDraft): boolean {
  return (
    d.to.length + d.cc.length + d.bcc.length > 0 ||
    d.subject.trim() !== "" ||
    d.body.trim() !== "" ||
    d.attachments.length > 0
  );
}

export function attachmentBytes(d: Pick<LocalDraft, "attachments">): number {
  return d.attachments.reduce((n, a) => n + a.size, 0);
}

/** The full text that is saved and sent: the body, then the quote. */
export function fullText(d: Pick<LocalDraft, "body" | "quoted">): string {
  const body = d.body.replace(/\s+$/, "");
  return d.quoted ? `${body}\n\n${d.quoted}` : body;
}

/** `DraftContent` for `PUT /v1/drafts` and `POST /v1/send` (§7.3). Null until an account is chosen. */
export function toDraftContent(d: LocalDraft, html: (text: string) => string): DraftContent | null {
  if (!d.accountId || !d.identityEmail) return null;
  const text = fullText(d);
  const attachments: DraftContent["attachments"] = [];
  for (const a of d.attachments) {
    if (a.status !== "ready") continue;
    if (a.uploadId) attachments.push({ uploadId: a.uploadId });
    else if (a.fromMessageId && a.partId)
      attachments.push({ fromMessageId: a.fromMessageId, partId: a.partId });
  }
  return {
    accountId: d.accountId,
    identityEmail: d.identityEmail,
    to: d.to,
    cc: d.cc,
    bcc: d.bcc,
    subject: d.subject,
    html: text.trim() ? html(text) : null,
    text,
    ...(d.inReplyToMessageId && { inReplyToMessageId: d.inReplyToMessageId }),
    ...(d.forwardOfMessageId && { forwardOfMessageId: d.forwardOfMessageId }),
    attachments,
    ...(d.readReceipt && { readReceipt: true }),
  };
}

/** Why a draft cannot be sent yet, or null. */
export function sendProblem(d: LocalDraft): string | null {
  if (!d.accountId || !d.identityEmail) return "Choose an account to send from.";
  if (d.to.length + d.cc.length + d.bcc.length === 0) return "Add at least one recipient.";
  if (d.attachments.some((a) => a.status === "uploading"))
    return "Wait for the attachments to finish uploading.";
  if (d.attachments.some((a) => a.status === "error")) return "Remove the attachments that failed to upload.";
  if (attachmentBytes(d) > MAX_ATTACHMENT_BYTES) return "Attachments are limited to 25 MB per message.";
  return null;
}

/** A new message from an account (its default identity). */
export function newDraft(
  ctx: BuildContext,
  over: { accountId?: AccountId | null; to?: Addr[]; subject?: string; body?: string } = {},
): LocalDraft {
  const account = (over.accountId && ctx.accounts.find((a) => a._id === over.accountId)) || ctx.accounts[0];
  return emptyDraft(
    {
      accountId: account?._id ?? null,
      identityEmail: pickIdentity(account, ctx.identities),
      to: over.to ?? [],
      subject: over.subject ?? "",
      body: over.body ?? "",
    },
    ctx.now,
  );
}
