// The structured rule language of splits and filter rules (ARCHITECTURE §3.1 `vRuleCond`, §3.4), for the
// editors: field metadata, defaults, validation, and the mapping between filter actions shown in the UI and
// `ThreadAction`s (built with `actionsFor`, so folder accounts get `move`s). Pure; tested in test/settings.
import { actionsFor, type ThreadAction, type TriageCategory } from "@akatsuki/mail-model";
import type { LabelView, MailAccountView, Rule } from "../backend/types.ts";

export type RuleCond = Rule["conds"][number];
export type CondField = RuleCond["f"];

export const MAX_SPLITS = 20;
export const MAX_CONDS = 20;

export const TRIAGE_CATEGORIES: readonly TriageCategory[] = [
  "important",
  "personal",
  "updates",
  "newsletters",
  "promotions",
  "receipts",
  "calendar",
  "notifications",
  "other",
];

export const COND_FIELDS: readonly {
  f: CondField;
  label: string;
  kind: "text" | "bool" | "label" | "triage" | "account";
  placeholder?: string;
}[] = [
  { f: "from_email", label: "From address", kind: "text", placeholder: "name@example.com" },
  { f: "from_domain", label: "From domain", kind: "text", placeholder: "example.com" },
  { f: "to_email", label: "To or Cc address", kind: "text", placeholder: "team@example.com" },
  { f: "subject_contains", label: "Subject contains", kind: "text", placeholder: "invoice" },
  { f: "list_id", label: "Mailing list id", kind: "text", placeholder: "news.example.com" },
  { f: "label", label: "Has label", kind: "label" },
  { f: "triage", label: "AI category", kind: "triage" },
  { f: "needs_reply", label: "Needs a reply (AI)", kind: "bool" },
  { f: "is_bulk", label: "Bulk or newsletter", kind: "bool" },
  { f: "vip", label: "Sender is a VIP", kind: "bool" },
  { f: "account", label: "Account", kind: "account" },
];

export function fieldInfo(f: CondField) {
  return COND_FIELDS.find((x) => x.f === f)!;
}

/** A fresh condition for a field (keeps the old value when both are text). */
export function defaultCond(
  f: CondField,
  ctx: { labels: readonly LabelView[]; accounts: readonly MailAccountView[] },
  prev?: RuleCond,
): RuleCond {
  const prevText =
    prev && "eq" in prev && typeof prev.eq === "string" ? prev.eq : prev && "text" in prev ? prev.text : "";
  switch (f) {
    case "from_email":
    case "from_domain":
    case "to_email":
    case "list_id":
      return { f, eq: prevText };
    case "subject_contains":
      return { f, text: prevText };
    case "label":
      return { f, key: ctx.labels.find((l) => l.kind !== "system")?.labelKey ?? "sys:starred" };
    case "triage":
      return { f, category: "newsletters" };
    case "needs_reply":
    case "is_bulk":
    case "vip":
      return { f, eq: true };
    case "account":
      return { f, accountId: (ctx.accounts[0]?._id ?? "") as never };
  }
}

/** The editable text of a text condition (or null for the other kinds). */
export function condText(c: RuleCond): string | null {
  if (c.f === "subject_contains") return c.text;
  if (c.f === "from_email" || c.f === "from_domain" || c.f === "to_email" || c.f === "list_id") return c.eq;
  return null;
}

export function withText(c: RuleCond, text: string): RuleCond {
  if (c.f === "subject_contains") return { ...c, text };
  if (c.f === "from_email" || c.f === "from_domain" || c.f === "to_email" || c.f === "list_id")
    return { ...c, eq: text };
  return c;
}

/** Normalises values the server compares lowercased (addresses, domains). */
export function normaliseRule(rule: Rule): Rule {
  return {
    match: rule.match,
    conds: rule.conds.map((c) => {
      if (c.f === "from_email" || c.f === "to_email") return { ...c, eq: c.eq.trim().toLowerCase() };
      if (c.f === "from_domain") return { ...c, eq: c.eq.trim().toLowerCase().replace(/^@/, "") };
      if (c.f === "list_id") return { ...c, eq: c.eq.trim().replace(/^<|>$/g, "") };
      if (c.f === "subject_contains") return { ...c, text: c.text.trim() };
      return c;
    }),
  };
}

