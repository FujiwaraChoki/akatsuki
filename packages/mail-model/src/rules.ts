// Split and filter rule matching on metadata (ARCHITECTURE §3.4). Rust twin: crates/core::model::rules.
// Both run fixtures/rules.json.
//
// A rule sees only: lastFrom.email, participants, labelKeys, triage, isBulk, listId, subject, accountId and
// VIP status of the sender. Nothing else.
import type { Rule, RuleCond } from "./types.ts";

/** What a rule can see about a thread. */
export interface RuleFacts {
  /** `threads.lastFrom.email`. */
  fromEmail?: string;
  /** Participant emails (to/cc/from of the newest messages). */
  participants: readonly string[];
  labelKeys: readonly string[];
  /** Absent until the thread is triaged; conditions on triage then never match. */
  triage?: { category: string; needsReply: boolean };
  isBulk: boolean;
  listId?: string;
  subject: string;
  accountId: string;
  /** The sender (address or `@domain`) has `senderPrefs.vip`. */
  vip: boolean;
}

/** The lowercased domain after the last `@`, or "" when there is none. */
export function domainOf(email: string): string {
  const at = email.lastIndexOf("@");
  return at < 0 ? "" : email.slice(at + 1).toLowerCase();
}

export function condMatches(c: RuleCond, t: RuleFacts): boolean {
  switch (c.f) {
    case "from_email":
      return t.fromEmail !== undefined && t.fromEmail.toLowerCase() === c.eq.toLowerCase();
    case "from_domain": {
      if (t.fromEmail === undefined) return false;
      const d = domainOf(t.fromEmail);
      const want = c.eq.toLowerCase().replace(/^@/, "");
      return d === want || d.endsWith(`.${want}`);
    }
    case "to_email":
      return t.participants.some((p) => p.toLowerCase() === c.eq.toLowerCase());
    case "label":
      return t.labelKeys.includes(c.key);
    case "triage":
      return t.triage !== undefined && t.triage.category === c.category;
    case "needs_reply":
      return t.triage !== undefined && t.triage.needsReply === c.eq;
    case "is_bulk":
      return t.isBulk === c.eq;
    case "list_id":
      return t.listId !== undefined && t.listId.toLowerCase() === c.eq.toLowerCase();
    case "subject_contains":
      return c.text.length > 0 && t.subject.toLowerCase().includes(c.text.toLowerCase());
    case "vip":
      return t.vip === c.eq;
    case "account":
      return t.accountId === c.accountId;
  }
}

/** A rule with no conditions never matches. */
export function matchRule(rule: Rule, facts: RuleFacts): boolean {
  if (rule.conds.length === 0) return false;
  return rule.match === "all"
    ? rule.conds.every((c) => condMatches(c, facts))
    : rule.conds.some((c) => condMatches(c, facts));
}

/** The first split (in `order`) whose rule matches an inbox thread, else `"split:other"` (§3.4). */
export function firstMatchingSplit(splits: readonly { _id: string; rule: Rule }[], facts: RuleFacts): string {
  for (const s of splits) if (matchRule(s.rule, facts)) return `split:${s._id}`;
  return "split:other";
}
