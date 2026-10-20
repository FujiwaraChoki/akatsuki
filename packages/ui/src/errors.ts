// User-facing copy for every §15 error code (ARCHITECTURE §5.7, §15). Plain, direct sentences; no provider
// text, no user data except what the caller passes (a subject, a masked account). The record is typed against
// the registry in @akatsuki/wire, so a new code does not compile until it has copy here.
import type { ThreadAction } from "@akatsuki/mail-model";
import { type ErrorCode, isErrorCode, toErrorPayload } from "@akatsuki/wire/errors";

/** What the UI can offer next to the message. */
export type ErrorRemedy = "retry" | "reconnect" | "upgrade" | "settings" | "update" | "sign_in" | "none";

export interface ErrorCopy {
  /** A short sentence for a toast or banner title. */
  title: string;
  /** A lowercase clause for "Couldn't archive 'X' in sami@…: <reason>." */
  reason: string;
  remedy: ErrorRemedy;
}

const c = (title: string, reason: string, remedy: ErrorRemedy = "none"): ErrorCopy => ({
  title,
  reason,
  remedy,
});

export const ERROR_COPY: Record<ErrorCode, ErrorCopy> = {
  unauthenticated: c("Your session ended. Sign in again.", "your session ended", "sign_in"),
  not_found: c("That no longer exists.", "the message no longer exists"),
  not_entitled: c(
    "This needs Akatsuki Pro. Start a trial or subscribe in Settings.",
    "this needs Akatsuki Pro",
    "upgrade",
  ),
  invalid_argument: c("That request was not valid.", "the request was not valid"),
  invalid_action: c("That action is not available for this account.", "that action is not available here"),
  invalid_vars: c("The AI request was not valid. Try again with less text.", "the AI request was not valid"),
  payload_too_large: c("That is too large to send.", "it is too large"),
  rate_limited: c("Too many requests. Try again in a moment.", "too many requests right now", "retry"),
  ai_budget_exhausted: c(
    "This month's Akatsuki AI budget is used up. It resets at the start of next month.",
    "this month's AI budget is used up",
    "settings",
  ),
  ai_model_required: c(
    "Choose a model for this AI feature in Settings.",
    "no AI model is chosen",
    "settings",
  ),
  consent_required: c("Turn on AI features in Settings first.", "AI features are turned off", "settings"),
  ai_off: c(
    "This AI feature is turned off. Turn it on in Settings.",
    "this AI feature is turned off",
    "settings",
  ),
  key_missing: c(
    "The API key for this AI provider is not set on this device.",
    "the API key is not set on this device",
    "settings",
  ),
  prompt_version_mismatch: c(
    "Update Akatsuki to use this AI feature.",
    "this app version is out of date",
    "update",
  ),
  already_connected: c(
    "This mailbox is already connected to another Akatsuki account.",
    "the mailbox belongs to another Akatsuki account",
  ),
  already_subscribed: c("You already have Akatsuki Pro.", "you already have Akatsuki Pro"),
  draft_version_conflict: c(
    "This draft changed on another device. Reload it to keep editing.",
    "the draft changed on another device",
  ),
  lease_lost: c("Sync moved to another server. Try again.", "sync moved to another server", "retry"),
  version_conflict: c(
    "The account credentials changed. Try again.",
    "the account credentials changed",
    "retry",
  ),
  imap_auth_failed: c(
    "The IMAP server rejected the username or password.",
    "the IMAP server rejected the username or password",
  ),
  smtp_auth_failed: c(
    "The SMTP server rejected the username or password.",
    "the SMTP server rejected the username or password",
  ),
  tls_failed: c(
    "A secure connection to the server failed. Check the port and security setting.",
    "a secure connection failed",
  ),
  unreachable: c(
    "The server could not be reached. Check your connection.",
    "the server could not be reached",
    "retry",
  ),
  reauth_required: c(
    "Reconnect this account to keep it in sync.",
    "the account needs to be reconnected",
    "reconnect",
  ),
  reauth_timeout: c(
    "This change was dropped because the account was not reconnected within 7 days.",
    "the account was not reconnected in time",
    "reconnect",
  ),
  provider_transient: c(
    "The mail provider had a temporary problem. Try again.",
    "the mail provider had a temporary problem",
    "retry",
  ),
  provider_rate_limited: c(
    "The mail provider is limiting requests. Akatsuki will retry.",
    "the mail provider is limiting requests",
    "retry",
  ),
  provider_invalid: c("The mail provider rejected the change.", "the mail provider rejected the change"),
  provider_conflict: c("The mail provider reported a conflict.", "the mail provider reported a conflict"),
  provider_quota: c(
    "The mailbox is full or a sending limit was reached.",
    "the mailbox is full or a sending limit was reached",
  ),
  provider_unavailable: c(
    "This service is not available right now. Try again.",
    "the service is not available",
    "retry",
  ),
  unsupported: c("This account does not support that.", "this account does not support it"),
  draft_deleted: c("The draft was deleted before it was sent.", "the draft was deleted"),
  stale_schedule: c(
    "This scheduled message was not sent because it was more than a day overdue.",
    "it was more than a day overdue",
  ),
  send_unconfirmed: c(
    "We could not confirm this was sent. Check Sent, then choose Send again or Discard.",
    "we could not confirm it was sent",
  ),
  internal: c("Something went wrong. Try again.", "something went wrong", "retry"),
  not_implemented: c("This is not available yet.", "this is not available yet"),
};

