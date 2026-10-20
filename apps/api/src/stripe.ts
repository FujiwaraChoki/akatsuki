// Stripe (ARCHITECTURE §13.1, §13.2). The secret key and webhook secret live only on the Hetzner host (D-013).
import Stripe from "stripe";
import type { BillingStatus, StripeSubSnapshot } from "./convex.ts";

const KNOWN_STATUSES: readonly BillingStatus[] = [
  "trialing",
  "active",
  "past_due",
  "canceled",
  "unpaid",
  "incomplete",
  "incomplete_expired",
  "paused",
];

/** Stripe may add statuses; anything unknown is treated as not entitled ("incomplete"). */
export function normalizeStatus(status: string): BillingStatus {
  return (KNOWN_STATUSES as readonly string[]).includes(status) ? (status as BillingStatus) : "incomplete";
}

export interface BillingProvider {
  /** Verifies the `stripe-signature` header against the raw body; throws on any mismatch. */
  constructEvent(rawBody: string, signature: string): Stripe.Event;
  /** The customer's most recent subscription (any status), re-fetched so event order never matters. */
  latestSubscription(customerId: string): Promise<Stripe.Subscription | null>;
  createCustomer(a: { email: string; userId: string }): Promise<string>;
  createCheckout(a: { customerId: string; userId: string; trialEligible: boolean }): Promise<string>;
  createPortal(customerId: string): Promise<string>;
  cancel(customerId: string, immediately: boolean): Promise<void>;
}

const ms = (s: number | null | undefined) => (typeof s === "number" ? s * 1000 : undefined);

/** `StripeSubSnapshot` from a subscription, times converted to ms (§13.2). */
export function toSnapshot(sub: Stripe.Subscription): StripeSubSnapshot {
  const item = sub.items?.data?.[0];
  const snapshot: StripeSubSnapshot = {
    id: sub.id,
    status: normalizeStatus(sub.status),
    priceId: item?.price?.id ?? "",
    cancelAtPeriodEnd: Boolean(sub.cancel_at_period_end),
  };
  const trialEnd = ms(sub.trial_end);
  if (trialEnd !== undefined) snapshot.trialEnd = trialEnd;
  const periodEnd = ms(item?.current_period_end);
  if (periodEnd !== undefined) snapshot.currentPeriodEnd = periodEnd;
  return snapshot;
}

/** Event types whose arrival triggers a snapshot re-fetch (§13.2 step 3). */
export const SNAPSHOT_EVENTS = new Set([
  "checkout.session.completed",
  "customer.subscription.created",
  "customer.subscription.updated",
  "customer.subscription.deleted",
  "customer.subscription.paused",
  "customer.subscription.resumed",
  "invoice.paid",
  "invoice.payment_failed",
]);

const idOf = (v: string | { id: string } | null | undefined) => (typeof v === "string" ? v : v?.id);

/** The Stripe customer id and, when the object carries it, the Akatsuki userId. */
export function eventCustomer(event: Stripe.Event): { customerId?: string; userId?: string } {
  const obj = event.data.object as unknown as Record<string, unknown>;
  const customerId = idOf(obj.customer as string | { id: string } | null | undefined);
  const metadata = (obj.metadata ?? {}) as Record<string, string>;
  const userId =
    (typeof obj.client_reference_id === "string" ? obj.client_reference_id : undefined) ?? metadata.userId;
  return { customerId, userId: userId || undefined };
}

/** One trial per user, ever: 14 days, card up front (BRIEF decision 7, §13.1). */
export const TRIAL_DAYS = 14;
/** How long the resolved price id is reused (§13.1 step 3). */
export const PRICE_CACHE_MS = 10 * 60 * 1000;

/**
 * Stripe client. The secret key and webhook secret come from the environment at runtime (STRIPE_SECRET_KEY,
 * STRIPE_WEBHOOK_SECRET via config.ts) and are never logged. `fetch` is a test seam: tests pass a stub, so
 * no request reaches Stripe.
 */
