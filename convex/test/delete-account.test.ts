// me.deleteAccount (§4.2, D-393, D-393): app rows, the Better Auth login and the one-trial rule.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, components, internal } from "../_generated/api";
import { asApi, asUser, entitledUser, makeTest, setEnv, type T } from "./setup.helpers";

async function authUser(t: T, email: string) {
  return await t.run(async (ctx) => {
    const now = Date.now();
    const user = (await ctx.runMutation(components.betterAuth.adapter.create, {
      input: {
        model: "user",
        data: { email, name: "Ada", emailVerified: true, createdAt: now, updatedAt: now },
      },
    })) as { _id: string };
    await ctx.runMutation(components.betterAuth.adapter.create, {
      input: {
        model: "session",
        data: {
          userId: user._id,
          token: `tok-${user._id}`,
          expiresAt: now + 86_400_000,
          createdAt: now,
          updatedAt: now,
        },
      },
    });
    await ctx.runMutation(components.betterAuth.adapter.create, {
      input: {
        model: "account",
        data: { userId: user._id, accountId: "gh-1", providerId: "github", createdAt: now, updatedAt: now },
      },
    });
    return user._id;
  });
}

async function authRows(t: T, userId: string) {
  return await t.run(async (ctx) => {
    const one = (model: "user" | "session" | "account", field: string) =>
      ctx.runQuery(components.betterAuth.adapter.findOne, {
        model,
        where: [{ field: field as never, value: userId }],
      });
    return {
      user: await one("user", "_id"),
      session: await one("session", "userId"),
      account: await one("account", "userId"),
    };
  });
}

describe("me.deleteAccount", () => {
  beforeEach(() => {
    setEnv();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("removes the Better Auth user, its sessions and linked logins with the app rows", async () => {
    const t = makeTest();
    const userId = await authUser(t, "ada@example.com");
    await entitledUser(t, userId);
    expect((await authRows(t, userId)).user).not.toBeNull();

    await asUser(t, userId, "ada@example.com").mutation(api.me.deleteAccount, { confirm: "DELETE" });
    await t.finishAllScheduledFunctions(vi.runAllTimers);

    expect(await authRows(t, userId)).toEqual({ user: null, session: null, account: null });
    const left = await t.run(async (ctx) => ({
      profile: await ctx.db
        .query("profiles")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .unique(),
      billing: await ctx.db
        .query("billing")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .unique(),
    }));
    expect(left).toEqual({ profile: null, billing: null });
  });

  it("keeps the used trial for the same email after deletion and sign-up again", async () => {
    const t = makeTest();
    const first = await authUser(t, "Ada@Example.com");
    await asUser(t, first, "Ada@Example.com").mutation(api.me.bootstrap, { timezone: "UTC" });
    await entitledUser(t, first); // active subscription: trial used
    const before = await asApi(t).query(api.ee.service.billingIdentity, { userId: first });
    expect(before.trialUsedAt).toBeTypeOf("number");

    await asUser(t, first, "ada@example.com").mutation(api.me.deleteAccount, { confirm: "DELETE" });
    await t.finishAllScheduledFunctions(vi.runAllTimers);

    // A new Better Auth user with the same address (different case).
    const second = await authUser(t, "ada@example.com");
    await asUser(t, second, "ada@example.com").mutation(api.me.bootstrap, { timezone: "UTC" });
    const again = await asApi(t).query(api.ee.service.billingIdentity, { userId: second });
    expect(again.trialUsedAt).toBe(before.trialUsedAt);
    // The ledger holds a hash, never the address.
    const ledger = await t.run(async (ctx) => await ctx.db.query("trialLedger").collect());
    expect(ledger).toHaveLength(1);
    expect(ledger[0]!.emailHash).toMatch(/^[0-9a-f]{64}$/);
    expect(JSON.stringify(ledger)).not.toContain("ada");

    // Someone else is still eligible.
    const other = await authUser(t, "grace@example.com");
    await asUser(t, other, "grace@example.com").mutation(api.me.bootstrap, { timezone: "UTC" });
    expect(
      (await asApi(t).query(api.ee.service.billingIdentity, { userId: other })).trialUsedAt,
    ).toBeUndefined();
  });

  it("Better Auth's own deleteUser purges the app rows through the onDelete trigger", async () => {
    const t = makeTest();
    const userId = await authUser(t, "lin@example.com");
    await entitledUser(t, userId);
    await t.mutation(internal.auth.onDelete, {
      model: "user",
      doc: { _id: userId, email: "lin@example.com" },
    } as never);
    await t.finishAllScheduledFunctions(vi.runAllTimers);
    const profile = await t.run(
      async (ctx) =>
        await ctx.db
          .query("profiles")
          .withIndex("by_user", (q) => q.eq("userId", userId))
          .unique(),
    );
    expect(profile).toBeNull();
  });
});
