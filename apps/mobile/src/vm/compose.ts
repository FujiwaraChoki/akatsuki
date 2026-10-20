// Compose view-model (ARCHITECTURE §7.3 `DraftContent`/`SendRequest`, §10.1 autosave, §10.2 send, §10.3 undo).
// Text is composed as plain text; the worker builds the MIME message, adds the identity's signature and
// re-sanitizes (crates/mime::compose is the only MIME builder). Draft text never goes to Convex.
import { formatAddress } from "./format.ts";

export interface Addr {
  name?: string;
  email: string;
}

const EMAIL_RE = /^[^\s@<>(),;:"[\]]+@[^\s@<>(),;:"[\]]+\.[^\s@<>(),;:"[\]]+$/;

export function isEmail(s: string): boolean {
  return EMAIL_RE.test(s) && s.length <= 320;
}

/** Splits a recipient field on commas, semicolons and newlines, keeping commas inside quoted names. */
function splitRecipients(input: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  let angle = false;
  for (const ch of input) {
    if (ch === '"') quoted = !quoted;
    if (!quoted && ch === "<") angle = true;
    if (!quoted && ch === ">") angle = false;
    if (!quoted && !angle && (ch === "," || ch === ";" || ch === "\n")) {
      out.push(cur);
      cur = "";
    } else cur += ch;
  }
  out.push(cur);
  return out.map((s) => s.trim()).filter(Boolean);
}

/** Parses one recipient: `Name <a@b.c>`, `"Last, First" <a@b.c>` or `a@b.c`. */
export function parseAddress(raw: string): Addr | null {
  const s = raw.trim();
  const m = /^(.*)<([^<>]+)>$/.exec(s);
  if (m) {
    const email = m[2]!.trim().toLowerCase();
    if (!isEmail(email)) return null;
    const name = m[1]!
      .trim()
      .replace(/^"(.*)"$/, "$1")
      .trim();
    return name ? { name, email } : { email };
  }
  const email = s.replace(/^mailto:/i, "").toLowerCase();
  return isEmail(email) ? { email } : null;
}

/** Parses a recipient field into addresses and the parts that are not addresses. */
export function parseAddressList(input: string): { valid: Addr[]; invalid: string[] } {
  const valid: Addr[] = [];
  const invalid: string[] = [];
  for (const part of splitRecipients(input)) {
    const a = parseAddress(part);
    if (a) valid.push(a);
    else invalid.push(part);
  }
  return { valid: dedupe(valid), invalid };
}

export function dedupe(list: readonly Addr[], exclude: ReadonlySet<string> = new Set()): Addr[] {
  const seen = new Set(exclude);
  const out: Addr[] = [];
  for (const a of list) {
    const key = a.email.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
  }
  return out;
}

/** `Re:`/`Fwd:` without stacking prefixes. */
export function prefixedSubject(kind: "reply" | "forward", subject: string): string {
  const s = subject.trim();
  if (kind === "reply") return /^(re|aw|sv)\s*:/i.test(s) ? s : `Re: ${s}`;
  return /^(fwd?|wg)\s*:/i.test(s) ? s : `Fwd: ${s}`;
}

/** The message metadata a reply is built from (`MessageMeta`, §4.4). */
export interface MessageMetaLike {
  _id: string;
  accountId: string;
  from?: Addr;
  to: Addr[];
  cc: Addr[];
  replyTo: Addr[];
  subject: string;
  date: number;
  isOutbound: boolean;
  isDraft: boolean;
}

export type ComposeKind = "new" | "reply" | "replyAll" | "forward";

/** §1.6: 25 MB of attachments per message (the worker refuses more). */
export const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024;

/** A file attached on this device, uploaded to the worker's spool (`POST /v1/uploads`, §10.4). */
export interface ComposeAttachment {
  key: string;
  filename: string;
  size: number;
  mimeType: string;
  status: "uploading" | "ready" | "error";
  uploadId?: string;
  error?: string;
}

export interface ComposeState {
  clientDraftId: string;
  kind: ComposeKind;
  accountId: string | null;
  identityEmail: string | null;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  /** Text typed into a recipient field but not committed as a chip yet. */
  pending: { to: string; cc: string; bcc: string };
  showCcBcc: boolean;
  subject: string;
  text: string;
  inReplyToMessageId?: string;
  forwardOfMessageId?: string;
  threadId?: string;
  /** Provider draft version (0 = never saved). */
  baseVersion: number;
  /** Files attached on this device (absent in copies saved before attachments existed). */
  attachments?: ComposeAttachment[];
}

export function attachmentsOf(state: Pick<ComposeState, "attachments">): ComposeAttachment[] {
  return state.attachments ?? [];
}

export function addAttachment(state: ComposeState, a: ComposeAttachment): ComposeState {
  return { ...state, attachments: [...attachmentsOf(state), a] };
}

export function updateAttachment(
  state: ComposeState,
  key: string,
  patch: Partial<Omit<ComposeAttachment, "key">>,
): ComposeState {
  return { ...state, attachments: attachmentsOf(state).map((a) => (a.key === key ? { ...a, ...patch } : a)) };
}

export function removeAttachment(state: ComposeState, key: string): ComposeState {
  return { ...state, attachments: attachmentsOf(state).filter((a) => a.key !== key) };
}

export function emptyCompose(
  clientDraftId: string,
  accountId: string | null,
  identityEmail: string | null,
): ComposeState {
  return {
    clientDraftId,
    kind: "new",
    accountId,
    identityEmail,
    to: [],
    cc: [],
    bcc: [],
    pending: { to: "", cc: "", bcc: "" },
    showCcBcc: false,
    subject: "",
    text: "",
    baseVersion: 0,
    attachments: [],
  };
}

/** The message a reply answers: the newest non-draft message from someone else, else the newest one. */
export function replyTarget(messages: readonly MessageMetaLike[]): MessageMetaLike | undefined {
  const real = messages.filter((m) => !m.isDraft).sort((a, b) => a.date - b.date);
  return [...real].reverse().find((m) => !m.isOutbound) ?? real[real.length - 1];
}

/**
 * A reply, reply-all or forward of a thread. `selfEmails` are the user's identities (lowercased), which never
 * appear as recipients of a reply-all.
 */
export function composeFromThread(
  kind: Exclude<ComposeKind, "new">,
  opts: {
    clientDraftId: string;
    threadId: string;
    subject: string;
    messages: readonly MessageMetaLike[];
    selfEmails: ReadonlySet<string>;
    identityEmail: string | null;
  },
): ComposeState {
  const target = replyTarget(opts.messages);
  const base = emptyCompose(opts.clientDraftId, target?.accountId ?? null, opts.identityEmail);
  base.kind = kind;
  base.threadId = opts.threadId;
  if (!target) return { ...base, subject: opts.subject };
  if (kind === "forward") {
    return {
      ...base,
      subject: prefixedSubject("forward", target.subject || opts.subject),
      forwardOfMessageId: target._id,
    };
  }
  const self = opts.selfEmails;
  let to: Addr[];
  if (target.isOutbound) to = target.to;
  else if (target.replyTo.length > 0) to = target.replyTo;
  else to = target.from ? [target.from] : [];
  to = dedupe(to, target.isOutbound ? new Set() : self);
  let cc: Addr[] = [];
  if (kind === "replyAll") {
    const exclude = new Set([...self, ...to.map((a) => a.email.toLowerCase())]);
    cc = dedupe([...(target.isOutbound ? [] : target.to), ...target.cc], exclude);
  }
  return {
    ...base,
    to,
    cc,
    showCcBcc: cc.length > 0,
    subject: prefixedSubject("reply", target.subject || opts.subject),
    inReplyToMessageId: target._id,
  };
}

/** Commits the pending text of a recipient field into chips; invalid parts stay pending. */
export function commitPending(state: ComposeState, field: "to" | "cc" | "bcc"): ComposeState {
  const { valid, invalid } = parseAddressList(state.pending[field]);
  if (valid.length === 0) return state;
  return {
    ...state,
    [field]: dedupe([...state[field], ...valid]),
    pending: { ...state.pending, [field]: invalid.join(", ") },
  };
}

export function removeRecipient(
  state: ComposeState,
  field: "to" | "cc" | "bcc",
  email: string,
): ComposeState {
  return { ...state, [field]: state[field].filter((a) => a.email !== email) };
}

export interface ComposeProblems {
  /** Blocks sending. */
  errors: string[];
  /** Shown, but sending is allowed. */
  warnings: string[];
}

export function validateCompose(state: ComposeState): ComposeProblems {
  const errors: string[] = [];
  const warnings: string[] = [];
  const committed = [...state.to, ...state.cc, ...state.bcc];
  const pending = (["to", "cc", "bcc"] as const).flatMap((f) => parseAddressList(state.pending[f]));
  const pendingValid = pending.flatMap((p) => p.valid);
  const pendingInvalid = pending.flatMap((p) => p.invalid);
  if (!state.accountId || !state.identityEmail) errors.push("Choose the address to send from.");
  if (committed.length + pendingValid.length === 0) errors.push("Add at least one recipient.");
  for (const bad of pendingInvalid) errors.push(`"${bad}" is not an email address.`);
  if (committed.length + pendingValid.length > 300) errors.push("At most 300 recipients.");
  if (!state.subject.trim()) warnings.push("The subject is empty.");
  const files = attachmentsOf(state);
  if (files.some((a) => a.status === "uploading"))
    errors.push("Wait for the attachments to finish uploading.");
  if (files.some((a) => a.status === "error")) errors.push("Remove the attachments that failed to upload.");
  if (files.reduce((n, a) => n + a.size, 0) > MAX_ATTACHMENT_BYTES)
    errors.push("Attachments are limited to 25 MB per message.");
  if (!state.text.trim() && !state.forwardOfMessageId && files.length === 0)
    warnings.push("The message is empty.");
  return { errors, warnings };
}

/** `DraftContent` for autosave and send (pending recipient text is committed first). */
export function draftContent(state: ComposeState) {
  let s = state;
  for (const f of ["to", "cc", "bcc"] as const) s = commitPending(s, f);
  return {
    accountId: s.accountId ?? "",
    identityEmail: s.identityEmail ?? "",
    to: s.to,
    cc: s.cc,
    bcc: s.bcc,
    subject: s.subject,
    html: null,
    text: s.text,
    ...(s.inReplyToMessageId ? { inReplyToMessageId: s.inReplyToMessageId } : {}),
    ...(s.forwardOfMessageId ? { forwardOfMessageId: s.forwardOfMessageId } : {}),
    // Uploaded files only; one still uploading joins the next autosave.
    attachments: attachmentsOf(s).flatMap((a) =>
      a.status === "ready" && a.uploadId ? [{ uploadId: a.uploadId }] : [],
    ),
  };
}

/** A stable fingerprint of what autosave would write; autosave runs when it changes. */
export function draftSignature(state: ComposeState): string {
  const d = draftContent(state);
  return JSON.stringify([d.identityEmail, d.to, d.cc, d.bcc, d.subject, d.text, d.attachments]);
}

/** Nothing worth saving yet (a new, untouched composer). */
export function isBlank(state: ComposeState): boolean {
  return (
    state.to.length + state.cc.length + state.bcc.length === 0 &&
    !state.pending.to.trim() &&
    !state.pending.cc.trim() &&
    !state.pending.bcc.trim() &&
    !state.subject.trim() &&
    !state.text.trim() &&
    attachmentsOf(state).length === 0
  );
}

/** Autosave debounce (§10.1). */
export const AUTOSAVE_MS = 2000;

export function recipientsSummary(state: ComposeState): string {
  const all = [...state.to, ...state.cc, ...state.bcc];
  if (all.length === 0) return "No recipients";
  return all.map(formatAddress).join(", ");
}

// ───────────── send with undo (§10.2, §10.3) ─────────────

export type SendPhase =
  | { kind: "idle" }
  | { kind: "submitting"; clientSendId: string; later: boolean }
  | { kind: "scheduled"; clientSendId: string; sendAt: number; later: boolean }
  | { kind: "cancelling"; clientSendId: string }
  | { kind: "cancelled"; clientSendId: string }
  | { kind: "sent"; clientSendId: string }
  | { kind: "too_late"; clientSendId: string }
  | { kind: "failed"; clientSendId: string; message: string };

export type SendEvent =
  | { type: "submit"; clientSendId: string; later: boolean }
  | { type: "accepted"; sendAt: number; state: "scheduled" | "due" | "cancelled" }
  | { type: "undo" }
  | { type: "cancel_result"; result: "cancelled" | "too_late" }
  | { type: "elapsed" }
  | { type: "error"; message: string };

/** The undo-send state machine. Undo is possible from the moment Send is pressed until `sendAt`. */
export function sendReducer(phase: SendPhase, event: SendEvent): SendPhase {
  switch (event.type) {
    case "submit":
      return { kind: "submitting", clientSendId: event.clientSendId, later: event.later };
    case "accepted":
      if (phase.kind === "submitting") {
        if (event.state === "cancelled") return { kind: "cancelled", clientSendId: phase.clientSendId };
        if (event.state === "due") return { kind: "sent", clientSendId: phase.clientSendId };
        return {
          kind: "scheduled",
          clientSendId: phase.clientSendId,
          sendAt: event.sendAt,
          later: phase.later,
        };
      }
      return phase; // undo already in progress: the cancel result decides
    case "undo":
      if (phase.kind === "submitting" || phase.kind === "scheduled")
        return { kind: "cancelling", clientSendId: phase.clientSendId };
      return phase;
    case "cancel_result":
      if (phase.kind !== "cancelling") return phase;
      return event.result === "cancelled"
        ? { kind: "cancelled", clientSendId: phase.clientSendId }
        : { kind: "too_late", clientSendId: phase.clientSendId };
    case "elapsed":
      return phase.kind === "scheduled" && !phase.later
        ? { kind: "sent", clientSendId: phase.clientSendId }
        : phase;
    case "error":
      if (phase.kind === "idle") return phase;
      return { kind: "failed", clientSendId: phase.clientSendId, message: event.message };
  }
}

/** Whether the Undo button is live. */
export function canUndo(phase: SendPhase, now: number): boolean {
  if (phase.kind === "submitting") return true;
  if (phase.kind === "scheduled") return phase.later || now < phase.sendAt;
  return false;
}

/** Status line for the send toast. */
export function sendStatus(phase: SendPhase, now: number, describe: (at: number) => string): string {
  switch (phase.kind) {
    case "idle":
      return "";
    case "submitting":
      return phase.later ? "Scheduling…" : "Sending…";
    case "scheduled":
      if (phase.later) return `Scheduled for ${describe(phase.sendAt)}`;
      return `Sending in ${Math.max(0, Math.ceil((phase.sendAt - now) / 1000))} s`;
    case "cancelling":
      return "Undoing…";
    case "cancelled":
      return "Not sent. The draft is back in the composer.";
    case "sent":
      return "Sent";
    case "too_late":
      return "Too late to undo: the message was already sent.";
    case "failed":
      return phase.message;
  }
}
