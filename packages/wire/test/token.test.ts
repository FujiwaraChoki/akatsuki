// getAccessToken (§6.2 step 4): memory cache, refresh 60 s before exp, one in-flight mint.
import { afterEach, describe, expect, it } from "vitest";
import {
  clearAccessToken,
  createAccessTokenSource,
  getAccessToken,
  jwtExpiresAt,
  setAccessTokenSource,
  WireError,
} from "../src/index.ts";

const b64url = (s: string) => Buffer.from(s).toString("base64url");
const jwt = (exp: number, extra: Record<string, unknown> = {}) =>
  `${b64url('{"alg":"RS256"}')}.${b64url(JSON.stringify({ exp, sub: "u1", ...extra }))}.sig`;

afterEach(() => setAccessTokenSource(null));

describe("jwtExpiresAt", () => {
  it("reads exp in ms without verifying", () => {
    expect(jwtExpiresAt(jwt(1_800_000_000))).toBe(1_800_000_000_000);
    expect(jwtExpiresAt(jwt(1_800_000_000, { name: "Zoë 東京" }))).toBe(1_800_000_000_000);
    expect(jwtExpiresAt("not-a-jwt")).toBeNull();
    expect(jwtExpiresAt("a.!!!.c")).toBeNull();
  });
});

describe("createAccessTokenSource", () => {
  it("caches until 60 s before exp, then refreshes", async () => {
    let t = 1_000_000_000_000;
    let n = 0;
    const src = createAccessTokenSource(
      async () => {
        n++;
        return jwt(t / 1000 + 15 * 60, { n });
      },
      { now: () => t },
    );
    const a = await src.getAccessToken();
    expect(await src.getAccessToken()).toBe(a);
    t += 13 * 60_000;
    expect(await src.getAccessToken()).toBe(a);
    t += 61_000; // 59 s before exp
    const b = await src.getAccessToken();
    expect(b).not.toBe(a);
    expect(n).toBe(2);
    expect(await src.getAccessToken({ forceRefresh: true })).not.toBe(b);
    expect(n).toBe(3);
  });

  it("shares one in-flight request", async () => {
    let n = 0;
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const src = createAccessTokenSource(async () => {
      n++;
      await gate;
      return jwt(Date.now() / 1000 + 900);
    });
    const p = Promise.all([src.getAccessToken(), src.getAccessToken(), src.getAccessToken()]);
    release();
    const [a, b, c] = await p;
    expect(a).toBe(b);
    expect(b).toBe(c);
    expect(n).toBe(1);
  });

  it("wraps a Better Auth client and throws unauthenticated when signed out", async () => {
    let token: string | null = jwt(Date.now() / 1000 + 900);
    const calls: unknown[] = [];
    const src = createAccessTokenSource({
      convex: {
        token: async (opts) => {
          calls.push(opts);
          return { data: token ? { token } : null };
        },
      },
    });
    expect(await src.getAccessToken()).toBe(token);
    expect(calls).toEqual([{ fetchOptions: { throw: false } }]);
    token = null;
    src.clear();
    await expect(src.getAccessToken()).rejects.toMatchObject({ code: "unauthenticated" });
    await expect(src.getAccessToken()).rejects.toBeInstanceOf(WireError);
  });

  it("maps a failing mint to unauthenticated", async () => {
    const src = createAccessTokenSource(async () => {
      throw new Error("network");
    });
    await expect(src.getAccessToken()).rejects.toMatchObject({ code: "unauthenticated" });
  });
});

describe("getAccessToken", () => {
  it("needs a configured source", async () => {
    await expect(getAccessToken()).rejects.toMatchObject({ code: "unauthenticated" });
    setAccessTokenSource(async () => "tok.en.x");
    expect(await getAccessToken()).toBe("tok.en.x");
    clearAccessToken();
  });
});

describe("setAccessTokenSource (D-386)", () => {
  it("treats a Better Auth-style Proxy client as a fetcher, not as a ready source", async () => {
    // Better Auth's client is a Proxy that answers every property, so `"getAccessToken" in client` is true.
    const client = new Proxy(() => undefined, {
      has: () => true,
      get: (_t, key) =>
        key === "convex" ? { token: async () => ({ data: { token: "client.jwt.x" } }) } : () => undefined,
    });
    setAccessTokenSource(client as unknown as Parameters<typeof setAccessTokenSource>[0]);
    expect(await getAccessToken()).toBe("client.jwt.x");
  });

  it("accepts a ready source, branded or a plain object", async () => {
    setAccessTokenSource(createAccessTokenSource(async () => "branded.jwt.x"));
    expect(await getAccessToken()).toBe("branded.jwt.x");
    setAccessTokenSource({ getAccessToken: async () => "plain.jwt.x", clear: () => undefined });
    expect(await getAccessToken()).toBe("plain.jwt.x");
  });
});
