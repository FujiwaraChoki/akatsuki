// Sign-in abuse limits (D-394, D-401): Better Auth keeps per-IP counters in the database, and sign-in emails are
// limited per recipient in internal.email.send whatever the caller's IP is.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { internal } from "../_generated/api";
import { authLog, createAuth, ipAddressOptions, rateLimitRules, rateLimitStorage } from "../auth";
import { makeTest, setEnv } from "./setup.helpers";

describe("sign-in limits", () => {
  beforeEach(() => setEnv({ EMAIL_TRANSPORT: "none" }));
  afterEach(() => vi.restoreAllMocks());

  it("Better Auth rate limits in the app's table, strictly on the email-sending routes", () => {
    const opts = createAuth({} as never).options;
    expect(opts.rateLimit).toMatchObject({ enabled: true });
    expect(opts.rateLimit?.customStorage?.consume).toBeTypeOf("function");
    expect(opts.advanced?.ipAddress?.ipAddressHeaders).toContain("x-forwarded-for");
    const rules = opts.rateLimit?.customRules as ReturnType<typeof rateLimitRules>;
    const withIp = new Request("https://x.convex.site/api/auth/sign-in/magic-link", {
      headers: { "x-forwarded-for": "203.0.113.7" },
    });
    const current = { window: 10, max: 3 };
    expect(rules["/sign-in/magic-link"]!(withIp, current)).toEqual({ window: 60, max: 3 });
    expect(rules["/email-otp/send-verification-otp"]!(withIp, current)).toEqual({ window: 60, max: 3 });
    expect(rules["/sign-in/email-otp"]!(withIp, current)).toEqual({ window: 60, max: 10 });
    // Every other path keeps Better Auth's own rule for that path, per IP.
    expect(rules["/**"]!(withIp, current)).toEqual(current);
    expect(Object.keys(rules).at(-1)).toBe("/**");
  });

  it("does not put clients without a resolvable IP into one shared bucket (D-401)", () => {
    const rules = rateLimitRules({});
    const current = { window: 60, max: 100 };
    const bare = new Request("http://127.0.0.1:3411/api/auth/email-otp/send-verification-otp");
    // Two hops and no trusted proxies: Better Auth cannot trust the leftmost address.
    const chain = new Request("https://x.convex.site/api/auth/email-otp/send-verification-otp", {
      headers: { "x-forwarded-for": "203.0.113.7, 10.0.0.2" },
    });
    for (const req of [bare, chain]) {
      for (const rule of Object.values(rules)) expect(rule(req, current)).toBe(false);
    }
    // With the proxy declared, the client address resolves and the limit applies again.
    const trusted = rateLimitRules({ AUTH_TRUSTED_PROXIES: "10.0.0.0/8" });
    expect(trusted["/email-otp/send-verification-otp"]!(chain, current)).toEqual({ window: 60, max: 3 });
    expect(ipAddressOptions({ AUTH_TRUSTED_PROXIES: " 10.0.0.0/8, ::1 " }).trustedProxies).toEqual([
      "10.0.0.0/8",
      "::1",
    ]);
    expect(ipAddressOptions({}).trustedProxies).toBeUndefined();
    // x-real-ip is read too.
    const real = new Request("https://x.convex.site/api/auth/sign-in/email-otp", {
      headers: { "x-real-ip": "2001:db8::1" },
    });
    expect(rules["/sign-in/email-otp"]!(real, current)).toEqual({ window: 60, max: 10 });
  });

  it("drops the sixth sign-in email to one address within an hour, others still go out", async () => {
    const t = makeTest();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const send = (to: string) =>
      t.action(internal.email.send, { template: "otp", to, vars: { otp: "123456" } });
    for (let i = 0; i < 5; i++) await send("Victim@Example.com");
    expect(warn).not.toHaveBeenCalled();
    await send("victim@example.com");
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]![0])).not.toContain("victim");
    await send("someone@example.com");
    expect(warn).toHaveBeenCalledTimes(1);
    // System emails are not limited this way.
    for (let i = 0; i < 6; i++)
      await t.action(internal.email.send, {
        template: "trial_will_end",
        to: "victim@example.com",
        vars: { endsOn: "soon" },
      });
    expect(warn).toHaveBeenCalledTimes(1);
  });
});

describe("Better Auth counters in authRateLimits (D-401)", () => {
  beforeEach(() => setEnv());

  it("counts a fixed window per key and refuses at max with the seconds left", async () => {
    const t = makeTest();
    const rule = { key: "203.0.113.7|/sign-in/magic-link", window: 60, max: 3 };
    for (let i = 0; i < 3; i++)
      expect(await t.mutation(internal.authLimits.consume, rule)).toEqual({
        allowed: true,
        retryAfter: null,
      });
    const refused = await t.mutation(internal.authLimits.consume, rule);
    expect(refused.allowed).toBe(false);
    expect(refused.retryAfter).toBeGreaterThan(0);
    expect(refused.retryAfter).toBeLessThanOrEqual(60);
    // Another client is not affected.
    expect(
      (await t.mutation(internal.authLimits.consume, { ...rule, key: "198.51.100.1|/sign-in/magic-link" }))
        .allowed,
    ).toBe(true);
    // A new window starts after the old one ends.
    await t.run(async (ctx) => {
      const row = await ctx.db
        .query("authRateLimits")
        .withIndex("by_key", (q) => q.eq("key", rule.key))
        .unique();
      await ctx.db.patch(row!._id, { lastRequest: Date.now() - 61_000 });
    });
    expect((await t.mutation(internal.authLimits.consume, rule)).allowed).toBe(true);
  });

  it("the cron sweep removes expired counters through the lastRequest index", async () => {
    const t = makeTest();
    await t.run(async (ctx) => {
      await ctx.db.insert("authRateLimits", { key: "old|/x", count: 3, lastRequest: Date.now() - 3_600_000 });
      await ctx.db.insert("authRateLimits", { key: "new|/x", count: 1, lastRequest: Date.now() });
    });
    expect(await t.mutation(internal.authLimits.gc, {})).toBe(1);
    const keys = await t.run(async (ctx) =>
      (await ctx.db.query("authRateLimits").collect()).map((r) => r.key),
    );
    expect(keys).toEqual(["new|/x"]);
  });

  it("the storage passes Better Auth's rule to the mutation from an HTTP action context", async () => {
    const calls: unknown[] = [];
    const ctx = {
      runMutation: async (_ref: unknown, args: unknown) => {
        calls.push(args);
        return { allowed: false, retryAfter: 42 };
      },
    };
    const storage = rateLimitStorage(ctx as never);
    expect(await storage.consume!("k", { window: 60, max: 3 })).toEqual({ allowed: false, retryAfter: 42 });
    expect(calls).toEqual([{ key: "k", window: 60, max: 3 }]);
    // Query contexts have no runMutation: nothing to count, the request goes through.
    expect(await rateLimitStorage({ db: {} } as never).consume!("k", { window: 60, max: 3 })).toEqual({
      allowed: true,
      retryAfter: null,
    });
  });
});

describe("Better Auth log filter", () => {
  afterEach(() => vi.restoreAllMocks());
  it("drops only the misleading shared-bucket warning", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    authLog(
      "warn",
      "Rate limiting could not determine a client IP and is falling back to a single shared bucket.",
    );
    expect(warn).not.toHaveBeenCalled();
    authLog("warn", "Something else");
    authLog("error", "Broken", 1);
    expect(warn).toHaveBeenCalledWith("[Better Auth] Something else");
    expect(error).toHaveBeenCalledWith("[Better Auth] Broken", 1);
  });
});
