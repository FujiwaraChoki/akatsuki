// Shared validators (ARCHITECTURE §3.1). Web, mobile and apps/api import these through packages/wire.
import { type Infer, v } from "convex/values";

export const vProvider = v.union(v.literal("gmail"), v.literal("graph"), v.literal("imap"));
export const vAddr = v.object({ name: v.optional(v.string()), email: v.string() }); // email lowercased, trimmed
export const vLabelKey = v.string(); // grammar §1.4; every mutation checks isLabelKey()

export const vAppError = v.object({ code: v.string(), message: v.string(), retryable: v.boolean() });

export const vThreadAction = v.union(
  v.object({ kind: v.literal("modify"), add: v.array(vLabelKey), remove: v.array(vLabelKey) }),
  v.object({ kind: v.literal("trash") }),
  v.object({ kind: v.literal("untrash") }),
  v.object({ kind: v.literal("delete_forever") }), // only if account.caps.deleteForever
  v.object({ kind: v.literal("move"), to: vLabelKey }), // folder providers only (caps.multiLabel === false)
  v.object({
    kind: v.literal("unsubscribe"),
    method: v.union(v.literal("one_click_post"), v.literal("mailto")),
  }),
);

export const vOpPayload = v.union(
  v.object({ kind: v.literal("thread"), action: vThreadAction }),
  v.object({ kind: v.literal("send"), scheduledSendId: v.id("scheduledSends") }),
  v.object({ kind: v.literal("delete_draft"), draftId: v.id("drafts") }),
  v.object({ kind: v.literal("label_create"), labelId: v.id("labels") }),
  v.object({ kind: v.literal("label_rename"), labelId: v.id("labels"), name: v.string() }),
  v.object({ kind: v.literal("label_delete"), labelId: v.id("labels") }),
);
export const vOpState = v.union(
  v.literal("queued"),
  v.literal("inflight"),
  v.literal("done"),
  v.literal("failed"),
  v.literal("cancelled"),
  v.literal("unknown"),
);

export const vTriageCategory = v.union(
  v.literal("important"),
  v.literal("personal"),
  v.literal("updates"),
  v.literal("newsletters"),
  v.literal("promotions"),
  v.literal("receipts"),
  v.literal("calendar"),
  v.literal("notifications"),
  v.literal("other"),
);
export const vTriage = v.object({
  category: vTriageCategory,
  needsReply: v.boolean(),
  source: v.union(v.literal("managed"), v.literal("client"), v.literal("rule")),
  model: v.string(), // resolved model id, e.g. "anthropic/claude-haiku-5.5"
  promptVersion: v.number(),
  at: v.number(),
});

// Splits and filter rules share one condition language, evaluated on metadata only.
export const vRuleCond = v.union(
  v.object({ f: v.literal("from_email"), eq: v.string() }), // exact, lowercased
  v.object({ f: v.literal("from_domain"), eq: v.string() }), // "example.com", matches subdomains
  v.object({ f: v.literal("to_email"), eq: v.string() }), // any participant on to/cc
  v.object({ f: v.literal("label"), key: vLabelKey }), // sys:* or l:* only
  v.object({ f: v.literal("triage"), category: vTriageCategory }),
  v.object({ f: v.literal("needs_reply"), eq: v.boolean() }),
  v.object({ f: v.literal("is_bulk"), eq: v.boolean() }),
  v.object({ f: v.literal("list_id"), eq: v.string() }),
  v.object({ f: v.literal("subject_contains"), text: v.string() }), // case-insensitive substring
  v.object({ f: v.literal("vip"), eq: v.boolean() }), // sender has senderPrefs.vip
  v.object({ f: v.literal("account"), accountId: v.id("mailAccounts") }),
);
export const vRule = v.object({
  match: v.union(v.literal("all"), v.literal("any")),
  conds: v.array(vRuleCond), // 1..20
});

export const vPrefs = v.object({
  undoSendSeconds: v.number(), // 0 | 5 | 10 | 20 | 30, default 10
  remoteImages: v.union(v.literal("block"), v.literal("proxy")), // default "block"
  readReceipts: v.object({ enabled: v.boolean(), warningAcknowledgedAt: v.optional(v.number()) }),
  cacheBodiesOnDevice: v.boolean(), // default true
  density: v.union(v.literal("compact"), v.literal("comfortable")),
  theme: v.union(v.literal("system"), v.literal("light"), v.literal("dark")),
  keymap: v.union(v.literal("superhuman"), v.literal("gmail")),
  unifiedInbox: v.boolean(),
  notifications: v.union(v.literal("off"), v.literal("private"), v.literal("full")),
});

export const vEntitlement = v.object({
  state: v.union(
    v.literal("none"),
    v.literal("trialing"),
    v.literal("active"),
    v.literal("grace"),
    v.literal("lapsed"),
    v.literal("selfhost"),
  ),
  hostedSync: v.boolean(), // may connect/sync mailboxes and use mail functions
  managedAi: v.boolean(), // may use the Akatsuki-managed AI provider
  until: v.number(), // ms; when this must be re-evaluated (0 = never entitled)
});

export const vAiProviderKind = v.union(
  v.literal("akatsuki"),
  v.literal("openrouter"),
  v.literal("anthropic"),
  v.literal("openai"),
  v.literal("google"),
  v.literal("gateway"),
  v.literal("ollama"),
  v.literal("openai_compatible"),
);
export const vAiFeature = v.union(
  v.literal("summarize"),
  v.literal("instant_reply"),
  v.literal("compose"),
  v.literal("rewrite"),
  v.literal("extract_actions"),
  v.literal("triage"),
  v.literal("nl_search"),
  v.literal("voice_profile"),
  v.literal("dictation"),
);
export type ThreadAction = Infer<typeof vThreadAction>;
export type OpPayload = Infer<typeof vOpPayload>;
export type Rule = Infer<typeof vRule>;
export type RuleCond = Infer<typeof vRuleCond>;
export type Prefs = Infer<typeof vPrefs>;
export type Entitlement = Infer<typeof vEntitlement>;
export type Addr = Infer<typeof vAddr>;
export type Triage = Infer<typeof vTriage>;
export type TriageCategory = Infer<typeof vTriageCategory>;
export type AppError = Infer<typeof vAppError>;
export type OpState = Infer<typeof vOpState>;
