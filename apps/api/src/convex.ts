// apps/api → Convex: the ee.service.* functions (§4.3) under the apps/api service identity, plus the
// user-scoped ee.billing.entitlement query for /v1/ai/quota. Function references are built by name so this
// service does not typecheck the whole Convex tree; test/convex-contract.test.ts checks the names exist.
import { ConvexHttpClient } from "convex/browser";
import { makeFunctionReference } from "convex/server";

export type Tier = "fast" | "default" | "heavy" | "transcription";
export type Entitlement = {
  state: "none" | "trialing" | "active" | "grace" | "lapsed" | "selfhost";
  hostedSync: boolean;
  managedAi: boolean;
  until: number;
};
export type PrecheckResult =
  | { ok: true; remainingMicros: number }
  | {
      ok: false;
      code: "not_entitled" | "rate_limited" | "ai_budget_exhausted";
      retryAfterMs?: number;
      /** `transcription_day`: the 120 min/day dictation cap refused the request (§13.4, D-343). */
      limit?: "transcription_day";
    };
export type BillingStatus =
  | "none"
  | "trialing"
  | "active"
  | "past_due"
  | "canceled"
  | "unpaid"
  | "incomplete"
  | "incomplete_expired"
  | "paused";
export interface StripeSubSnapshot {
  id: string;
  status: BillingStatus;
  priceId: string;
  trialEnd?: number;
  currentPeriodEnd?: number;
  cancelAtPeriodEnd: boolean;
}

export interface UsageRecord {
  userId: string;
  feature: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  audioSeconds: number;
  costMicros: number;
}

export interface ConvexGateway {
  aiPrecheck(a: {
    userId: string;
    feature: string;
    tier: Tier;
    estCostMicros: number;
    /** Dictation: estimated recording length, checked against the daily transcription cap. */
    audioSeconds?: number;
  }): Promise<PrecheckResult>;
  aiRecordUsage(a: UsageRecord): Promise<{ remainingMicros: number }>;
  billingIdentity(a: { userId: string }): Promise<{
    email: string;
    stripeCustomerId?: string;
    trialUsedAt?: number;
    entitlement: Entitlement;
  }>;
  linkStripeCustomer(a: { userId: string; stripeCustomerId: string }): Promise<null>;
  recordStripeEvent(a: { eventId: string; type: string }): Promise<{ duplicate: boolean }>;
  applyStripeSubscription(a: {
    stripeCustomerId: string;
    userId?: string;
    snapshot: StripeSubSnapshot | null;
    fetchedAt: number;
  }): Promise<null>;
  trialWillEnd(a: { stripeCustomerId: string }): Promise<null>;
  /** ee.billing.entitlement with the user's own JWT. */
  userAiUsage(userJwt: string): Promise<{ period: string; usedMicros: number; limitMicros: number }>;
}

export const SERVICE_FUNCTIONS = {
  aiPrecheck: ["mutation", "ee/service:aiPrecheck"],
  aiRecordUsage: ["mutation", "ee/service:aiRecordUsage"],
  billingIdentity: ["query", "ee/service:billingIdentity"],
  linkStripeCustomer: ["mutation", "ee/service:linkStripeCustomer"],
  recordStripeEvent: ["mutation", "ee/service:recordStripeEvent"],
  applyStripeSubscription: ["mutation", "ee/service:applyStripeSubscription"],
  trialWillEnd: ["mutation", "ee/service:trialWillEnd"],
} as const;
export const USER_ENTITLEMENT = "ee/billing:entitlement";

export function createConvexGateway(opts: {
  convexUrl: string;
  serviceToken: () => Promise<string>;
}): ConvexGateway {
  const client = () => new ConvexHttpClient(opts.convexUrl, { logger: false });
  const call = async (name: keyof typeof SERVICE_FUNCTIONS, args: Record<string, unknown>) => {
    const [kind, path] = SERVICE_FUNCTIONS[name];
    const c = client();
    c.setAuth(await opts.serviceToken());
    return kind === "query"
      ? c.query(makeFunctionReference<"query">(path), args)
      : c.mutation(makeFunctionReference<"mutation">(path), args);
  };
  return {
    aiPrecheck: (a) => call("aiPrecheck", a),
    aiRecordUsage: (a) => call("aiRecordUsage", { ...a }),
    billingIdentity: (a) => call("billingIdentity", a),
    linkStripeCustomer: (a) => call("linkStripeCustomer", a),
    recordStripeEvent: (a) => call("recordStripeEvent", a),
    applyStripeSubscription: (a) => call("applyStripeSubscription", { ...a }),
    trialWillEnd: (a) => call("trialWillEnd", a),
    async userAiUsage(userJwt) {
      const c = client();
      c.setAuth(userJwt);
      const r = await c.query(makeFunctionReference<"query">(USER_ENTITLEMENT), {});
      return r.aiUsage;
    },
  };
}
