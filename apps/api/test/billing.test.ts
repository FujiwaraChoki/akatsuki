// Stripe (§13.1, §13.2) against a stubbed fetch: the real `stripe` client builds the requests, nothing
// leaves the process, and no real key exists (sk_test_dummy is a placeholder string).
import Stripe from "stripe";
import { describe, expect, it } from "vitest";
import { DEFAULT_PRICE_LOOKUP_KEY } from "../src/config.ts";
import { createStripeBilling, PRICE_CACHE_MS, StripeEventDedupe, TRIAL_DAYS } from "../src/stripe.ts";
import { testApp } from "./helpers.ts";

interface Seen {
  method: string;
  path: string;
  query: URLSearchParams;
  form: URLSearchParams;
  auth: string | null;
}

function stubStripe(routes: Record<string, unknown>) {
  const seen: Seen[] = [];
  const fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input));
    const method = (init?.method ?? "GET").toUpperCase();
    const headers = new Headers(init?.headers);
    seen.push({
      method,
      path: url.pathname,
      query: url.searchParams,
      form: new URLSearchParams(typeof init?.body === "string" ? init.body : ""),
      auth: headers.get("authorization"),
    });
    const body = routes[`${method} ${url.pathname}`];
    if (body === undefined)
      return new Response(JSON.stringify({ error: { type: "invalid_request_error", message: "no stub" } }), {
        status: 404,
        headers: { "content-type": "application/json" },
      });
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json", "request-id": "req_test" },
    });
  }) as typeof globalThis.fetch;
  return { fetch, seen };
}

const list = (data: unknown[]) => ({ object: "list", data, has_more: false, url: "/v1/x" });

describe("Stripe client requests (stubbed fetch)", () => {
  const routes = {
    "GET /v1/prices": list([{ id: "price_pro", object: "price", lookup_key: DEFAULT_PRICE_LOOKUP_KEY }]),
    "POST /v1/checkout/sessions": {
      id: "cs_1",
      object: "checkout.session",
      url: "https://checkout.stripe.com/c/cs_1",
    },
    "POST /v1/customers": { id: "cus_new", object: "customer" },
    "POST /v1/billing_portal/sessions": {
      id: "bps_1",
      object: "billing_portal.session",
      url: "https://billing.stripe.com/p/1",
    },
    "GET /v1/subscriptions": list([
      { id: "sub_live", object: "subscription", status: "trialing" },
      { id: "sub_old", object: "subscription", status: "canceled" },
    ]),
    "POST /v1/subscriptions/sub_live": { id: "sub_live", object: "subscription", status: "trialing" },
    "DELETE /v1/subscriptions/sub_live": { id: "sub_live", object: "subscription", status: "canceled" },
  };

  it("checkout: 14-day trial, card up front, price by lookup key (cached 10 minutes)", async () => {
    let now = 1_792_483_200_000;
    const { fetch, seen } = stubStripe(routes);
    const billing = createStripeBilling({
      secretKey: "sk_test_dummy",
      webhookSecret: "whsec_dummy",
      priceLookupKey: DEFAULT_PRICE_LOOKUP_KEY,
      siteUrl: "https://app.akatsuki.vc",
      now: () => now,
      fetch,
    });
    const url = await billing.createCheckout({
      customerId: "cus_1",
      userId: "user_123",
      trialEligible: true,
    });
    expect(url).toBe("https://checkout.stripe.com/c/cs_1");

    const [prices, session] = seen;
    expect(prices).toMatchObject({ method: "GET", path: "/v1/prices" });
    expect(prices!.query.get("lookup_keys[0]")).toBe("akatsuki_pro_monthly");
    expect(prices!.query.get("active")).toBe("true");
    expect(prices!.auth).toBe("Bearer sk_test_dummy");
    expect(session).toMatchObject({ method: "POST", path: "/v1/checkout/sessions" });
    expect(Object.fromEntries(session!.form)).toEqual({
      mode: "subscription",
      customer: "cus_1",
      client_reference_id: "user_123",
      "line_items[0][price]": "price_pro",
      "line_items[0][quantity]": "1",
      payment_method_collection: "always",
      "subscription_data[trial_period_days]": String(TRIAL_DAYS),
      "subscription_data[metadata][userId]": "user_123",
      "subscription_data[trial_settings][end_behavior][missing_payment_method]": "cancel",
      success_url: "https://app.akatsuki.vc/billing/done?session_id={CHECKOUT_SESSION_ID}",
      cancel_url: "https://app.akatsuki.vc/billing",
    });
    expect(TRIAL_DAYS).toBe(14);

    // A user who already had the trial gets none; the price id is reused within 10 minutes.
    await billing.createCheckout({ customerId: "cus_1", userId: "user_123", trialEligible: false });
    expect(seen.filter((s) => s.path === "/v1/prices")).toHaveLength(1);
    const second = seen.at(-1)!.form;
    expect(second.get("subscription_data[trial_period_days]")).toBeNull();
    expect(second.get("subscription_data[metadata][userId]")).toBe("user_123");

    now += PRICE_CACHE_MS;
    await billing.createCheckout({ customerId: "cus_1", userId: "user_123", trialEligible: true });
    expect(seen.filter((s) => s.path === "/v1/prices")).toHaveLength(2);
  });

  it("fails checkout when no active price has the lookup key", async () => {
    const { fetch } = stubStripe({ ...routes, "GET /v1/prices": list([]) });
    const billing = createStripeBilling({
      secretKey: "sk_test_dummy",
      webhookSecret: "whsec_dummy",
      priceLookupKey: "missing_key",
      siteUrl: "https://app.akatsuki.vc",
      fetch,
    });
    await expect(
      billing.createCheckout({ customerId: "cus_1", userId: "u", trialEligible: true }),
    ).rejects.toThrow(/no active Stripe price/);
  });

  it("customer, portal and cancel", async () => {
    const { fetch, seen } = stubStripe(routes);
    const billing = createStripeBilling({
      secretKey: "sk_test_dummy",
      webhookSecret: "whsec_dummy",
      priceLookupKey: DEFAULT_PRICE_LOOKUP_KEY,
      siteUrl: "https://app.akatsuki.vc",
      fetch,
    });
    expect(await billing.createCustomer({ email: "sam@example.org", userId: "user_123" })).toBe("cus_new");
    expect(Object.fromEntries(seen.at(-1)!.form)).toEqual({
      email: "sam@example.org",
      "metadata[userId]": "user_123",
    });
    expect(await billing.createPortal("cus_1")).toBe("https://billing.stripe.com/p/1");
    expect(Object.fromEntries(seen.at(-1)!.form)).toEqual({
      customer: "cus_1",
      return_url: "https://app.akatsuki.vc/billing",
    });

    await billing.cancel("cus_1", false);
    const atPeriodEnd = seen.at(-1)!;
    expect(atPeriodEnd).toMatchObject({ method: "POST", path: "/v1/subscriptions/sub_live" });
    expect(atPeriodEnd.form.get("cancel_at_period_end")).toBe("true");
    await billing.cancel("cus_1", true);
    expect(seen.at(-1)).toMatchObject({ method: "DELETE", path: "/v1/subscriptions/sub_live" });
    // Already-canceled subscriptions are left alone.
    expect(seen.some((s) => s.path === "/v1/subscriptions/sub_old")).toBe(false);
    const listCall = seen.find((s) => s.path === "/v1/subscriptions")!;
    expect(listCall.query.get("customer")).toBe("cus_1");
    expect(listCall.query.get("status")).toBe("all");
  });
});

