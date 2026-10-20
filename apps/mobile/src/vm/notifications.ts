// Push notification settings (ARCHITECTURE §14.2, D-351). `prefs.notifications`: `off`, `private` (default:
// a count only) or `full` (sender and subject travel through Expo's push service to Apple or Google and show on
// the lock screen). Changing to `full` needs an explicit confirmation that says exactly that.

export type NotificationMode = "off" | "private" | "full";

export const DEFAULT_NOTIFICATION_MODE: NotificationMode = "private";

export interface ModeOption {
  mode: NotificationMode;
  title: string;
  detail: string;
}

export const NOTIFICATION_MODES: readonly ModeOption[] = [
  {
    mode: "private",
    title: "Private",
    detail: 'Only "New mail" or "3 new messages". No sender, subject or preview leaves Akatsuki.',
  },
  {
    mode: "full",
    title: "Sender and subject",
    detail: "Shows who wrote and the subject. This text passes through Expo, Apple and Google.",
  },
  { mode: "off", title: "Off", detail: "No push notifications on any device." },
];

/** The confirmation shown before switching to `full`. */
export const FULL_MODE_WARNING = {
  title: "Show sender and subject?",
  body:
    "Each notification will contain the sender's name and the subject. To deliver it, Akatsuki hands that text to " +
    "Expo's push service (exp.host), which passes it to Apple (APNs) or Google (Firebase Cloud Messaging). They " +
    "can see it, and it appears on your lock screen. Message text, recipients and previews are never sent.",
  confirm: "Show sender and subject",
  cancel: "Keep private",
};

/** Whether changing mode needs the warning first. */
export function needsWarning(from: NotificationMode, to: NotificationMode): boolean {
  return to === "full" && from !== "full";
}

/** Whether the device should hold a push token for this mode. */
export function wantsPushToken(mode: NotificationMode): boolean {
  return mode !== "off";
}

/**
 * Where a tapped notification leads. Only `full` notifications carry a thread id (§14.2); a malformed or
 * missing id opens the inbox.
 */
export function notificationTarget(
  data: unknown,
): { pathname: "/thread/[threadId]"; params: { threadId: string } } | { pathname: "/inbox" } {
  if (data && typeof data === "object") {
    const id = (data as Record<string, unknown>).threadId;
    if (typeof id === "string" && /^[A-Za-z0-9_-]{8,64}$/.test(id))
      return { pathname: "/thread/[threadId]", params: { threadId: id } };
  }
  return { pathname: "/inbox" };
}

/** Copy for the OS permission state shown under the mode picker. */
export function permissionText(
  status: "granted" | "denied" | "undetermined" | "unavailable",
  hasProject: boolean,
): string | null {
  if (!hasProject) return "Push is not configured in this build (no EAS project id).";
  if (status === "denied") return "Notifications are turned off for Akatsuki in system settings.";
  if (status === "unavailable") return "Push notifications need a physical device.";
  return null;
}
