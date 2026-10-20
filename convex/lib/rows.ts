// Row projections (ARCHITECTURE §4.4). Validators for every view plus `project()` to strip fields,
// because Convex `returns` validators reject unknown fields.
import { type GenericValidator, type Infer, type VObject, v } from "convex/values";
import type { Doc } from "../_generated/dataModel";
import schema from "../schema";
import { labelKeyOf } from "./threads";
import { vAddr, vAppError, vLabelKey, vOpState, vTriageCategory } from "./validators";

const withSystem = <T extends string>(table: T) => ({
  _id: v.id(table),
  _creationTime: v.number(),
});

/** Copies only the fields the validator declares (and that are present on the source). */
// biome-ignore lint/suspicious/noExplicitAny: VObject's Type parameter is inferred per call
export function project<V extends VObject<any, Record<string, GenericValidator>>>(
  validator: V,
  source: Record<string, unknown>,
): Infer<V> {
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(validator.fields)) {
    if (source[key] !== undefined) out[key] = source[key];
  }
  return out as Infer<V>;
}

export const vThreadRow = v.object({
  _id: v.id("threads"),
  accountId: v.id("mailAccounts"),
  subject: v.string(),
  snippet: v.string(),
  participants: v.array(vAddr),
  lastFrom: v.optional(vAddr),
  lastMessageAt: v.number(),
  sortAt: v.number(),
  messageCount: v.number(),
  unreadCount: v.number(),
  draftCount: v.number(),
  hasAttachments: v.boolean(),
  isBulk: v.boolean(),
  labelKeys: v.array(vLabelKey),
  split: v.optional(v.string()),
  pendingOpCount: v.number(),
  snoozedUntil: v.optional(v.number()),
  returnedAt: v.optional(v.number()),
  reminderAt: v.optional(v.number()),
  triage: v.optional(v.object({ category: vTriageCategory, needsReply: v.boolean() })),
}); // ≈ 700 bytes per row
export type ThreadRow = Infer<typeof vThreadRow>;

export function toThreadRow(t: Doc<"threads">): ThreadRow {
  const row = project(vThreadRow, t);
  if (t.triage) row.triage = { category: t.triage.category, needsReply: t.triage.needsReply };
  return row;
}

export const vMessageMeta = schema.tables.messages.validator
  .extend(withSystem("messages"))
  .omit("userId", "rev", "imapLocations");
export type MessageMeta = Infer<typeof vMessageMeta>;
export const toMessageMeta = (m: Doc<"messages">): MessageMeta => project(vMessageMeta, m);

export const vOpSummary = v.object({
  _id: v.id("ops"),
  clientOpId: v.string(),
  state: vOpState,
  payloadKind: v.string(),
  createdAt: v.number(),
});
export const toOpSummary = (o: Doc<"ops">) => ({
  _id: o._id,
  clientOpId: o.clientOpId,
  state: o.state,
  payloadKind: o.payload.kind,
  createdAt: o.createdAt,
});

export const vOpFailure = v.object({
  _id: v.id("ops"),
  threadId: v.optional(v.id("threads")),
  subject: v.optional(v.string()),
  accountEmail: v.string(),
  payloadKind: v.string(),
  error: vAppError,
  state: v.union(v.literal("failed"), v.literal("unknown")),
  at: v.number(),
});

export const vTriageInput = v.object({
  threadId: v.id("threads"),
  from: v.optional(vAddr),
  to: v.array(vAddr),
  subject: v.string(),
  snippet: v.string(),
  listId: v.optional(v.string()),
  isBulk: v.boolean(),
  hasAttachments: v.boolean(),
}); // headers + snippet only; never body text

// ───────────── document views ─────────────

export const vProfile = schema.tables.profiles.validator.extend(withSystem("profiles")).omit("userId");
export const vBillingView = v.object({
  status: schema.tables.billing.validator.fields.status,
  trialEnd: v.optional(v.number()),
  currentPeriodEnd: v.optional(v.number()),
  cancelAtPeriodEnd: v.boolean(),
  trialUsedAt: v.optional(v.number()),
});
export const NO_BILLING_VIEW = { status: "none" as const, cancelAtPeriodEnd: false };

export const vDeviceView = schema.tables.devices.validator
  .extend(withSystem("devices"))
  .omit("userId", "expoPushToken");

const imapFields = schema.tables.mailAccounts.validator.fields.imap;
const vImapView = imapFields.omit("capabilities"); // still optional
export const vMailAccountView = schema.tables.mailAccounts.validator
  .extend(withSystem("mailAccounts"))
  .omit("userId", "shard", "imap")
  .extend({ imap: vImapView });
export function toMailAccountView(a: Doc<"mailAccounts">): Infer<typeof vMailAccountView> {
  const view = project(vMailAccountView, a);
  if (a.imap) {
    const { capabilities: _caps, ...rest } = a.imap;
    view.imap = rest;
  }
  return view;
}

export const vIdentityView = schema.tables.identities.validator
  .extend(withSystem("identities"))
  .omit("userId");

export const vLabelView = schema.tables.labels.validator
  .extend(withSystem("labels"))
  .omit("userId", "rev")
  .extend({ labelKey: v.string() });
export const toLabelView = (l: Doc<"labels">): Infer<typeof vLabelView> => ({
  ...project(vLabelView, l),
  labelKey: labelKeyOf(l),
});

export const vThreadMirror = schema.tables.threads.validator
  .extend(withSystem("threads"))
  .omit("userId", "triageLeaseUntil");

export const vDraftMeta = schema.tables.drafts.validator.extend(withSystem("drafts")).omit("userId");
export const vScheduledSendView = schema.tables.scheduledSends.validator
  .extend(withSystem("scheduledSends"))
  .omit("userId", "scheduledFnId");
export const vTimerView = schema.tables.timers.validator
  .extend(withSystem("timers"))
  .omit("userId", "scheduledFnId");
export const vSplitView = schema.tables.splits.validator
  .extend(withSystem("splits"))
  .omit("userId")
  .extend({ labelKey: v.string() });
export const toSplitView = (s: Doc<"splits">): Infer<typeof vSplitView> => ({
  ...project(vSplitView, s),
  labelKey: `split:${s._id}`,
});
export const vAiSettingsView = schema.tables.aiSettings.validator
  .extend(withSystem("aiSettings"))
  .omit("userId");
export const vFolderCursor = schema.tables.folderCursors.validator.extend({ _id: v.id("folderCursors") });

export const vRuleView = schema.tables.rules.validator.extend(withSystem("rules")).omit("userId");
export const vSnippetView = schema.tables.snippets.validator
  .extend(withSystem("snippets"))
  .omit("userId", "searchText");
export const vSenderPrefView = v.object({
  sender: v.string(),
  allowRemoteImages: v.boolean(),
  vip: v.boolean(),
  blocked: v.boolean(),
});
