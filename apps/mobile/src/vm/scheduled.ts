// "Scheduled messages" (ARCHITECTURE §4.2 `sends.list` / `sends.reschedule` / `sends.cancel` /
// `sends.resolveUnknown`, §10.2 send later, §5.6 unknown sends; web D-283 parity): the user's later sends with
// Change time and Cancel, sends that failed, and sends whose outcome is unknown with "It was sent" / "It was not
// sent". Pure row models; the screen calls the mutations.

export interface ScheduledSendLike {
  _id: string;
  clientSendId: string;
  draftId?: string;
  kind: "undo" | "later";
  sendAt: number;
  state: "scheduled" | "cancelled" | "due" | "sending" | "sent" | "failed" | "unknown";
  preview: { subject: string; to: { name?: string; email: string }[] };
  error?: { code: string; message?: string };
}

export type ScheduledAction = "reschedule" | "cancel" | "was_sent" | "not_sent";

export interface ScheduledRow {
  id: string;
  clientSendId: string;
  subject: string;
  to: string;
  status: string;
  tone: "default" | "error";
  actions: { id: ScheduledAction; label: string }[];
  sendAt: number;
}

/** The states the screen asks `sends.list` for. */
export const SCHEDULED_STATES = ["scheduled", "failed", "unknown"] as const;

function recipients(to: ScheduledSendLike["preview"]["to"]): string {
  if (to.length === 0) return "No recipients";
  const names = to.map((a) => a.name?.trim() || a.email);
  return names.length > 3 ? `${names.slice(0, 3).join(", ")} and ${names.length - 3} more` : names.join(", ");
}

/** One row per send that has a draft and is not cancelled; scheduled first by time, then problems. */
export function scheduledRows(
  sends: readonly ScheduledSendLike[],
  describe: (at: number) => string,
): ScheduledRow[] {
  const rank = (s: ScheduledSendLike) => (s.state === "scheduled" ? 0 : s.state === "unknown" ? 1 : 2);
  return sends
    .filter((s) => s.state !== "cancelled" && s.draftId && (s.kind === "later" || s.state !== "scheduled"))
    .sort((a, b) => rank(a) - rank(b) || a.sendAt - b.sendAt)
    .flatMap((s): ScheduledRow[] => {
      const base = {
        id: s._id,
        clientSendId: s.clientSendId,
        subject: s.preview.subject.trim() || "(no subject)",
        to: recipients(s.preview.to),
        sendAt: s.sendAt,
      };
      if (s.state === "scheduled")
        return [
          {
            ...base,
            status: `Sends ${describe(s.sendAt)}`,
            tone: "default",
            actions: [
              { id: "reschedule", label: "Change time" },
              { id: "cancel", label: "Cancel" },
            ],
          },
        ];
      if (s.state === "failed")
        return [
          {
            ...base,
            status:
              s.error?.code === "stale_schedule"
                ? "Not sent: it was more than a day overdue. It is still in Drafts."
                : "Not sent: the provider refused it. It is still in Drafts.",
            tone: "error",
            actions: [],
          },
        ];
      if (s.state === "unknown")
        return [
          {
            ...base,
            status: "Akatsuki could not confirm that this was sent. Check your Sent folder.",
            tone: "error",
            actions: [
              { id: "was_sent", label: "It was sent" },
              { id: "not_sent", label: "It was not sent" },
            ],
          },
        ];
      return [];
    });
}

/** Toast text after an action succeeded. */
export const SCHEDULED_DONE: Record<ScheduledAction, string> = {
  reschedule: "Time changed",
  cancel: "Cancelled. The message stays in Drafts.",
  was_sent: "Marked as sent",
  not_sent: "Kept as a draft",
};
