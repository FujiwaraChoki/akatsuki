// Mobile sign-in handoff (D-408): the akatsuki:// redirect carries only a single-use code bound to the app's
// verifier; Better Auth refuses custom-scheme callbacks, so the session cookie never travels in one.
import { beforeEach, describe, expect, it } from "vitest";
import { internal } from "../_generated/api";
import { rejectSchemeCallbacks } from "../auth";
import { HANDOFF_TTL_MS, s256 } from "../mobileAuth";
import { makeTest, setEnv } from "./setup.helpers";

const VERIFIER = "v".repeat(43);

describe("mobile sign-in handoff", () => {
  beforeEach(() => setEnv());

  it("redirects to the app with a code, never the one-time token", async () => {
    const t = makeTest();
    const challenge = await s256(VERIFIER);
    const res = await t.fetch(`/mobile/auth/handoff?challenge=${challenge}&ott=one-time-token-value`, {
      redirect: "manual",
    } as RequestInit);
    expect(res.status).toBe(302);
    const location = res.headers.get("location") ?? "";
    expect(location).toMatch(/^akatsuki:\/\/auth\/callback\?code=[A-Za-z0-9_-]{43}$/);
    expect(location).not.toContain("one-time-token-value");
    const rows = await t.run(async (ctx) => await ctx.db.query("authHandoffs").collect());
    expect(rows).toHaveLength(1);
    // Only the hash of the code is stored.
    const code = new URL(location).searchParams.get("code")!;
    expect(rows[0]!.codeHash).toBe(await s256(code));
    expect(rows[0]!.expiresAt - Date.now()).toBeLessThanOrEqual(HANDOFF_TTL_MS);
  });

  it("sends errors and malformed requests to the app as an error, storing nothing", async () => {
    const t = makeTest();
    for (const q of [
      "?error=access_denied",
      "?challenge=short&ott=x",
      `?challenge=${await s256(VERIFIER)}`,
    ]) {
      const res = await t.fetch(`/mobile/auth/handoff${q}`, { redirect: "manual" } as RequestInit);
      expect(res.headers.get("location")).toBe("akatsuki://auth/callback?error=expired");
    }
    expect(await t.run(async (ctx) => await ctx.db.query("authHandoffs").collect())).toEqual([]);
  });

  it("a code is redeemable once, only with the verifier, and burns on a wrong one", async () => {
    const t = makeTest();
    const challenge = await s256(VERIFIER);
    const put = async (code: string) =>
      await t.mutation(internal.mobileAuth.createHandoff, {
        codeHash: await s256(code),
        challenge,
        ott: "ott",
      });
    const take = async (code: string, verifier: string) =>
      await t.mutation(internal.mobileAuth.takeHandoff, {
        codeHash: await s256(code),
        challenge: await s256(verifier),
      });
    await put("code-1");
    expect(await take("code-1", VERIFIER)).toEqual({ ott: "ott" });
    expect(await take("code-1", VERIFIER)).toBeNull();
    // An intercepted code without the verifier is refused and gone.
    await put("code-2");
    expect(await take("code-2", "w".repeat(43))).toBeNull();
    expect(await take("code-2", VERIFIER)).toBeNull();
    // Expired.
    await put("code-3");
    await t.run(async (ctx) => {
      const row = (await ctx.db.query("authHandoffs").collect())[0]!;
      await ctx.db.patch(row._id, { expiresAt: Date.now() - 1 });
    });
    expect(await take("code-3", VERIFIER)).toBeNull();
  });

  it("redeem refuses malformed bodies and unknown codes without touching Better Auth", async () => {
    const t = makeTest();
    for (const body of [
      "{",
      JSON.stringify({ code: 1 }),
      JSON.stringify({ code: "a".repeat(43), verifier: "short" }),
    ]) {
      const res = await t.fetch("/mobile/auth/redeem", { method: "POST", body });
      expect(res.status).toBe(400);
    }
    const res = await t.fetch("/mobile/auth/redeem", {
      method: "POST",
      body: JSON.stringify({ code: "a".repeat(43), verifier: VERIFIER }),
    });
    expect(res.status).toBe(400);
  });

  it("Better Auth refuses custom-scheme sign-in callbacks", () => {
    expect(() => rejectSchemeCallbacks({ callbackURL: "akatsuki://auth/callback" })).toThrow(
      /https or a path/,
    );
    expect(() => rejectSchemeCallbacks(undefined, { errorCallbackURL: "exp://x" })).toThrow();
    expect(() => rejectSchemeCallbacks({ callbackURL: "javascript:alert(1)" })).toThrow();
    rejectSchemeCallbacks({ callbackURL: "https://app.akatsuki.vc", newUserCallbackURL: "/welcome" });
    rejectSchemeCallbacks({ callbackURL: "http://127.0.0.1:5173/" }, { email: "x" });
  });
});
