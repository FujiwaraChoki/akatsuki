// A trial that reaches trialEnd before Stripe's subscription.updated / invoice.paid webhook arrives stays entitled
// (§13.3, D-411): reevaluate at trialEnd must not fail queued ops, set lapsedAt or pause the mailbox.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, internal } from "../_generated/api";
import {
  asApi,
  asUser,
  asWorker,
  connectGmail,
  makeTest,
  message,
  opId,
  seedThread,
  setEnv,
  type T,
} from "./setup.helpers";

const DAY = 86_400_000;

async function trialUser(t: T, userId: string, trialEnd: number) {
  await asUser(t, userId).mutation(api.me.bootstrap, { timezone: "Europe/Zurich" });
  const customer = `cus_${userId}`;
  await asApi(t).mutation(api.ee.service.linkStripeCustomer, { userId, stripeCustomerId: customer });
  await asApi(t).mutation(api.ee.service.applyStripeSubscription, {
    stripeCustomerId: customer,
    userId,
    snapshot: {
      id: `sub_${userId}`,
      status: "trialing",
      priceId: "price_test",
      trialEnd,
      currentPeriodEnd: trialEnd,
      cancelAtPeriodEnd: false,
    },
    fetchedAt: Date.now(),
  });
  return customer;
}

const billingOf = (t: T, userId: string) =>
  t.run(async (ctx) =>
    ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique(),
  );

describe("trial end without a webhook yet", () => {
  beforeEach(() => {
    setEnv();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("reevaluate at trialEnd keeps the trial entitled and queued ops queued", async () => {
    const t = makeTest();
    const userId = "u-trial-end";
    const trialEnd = Date.now() + 60_000;
    await trialUser(t, userId, trialEnd);
    const { accountId } = await connectGmail(t, userId);
    const threadId = await seedThread(t, accountId, "thr-trial", [message()]);
    const res = await asUser(t, userId).mutation(api.mail.apply, {
      clientOpId: opId(41),
      threadIds: [threadId],
      action: { kind: "modify", add: [], remove: ["sys:inbox"] },
    });

    // The scheduled re-evaluation fires after trialEnd; Stripe's webhook has not arrived.
    vi.setSystemTime(trialEnd + 1_000);
    await t.mutation(internal.billing.reevaluate, { userId });

    const billing = await billingOf(t, userId);
    expect(billing?.entitlement).toMatchObject({ state: "trialing", hostedSync: true, managedAi: true });
    expect(billing?.entitlement.until).toBe(trialEnd + 3 * DAY);
    expect(billing?.lapsedAt).toBeUndefined();
    const op = await t.run((ctx) => ctx.db.get(res.opIds[0]!));
    expect(op?.state).toBe("queued");
    const assigned = await asWorker(t).query(api.worker.assignedAccounts, { shard: 0 });
    expect(assigned.find((a) => a.accountId === accountId)?.entitled).toBe(true);

    // Stripe's snapshot then moves it to active.
    await asApi(t).mutation(api.ee.service.applyStripeSubscription, {
      stripeCustomerId: `cus_${userId}`,
      userId,
      snapshot: {
        id: `sub_${userId}`,
        status: "active",
        priceId: "price_test",
        currentPeriodEnd: trialEnd + 30 * DAY,
        cancelAtPeriodEnd: false,
      },
      fetchedAt: Date.now(),
    });
    expect((await billingOf(t, userId))?.entitlement.state).toBe("active");
  });

  it("a trial with no snapshot for three days after trialEnd lapses", async () => {
    const t = makeTest();
    const userId = "u-trial-stale";
    const trialEnd = Date.now() + 60_000;
    await trialUser(t, userId, trialEnd);
    vi.setSystemTime(trialEnd + 3 * DAY + 1_000);
    await t.mutation(internal.billing.reevaluate, { userId });
    const billing = await billingOf(t, userId);
    expect(billing?.entitlement.state).toBe("lapsed");
    expect(billing?.lapsedAt).toBeDefined();
  });
});
