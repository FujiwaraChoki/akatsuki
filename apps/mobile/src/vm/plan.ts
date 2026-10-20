// Plan status (ARCHITECTURE §13.3 entitlement, §6.4/§13.1: the mobile app sells nothing). The app shows the
// state of the account's plan and never a price, a purchase button or a link to buy (App Store 3.1.1/3.1.3(b),
// and the same on Google Play, D-333).
import { describeTime } from "./snooze.ts";

export interface EntitlementLike {
  state: "none" | "trialing" | "active" | "grace" | "lapsed" | "selfhost";
  hostedSync: boolean;
  managedAi: boolean;
  until: number;
}

export interface BillingLike {
  status: string;
  trialEnd?: number;
  currentPeriodEnd?: number;
  cancelAtPeriodEnd: boolean;
}

export interface PlanStatus {
  title: string;
  detail: string | null;
  /** Mail is available (hosted sync entitlement). */
  mailAvailable: boolean;
  /** Akatsuki-managed AI is available. */
  managedAi: boolean;
}

export function planStatus(
  ent: EntitlementLike | undefined,
  billing: BillingLike | undefined,
  now: number,
  timeZone: string,
): PlanStatus {
  const when = (at: number | undefined) => (at ? describeTime(at, now, timeZone) : null);
  const base = { mailAvailable: ent?.hostedSync ?? false, managedAi: ent?.managedAi ?? false };
  switch (ent?.state) {
    case "trialing": {
      const end = when(billing?.trialEnd ?? ent.until);
      return { ...base, title: "Akatsuki Pro trial", detail: end ? `Trial ends ${end}.` : null };
    }
    case "active": {
      const end = when(billing?.currentPeriodEnd);
      const detail = billing?.cancelAtPeriodEnd
        ? end
          ? `Ends ${end}; it will not renew.`
          : "It will not renew."
        : end
          ? `Renews ${end}.`
          : null;
      return { ...base, title: "Akatsuki Pro", detail };
    }
    case "grace":
      return {
        ...base,
        title: "Akatsuki Pro, payment due",
        detail: "The last payment did not go through. Mail keeps syncing for now.",
      };
    case "lapsed":
      return { ...base, title: "Akatsuki Pro has ended", detail: "Mail is paused on this account." };
    case "selfhost":
      return {
        ...base,
        title: "Self-hosted",
        detail: base.managedAi ? null : "Akatsuki AI is not enabled on this server.",
      };
    default:
      return { ...base, title: "No active plan", detail: "Mail is not available on this account yet." };
  }
}
