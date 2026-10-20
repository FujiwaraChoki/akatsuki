// Entitlement (ARCHITECTURE §13.3). Pure: queries read the stored result, never the clock.
import type { Doc } from "../_generated/dataModel";
import type { Entitlement } from "../lib/validators";

const DAY = 24 * 60 * 60 * 1000;

/** Slack after `trialEnd` (and after `currentPeriodEnd`) for Stripe's own transition and a late webhook (D-411). */
export const TRIAL_GRACE_MS = 3 * DAY;

export const NO_ENTITLEMENT: Entitlement = { state: "none", hostedSync: false, managedAi: false, until: 0 };

export function billingDisabled(): boolean {
  return process.env.AKATSUKI_BILLING === "off";
}

export function selfhostEntitlement(): Entitlement {
  return {
    state: "selfhost",
    hostedSync: true,
    managedAi: process.env.AKATSUKI_MANAGED_AI === "on",
    until: Number.MAX_SAFE_INTEGER,
  };
}

type BillingFacts = Pick<Doc<"billing">, "status" | "trialEnd" | "currentPeriodEnd" | "pastDueSince">;

/** Maps the stored Stripe snapshot to an entitlement. `now` only matters for windows already over. */
export function computeEntitlement(billing: BillingFacts | null, now: number): Entitlement {
  if (billingDisabled()) return selfhostEntitlement();
  if (!billing) return NO_ENTITLEMENT;
  const lapsed: Entitlement = { state: "lapsed", hostedSync: false, managedAi: false, until: 0 };
  switch (billing.status) {
    case "trialing": {
      // Stripe moves a converting trial to `active` (or `past_due`, `canceled`) only after `trialEnd`, and the
      // webhook can be late. Until a snapshot says otherwise the trial stays entitled, with the same 3-day slack
      // as `active` (D-411); `until` is the re-evaluation time, the trial end itself is `billing.trialEnd`.
      if (billing.trialEnd === undefined) return lapsed;
      const until = billing.trialEnd + TRIAL_GRACE_MS;
      return until > now ? { state: "trialing", hostedSync: true, managedAi: true, until } : lapsed;
    }
    case "active": {
      const until = (billing.currentPeriodEnd ?? 0) + TRIAL_GRACE_MS;
      return until > now ? { state: "active", hostedSync: true, managedAi: true, until } : lapsed;
    }
    case "past_due": {
      const until = (billing.pastDueSince ?? now) + 7 * DAY;
      return until > now ? { state: "grace", hostedSync: true, managedAi: true, until } : lapsed;
    }
    case "canceled":
    case "unpaid":
    case "incomplete_expired":
    case "paused":
      return lapsed;
    case "none":
    case "incomplete":
      return NO_ENTITLEMENT;
  }
}

/** Managed-AI monthly budget in micro-dollars (§13.4): $8 on Pro, $1 during the trial. */
export function aiBudgetMicros(e: Entitlement): number {
  if (!e.managedAi) return 0;
  return e.state === "trialing" ? 1_000_000 : 8_000_000;
}

export function periodOf(now: number): string {
  const d = new Date(now);
  return `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}`;
}

export function nextPeriodStart(now: number): number {
  const d = new Date(now);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1);
}
