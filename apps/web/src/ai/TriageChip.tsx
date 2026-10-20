// Triage labels (§12.5) on list rows and the open thread: the AI category and "Needs reply". Set by managed
// triage in the worker or by a client's BYO triage; rules and splits can use both.
import type { TriageCategory } from "@akatsuki/mail-model";
import { Badge } from "@akatsuki/ui";

export const TRIAGE_LABEL: Record<TriageCategory, string> = {
  important: "Important",
  personal: "Personal",
  updates: "Updates",
  newsletters: "Newsletter",
  promotions: "Promotion",
  receipts: "Receipt",
  calendar: "Calendar",
  notifications: "Notification",
  other: "Other",
};

/** Triage labels (§12.5): the category and "Needs reply", set by managed or client-side triage. */
export function TriageChip({ triage }: { triage?: { category: TriageCategory; needsReply: boolean } }) {
  if (!triage) return null;
  return (
    <>
      {triage.needsReply && (
        <Badge tone="accent" className="ak-triage ak-triage--reply">
          Needs reply
        </Badge>
      )}
      {triage.category !== "other" && (
        <Badge tone="outline" className="ak-triage" data-category={triage.category}>
          {TRIAGE_LABEL[triage.category]}
        </Badge>
      )}
    </>
  );
}