describe("Stripe webhook dedupe (§13.2)", () => {
  const WHSEC = "whsec_test_only_not_a_real_secret";
  const sign = (payload: string) =>
    new Stripe("sk_test_dummy").webhooks.generateTestHeaderString({ payload, secret: WHSEC });
  const event = (type: string, id: string) =>
    JSON.stringify({
      id,
      object: "event",
      type,
      data: { object: { customer: "cus_1", metadata: { userId: "user_123" } } },
      created: 0,
      livemode: false,
      api_version: "x",
    });
  const hook = (payload: string) =>
    new Request("http://api.test/hooks/stripe", {
      method: "POST",
      headers: { "content-type": "application/json", "stripe-signature": sign(payload) },
      body: payload,
    });

  async function setup(latest: (customerId: string) => Promise<Stripe.Subscription | null>) {
    const fetched: string[] = [];
    const billing = {
      ...createStripeBilling({
        secretKey: "sk_test_dummy",
        webhookSecret: WHSEC,
        priceLookupKey: "k",
        siteUrl: "https://app.akatsuki.vc",
      }),
      latestSubscription: async (customerId: string) => {
        fetched.push(customerId);
        return latest(customerId);
      },
    };
    return { ...(await testApp({ billing })), fetched };
  }

  it("a redelivered event is answered without re-fetching or touching Convex", async () => {
    const t = await setup(async () => null);
    const payload = event("customer.subscription.updated", "evt_dup");
    for (let i = 0; i < 3; i++) expect((await t.app.request(hook(payload))).status).toBe(200);
    expect(t.fetched).toEqual(["cus_1"]);
    expect(t.convex.applied).toHaveLength(1);
    expect(t.convex.events.has("evt_dup")).toBe(true);
    expect(t.logs.filter((l) => l.code === "evt_duplicate")).toHaveLength(2);
  });

  it("concurrent deliveries of one event share one run", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const t = await setup(async () => {
      await gate;
      return null;
    });
    const payload = event("invoice.paid", "evt_race");
    const a = t.app.request(hook(payload));
    const b = t.app.request(hook(payload));
    await new Promise((r) => setTimeout(r, 10));
    release();
    expect((await a).status).toBe(200);
    expect((await b).status).toBe(200);
    expect(t.fetched).toEqual(["cus_1"]);
    expect(t.convex.applied).toHaveLength(1);
  });

  it("a failed event is not remembered, so Stripe's retry applies it", async () => {
    let fail = true;
    const t = await setup(async () => {
      if (fail) throw new Error("Stripe is down");
      return null;
    });
    const payload = event("customer.subscription.deleted", "evt_retry");
    expect((await t.app.request(hook(payload))).status).toBe(500);
    expect(t.convex.events.has("evt_retry")).toBe(false);
    fail = false;
    expect((await t.app.request(hook(payload))).status).toBe(200);
    expect(t.fetched).toEqual(["cus_1", "cus_1"]);
    expect(t.convex.applied).toHaveLength(1);
  });

  it("the cache is bounded and forgets after its TTL", async () => {
    let now = 0;
    const d = new StripeEventDedupe({ max: 2, ttlMs: 1000, now: () => now });
    for (const id of ["a", "b", "c"]) expect(await d.run(id, async () => {})).toBe("processed");
    expect(d.seen("a")).toBe(false);
    expect(d.seen("c")).toBe(true);
    now = 1000;
    expect(d.seen("c")).toBe(false);
  });
});