export function createStripeBilling(opts: {
  secretKey: string;
  webhookSecret: string;
  priceLookupKey: string;
  siteUrl: string;
  now?: () => number;
  fetch?: typeof globalThis.fetch;
}): BillingProvider {
  const stripe = new Stripe(opts.secretKey, {
    maxNetworkRetries: opts.fetch ? 0 : 2,
    timeout: 20_000,
    ...(opts.fetch ? { httpClient: Stripe.createFetchHttpClient(opts.fetch) } : {}),
  });
  const now = opts.now ?? Date.now;
  let price: { id: string; at: number } | undefined;

  async function priceId(): Promise<string> {
    if (price && now() - price.at < PRICE_CACHE_MS) return price.id;
    const list = await stripe.prices.list({ lookup_keys: [opts.priceLookupKey], active: true, limit: 1 });
    const id = list.data[0]?.id;
    if (!id) throw new Error("no active Stripe price for the lookup key");
    price = { id, at: now() };
    return id;
  }

  async function latestSubscription(customerId: string) {
    const list = await stripe.subscriptions.list({ customer: customerId, status: "all", limit: 1 });
    return list.data[0] ?? null;
  }

  return {
    constructEvent: (rawBody, signature) =>
      stripe.webhooks.constructEvent(rawBody, signature, opts.webhookSecret),
    latestSubscription,
    async createCustomer({ email, userId }) {
      const customer = await stripe.customers.create({ email, metadata: { userId } });
      return customer.id;
    },
    async createCheckout({ customerId, userId, trialEligible }) {
      const session = await stripe.checkout.sessions.create({
        mode: "subscription",
        customer: customerId,
        client_reference_id: userId,
        line_items: [{ price: await priceId(), quantity: 1 }],
        payment_method_collection: "always",
        subscription_data: trialEligible
          ? {
              trial_period_days: TRIAL_DAYS,
              metadata: { userId },
              trial_settings: { end_behavior: { missing_payment_method: "cancel" } },
            }
          : { metadata: { userId } },
        success_url: `${opts.siteUrl}/billing/done?session_id={CHECKOUT_SESSION_ID}`,
        cancel_url: `${opts.siteUrl}/billing`,
      });
      if (!session.url) throw new Error("Stripe returned no checkout URL");
      return session.url;
    },
    async createPortal(customerId) {
      const portal = await stripe.billingPortal.sessions.create({
        customer: customerId,
        return_url: `${opts.siteUrl}/billing`,
      });
      return portal.url;
    },
    async cancel(customerId, immediately) {
      const list = await stripe.subscriptions.list({ customer: customerId, status: "all", limit: 10 });
      for (const sub of list.data) {
        if (!["trialing", "active", "past_due", "unpaid", "incomplete", "paused"].includes(sub.status))
          continue;
        if (immediately) await stripe.subscriptions.cancel(sub.id);
        else await stripe.subscriptions.update(sub.id, { cancel_at_period_end: true });
      }
    },
  };
}

/**
 * Webhook dedupe inside one apps/api process (§13.2): Stripe retries and may deliver an event twice, even
 * concurrently. An event id that finished here is answered 200 at once, and a second delivery while the
 * first is still running waits for it instead of re-fetching. Across restarts `ee.service.recordStripeEvent`
 * is the durable dedupe; snapshot events are idempotent anyway (D-122). Failed events are not remembered,
 * so Stripe's retry runs them again.
 */
export class StripeEventDedupe {
  private readonly done = new Map<string, number>();
  private readonly running = new Map<string, Promise<void>>();
  private readonly max: number;
  private readonly ttlMs: number;
  private readonly now: () => number;
  constructor(opts: { max?: number; ttlMs?: number; now?: () => number } = {}) {
    this.max = opts.max ?? 10_000;
    this.ttlMs = opts.ttlMs ?? 24 * 60 * 60 * 1000;
    this.now = opts.now ?? Date.now;
  }

  /** True when the event already finished in this process (and is still remembered). */
  seen(eventId: string): boolean {
    const at = this.done.get(eventId);
    if (at === undefined) return false;
    if (this.now() - at < this.ttlMs) return true;
    this.done.delete(eventId);
    return false;
  }

  /** Runs `work` once per event id; concurrent callers share the same run. */
  async run(eventId: string, work: () => Promise<void>): Promise<"processed" | "duplicate"> {
    if (this.seen(eventId)) return "duplicate";
    const pending = this.running.get(eventId);
    if (pending) {
      await pending;
      return "duplicate";
    }
    const p = work();
    this.running.set(eventId, p);
    try {
      await p;
    } finally {
      this.running.delete(eventId);
    }
    this.done.set(eventId, this.now());
    if (this.done.size > this.max) {
      // Maps iterate in insertion order: drop the oldest.
      for (const id of this.done.keys()) {
        this.done.delete(id);
        if (this.done.size <= this.max) break;
      }
    }
    return "processed";
  }
}