/** Copy for a code; unknown codes (from a newer server) get the `internal` copy. */
export function errorCopy(code: string): ErrorCopy {
  return isErrorCode(code) ? ERROR_COPY[code] : ERROR_COPY.internal;
}

/** The title copy for anything thrown (WireError, ConvexError, AiFeatureError, Error). */
export function errorMessage(err: unknown): string {
  return errorCopy(toErrorPayload(err).code).title;
}

/** "sami@example.com" → "sami@…" (op failure toasts never show a full address, §5.7). */
export function maskAccount(email: string): string {
  const at = email.indexOf("@");
  return at <= 0 ? email : `${email.slice(0, at)}@…`;
}

const clip = (s: string, n: number) =>
  Array.from(s).length > n
    ? `${Array.from(s)
        .slice(0, n - 1)
        .join("")}…`
    : s;

/** The verb of a thread action in "Couldn't <verb> …", e.g. "archive", "move to Trash". */
export function actionVerb(action: ThreadAction): string {
  switch (action.kind) {
    case "trash":
      return "move to Trash";
    case "untrash":
      return "restore";
    case "delete_forever":
      return "delete";
    case "unsubscribe":
      return "unsubscribe from";
    case "move":
      if (action.to === "sys:inbox") return "move to Inbox";
      if (action.to === "sys:spam") return "report";
      if (action.to === "sys:snoozed") return "snooze";
      return "move";
    case "modify": {
      const has = (list: string[], k: string) => list.includes(k);
      if (has(action.add, "sys:spam")) return "report";
      if (has(action.add, "sys:snoozed")) return "snooze";
      if (has(action.remove, "sys:inbox") && action.add.length === 0) return "archive";
      if (has(action.add, "sys:inbox")) return "move to Inbox";
      if (has(action.remove, "sys:unread")) return "mark as read";
      if (has(action.add, "sys:unread")) return "mark as unread";
      if (has(action.add, "sys:starred")) return "star";
      if (has(action.remove, "sys:starred")) return "unstar";
      if (action.add.some((k) => k.startsWith("l:"))) return "label";
      if (action.remove.some((k) => k.startsWith("l:"))) return "remove the label from";
      return "update";
    }
  }
}

/**
 * The op-failure sentence of §5.7: "Couldn't archive 'Subject' in sami@…: the message no longer exists."
 * `verb` is `actionVerb(action)` or a custom verb ("send", "rename").
 */
export function describeOpFailure(opts: {
  verb: string;
  subject?: string;
  account?: string;
  code: string;
}): string {
  const subject = opts.subject !== undefined ? ` '${clip(opts.subject.trim() || "(no subject)", 60)}'` : "";
  const where = opts.account ? ` in ${maskAccount(opts.account)}` : "";
  return `Couldn't ${opts.verb}${subject}${where}: ${errorCopy(opts.code).reason}.`;
}
