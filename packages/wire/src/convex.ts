// `@akatsuki/wire/convex`: the Convex side of the contract for web and mobile (ARCHITECTURE §1.7): function
// references, document and id types, the shared validators (§3.1) and the row-view types (§4.4).
// This entry pulls in the Convex sources for types, so it is kept apart from the main entry (which
// packages/ai and apps/api import under NodeNext resolution). Use it from Bundler-resolution projects.
import type {
  vAiSettingsView,
  vBillingView,
  vDeviceView,
  vDraftMeta,
  vIdentityView,
  vLabelView,
  vMailAccountView,
  vOpFailure,
  vOpSummary,
  vProfile,
  vRuleView,
  vScheduledSendView,
  vSenderPrefView,
  vSnippetView,
  vSplitView,
  vTimerView,
  vTriageInput,
} from "@akatsuki/convex/rows";
import type { Infer } from "convex/values";

export type { DataModel, Doc, Id, TableNames } from "@akatsuki/convex";
export { api } from "@akatsuki/convex";
export type { MessageMeta, ThreadRow } from "@akatsuki/convex/rows";
export * from "@akatsuki/convex/validators";

export type OpSummary = Infer<typeof vOpSummary>;
export type OpFailure = Infer<typeof vOpFailure>;
export type TriageInputRow = Infer<typeof vTriageInput>;
export type ProfileView = Infer<typeof vProfile>;
export type BillingView = Infer<typeof vBillingView>;
export type DeviceView = Infer<typeof vDeviceView>;
export type MailAccountView = Infer<typeof vMailAccountView>;
export type IdentityView = Infer<typeof vIdentityView>;
export type LabelView = Infer<typeof vLabelView>;
export type DraftMeta = Infer<typeof vDraftMeta>;
export type ScheduledSendView = Infer<typeof vScheduledSendView>;
export type TimerView = Infer<typeof vTimerView>;
export type SplitView = Infer<typeof vSplitView>;
export type AiSettingsView = Infer<typeof vAiSettingsView>;
export type RuleView = Infer<typeof vRuleView>;
export type SnippetView = Infer<typeof vSnippetView>;
export type SenderPrefView = Infer<typeof vSenderPrefView>;