/** Why a rule cannot be saved, or null. */
export function ruleProblem(name: string, rule: Rule): string | null {
  if (!name.trim()) return "Give it a name.";
  if (name.trim().length > 60) return "Keep the name under 60 characters.";
  if (rule.conds.length === 0) return "Add at least one condition.";
  if (rule.conds.length > MAX_CONDS) return `At most ${MAX_CONDS} conditions.`;
  for (const c of rule.conds) {
    const t = condText(c);
    if (t !== null && !t.trim()) return `Fill in “${fieldInfo(c.f).label}”.`;
    if ((c.f === "from_email" || c.f === "to_email") && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(c.eq.trim()))
      return `${c.eq || "That"} is not an email address.`;
    if (c.f === "from_domain" && !/^@?[^\s@]+\.[^\s@]+$/.test(c.eq.trim())) return `${c.eq} is not a domain.`;
  }
  return null;
}

// ───────────── filter actions ─────────────

export type UiAction =
  | { type: "archive" }
  | { type: "read" }
  | { type: "star" }
  | { type: "important" }
  | { type: "trash" }
  | { type: "spam" }
  | { type: "label"; labelId: string }
  | { type: "move"; folderKey: string }
  | { type: "custom"; action: ThreadAction };

export const UI_ACTIONS: readonly { type: Exclude<UiAction["type"], "custom">; label: string }[] = [
  { type: "archive", label: "Archive" },
  { type: "read", label: "Mark as read" },
  { type: "star", label: "Star" },
  { type: "important", label: "Mark important" },
  { type: "label", label: "Add label" },
  { type: "move", label: "Move to folder" },
  { type: "trash", label: "Move to Trash" },
  { type: "spam", label: "Mark as spam" },
];

/** UI action → `ThreadAction` for one account (folder providers get `move`s). Throws when unavailable. */
export function toThreadAction(
  a: UiAction,
  account: MailAccountView,
  labels: readonly LabelView[],
): ThreadAction {
  const h = actionsFor(
    account,
    labels.filter((l) => l.accountId === account._id),
  );
  switch (a.type) {
    case "archive":
      return h.archive();
    case "read":
      return h.markRead();
    case "star":
      return h.star();
    case "important":
      return { kind: "modify", add: ["sys:important"], remove: [] };
    case "trash":
      return h.trash();
    case "spam":
      return h.spam();
    case "label":
      return h.addLabel(a.labelId);
    case "move":
      return h.moveTo(a.folderKey);
    case "custom":
      return a.action;
  }
}

const same = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((x) => b.includes(x));

/** `ThreadAction` → UI action, recognising what `toThreadAction` builds; anything else is `custom`. */
export function fromThreadAction(t: ThreadAction, labels: readonly LabelView[]): UiAction {
  if (t.kind === "trash") return { type: "trash" };
  if (t.kind === "move") {
    if (t.to === "sys:trash") return { type: "trash" };
    if (t.to === "sys:spam") return { type: "spam" };
    const folder = labels.find((l) => l.labelKey === t.to);
    if (folder?.role === "archive") return { type: "archive" };
    return { type: "move", folderKey: t.to };
  }
  if (t.kind === "modify") {
    if (same(t.add, []) && same(t.remove, ["sys:inbox"])) return { type: "archive" };
    if (same(t.add, []) && same(t.remove, ["sys:unread"])) return { type: "read" };
    if (same(t.add, ["sys:starred"]) && same(t.remove, [])) return { type: "star" };
    if (same(t.add, ["sys:important"]) && same(t.remove, [])) return { type: "important" };
    if (same(t.add, ["sys:spam"]) && same(t.remove, ["sys:inbox"])) return { type: "spam" };
    if (t.add.length === 1 && t.add[0]!.startsWith("l:") && t.remove.length === 0)
      return { type: "label", labelId: t.add[0]!.slice(2) };
  }
  return { type: "custom", action: t };
}

/** Rules whose name marks them as a sender block (see src/senders/block.ts). */
export const BLOCK_RULE_PREFIX = "Blocked: ";
