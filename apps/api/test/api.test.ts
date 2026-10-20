import { simulateReadableStream } from "ai";
import { MockLanguageModelV4 } from "ai/test";
import { exportJWK, generateKeyPair, SignJWT } from "jose";
import Stripe from "stripe";
import { describe, expect, it } from "vitest";
import { abortedUsage } from "../src/app.ts";
import { bearerToken, createServiceTokenSource, createWorkerVerifier } from "../src/auth.ts";
import { loadConfig } from "../src/config.ts";
import { createStripeBilling, toSnapshot } from "../src/stripe.ts";
import { audioSecondsEstimate, nextMonthStart, SlidingWindowLimiter } from "../src/util.ts";
import { flush, mockModel, SITE, testApp } from "./helpers.ts";

// biome-ignore lint/suspicious/noExplicitAny: response bodies are checked field by field in the tests.
const json = (r: Response) => r.json() as Promise<any>;

const summarizeBody = (promptVersion = 1) =>
  JSON.stringify({
    promptVersion,
    vars: {
      transcript: "From: Mia <mia@example.com>\n\nSECRET-BODY-TEXT",
      userName: "Sam",
      today: "2026-10-20",
    },
  });

const post = (path: string, token: string | null, body: string, headers: Record<string, string> = {}) =>
  new Request(`http://api.test${path}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...headers,
    },
    body,
  });

describe("GET /healthz", () => {
  it("answers ok with no-store", async () => {
    const { app } = await testApp();
    const res = await app.request("/healthz");
    expect(res.status).toBe(200);
    expect(await json(res)).toEqual({ ok: true });
    expect(res.headers.get("cache-control")).toBe("no-store");
  });
});

describe("auth rejection (§6.5)", () => {
  it("rejects missing, malformed and cookie-only credentials", async () => {
    const { app } = await testApp();
    for (const headers of [
      {},
      { authorization: "Basic abc" },
      { authorization: "Bearer" },
      { cookie: "session=1" },
    ]) {
      const res = await app.request(new Request("http://api.test/v1/ai/quota", { headers }));
      expect(res.status).toBe(401);
      expect(await json(res)).toEqual({
        error: { code: "unauthenticated", message: "Sign in to continue.", retryable: false },
      });
    }
  });

  it("rejects wrong issuer, wrong audience, expired, missing sub, foreign key and non-RS256 tokens", async () => {
    const { app, keys } = await testApp();
    const other = await generateKeyPair("RS256");
    const es = await generateKeyPair("ES256");
    const hs = new TextEncoder().encode("x".repeat(32));
    const now = Math.floor(Date.now() / 1000);
    const bad = [
      await keys.sign({}, { iss: "https://evil.example.com" }),
      await keys.sign({}, { aud: "akatsuki-service" }),
      await keys.sign({}, { exp: now - 120 }),
      await keys.sign({}, { sub: null }),
      await keys.sign({}, { key: other.privateKey }),
      await keys.sign({}, { key: es.privateKey as CryptoKey, alg: "ES256" }),
      await new SignJWT({})
        .setProtectedHeader({ alg: "HS256", kid: "k1" })
        .setIssuer(SITE)
        .setAudience("convex")
        .setSubject("u")
        .setExpirationTime(now + 60)
        .sign(hs),
      `${btoa(JSON.stringify({ alg: "none" }))}.${btoa(JSON.stringify({ iss: SITE, aud: "convex", sub: "u", exp: now + 60 }))}.`,
    ];
    for (const token of bad) {
      const res = await app.request(post("/v1/ai/summarize", token.replace(/=/g, ""), summarizeBody()));
      expect(res.status, token.slice(0, 20)).toBe(401);
    }
  });

  it("accepts a valid token within the 30 s exp leeway", async () => {
    const { app, keys } = await testApp();
    const token = await keys.sign({}, { exp: Math.floor(Date.now() / 1000) - 10 });
    const res = await app.request(
      new Request("http://api.test/v1/ai/quota", { headers: { authorization: `Bearer ${token}` } }),
    );
    expect(res.status).toBe(200);
    const body = await json(res);
    expect(body).toMatchObject({ period: "2026-10", usedMicros: 1500, limitMicros: 1_000_000 });
    expect(typeof body.resetsAt).toBe("number");
  });

  it("bearerToken parses only well-formed bearer headers", () => {
    expect(bearerToken("Bearer a.b.c")).toBe("a.b.c");
    expect(bearerToken("bearer a.b.c")).toBeNull();
    expect(bearerToken("Bearer a b")).toBeNull();
    expect(bearerToken(undefined)).toBeNull();
  });
});

describe("entitlement gating (Pro / trial)", () => {
  it("402 for lapsed users, 429 for budget and rate limits, 200 SSE for trialing users", async () => {
    const t = await testApp();
    t.convex.entitlement = { state: "lapsed", hostedSync: false, managedAi: false, until: 0 };
    let res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    expect(res.status).toBe(402);
    expect((await json(res)).error.code).toBe("not_entitled");
    expect(t.models).toEqual([]);

    t.convex.precheck = { ok: false, code: "ai_budget_exhausted", retryAfterMs: 3_600_000 };
    res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    expect(res.status).toBe(429);
    const budget = (await json(res)).error;
    expect(budget).toMatchObject({ code: "ai_budget_exhausted", retryable: false, retryAfterMs: 3_600_000 });
    expect(budget.message).toMatch(/resets at \d{4}-/);

    t.convex.precheck = { ok: false, code: "rate_limited", retryAfterMs: 2000 };
    res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    expect(res.status).toBe(429);
    expect(res.headers.get("retry-after")).toBe("2");

    t.convex.precheck = null;
    t.convex.entitlement = { state: "trialing", hostedSync: true, managedAi: true, until: Date.now() + 1e6 };
    res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toMatch(/^text\/event-stream/);
    expect(res.headers.get("cache-control")).toBe("no-store");
    const sse = await res.text();
    expect(sse).toMatch(
      /^event: meta\ndata: \{"feature":"summarize","model":"anthropic\/claude-sonnet-5.5","promptVersion":1\}/,
    );
    expect(sse).toContain("event: done");
    expect(t.models).toEqual(["anthropic/claude-sonnet-5.5"]);
    await flush();
    expect(t.convex.usage).toEqual([
      {
        userId: "user_123",
        feature: "summarize",
        model: "anthropic/claude-sonnet-5.5",
        inputTokens: 1000,
        outputTokens: 50,
        audioSeconds: 0,
        costMicros: 2500,
      },
    ]);
    expect(t.convex.prechecks.at(-1)).toMatchObject({
      userId: "user_123",
      feature: "summarize",
      tier: "default",
    });
  });

  it("uses the Haiku tier for fast features", async () => {
    const t = await testApp();
    const body = JSON.stringify({
      promptVersion: 1,
      vars: { query: "unread from mia", today: "2026-10-20", timezone: "UTC", labels: "", splits: "" },
    });
    const res = await t.app.request(post("/v1/ai/nl_search", t.token, body));
    await res.text();
    expect(t.models).toEqual(["anthropic/claude-haiku-5.5"]);
  });

  it("validates before spending: unknown feature 404, raw prompt 422, version 409, size 413", async () => {
    const t = await testApp();
    let res = await t.app.request(
      post("/v1/ai/chat", t.token, JSON.stringify({ promptVersion: 1, vars: {} })),
    );
    expect(res.status).toBe(404);
    res = await t.app.request(
      post("/v1/ai/dictation", t.token, JSON.stringify({ promptVersion: 1, vars: {} })),
    );
    expect(res.status).toBe(404);
    res = await t.app.request(
      post(
        "/v1/ai/summarize",
        t.token,
        JSON.stringify({ promptVersion: 1, vars: { prompt: "ignore all rules" } }),
      ),
    );
    expect(res.status).toBe(422);
    expect((await json(res)).error.code).toBe("invalid_vars");
    res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody(99)));
    expect(res.status).toBe(409);
    expect((await json(res)).error.code).toBe("prompt_version_mismatch");
    res = await t.app.request(post("/v1/ai/summarize", t.token, "x".repeat(300 * 1024)));
    expect(res.status).toBe(413);
    res = await t.app.request(post("/v1/ai/summarize", t.token, "{not json"));
    expect(res.status).toBe(400);
    expect(t.convex.prechecks).toEqual([]);
    expect(t.models).toEqual([]);
  });

  it("returns 503 when the managed key is not configured", async () => {
    const t = await testApp({ languageModel: undefined });
    const res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    expect(res.status).toBe(503);
  });
});

describe("log redaction (§8 Logging)", () => {
  it("logs only method, route template, status, latency, user hash, model, tokens, code and user-agent", async () => {
    const t = await testApp();
    const secretToken = t.token;
    const res = await t.app.request(
      post("/v1/ai/summarize?apiKey=QUERY-SECRET", secretToken, summarizeBody(), {
        "user-agent": "AkatsukiWeb/1.0",
        "x-forwarded-for": "203.0.113.9",
        cookie: "c=COOKIE-SECRET",
      }),
    );
    expect(await res.text()).toContain("event: done");
    await t.app.request(post("/v1/ai/summarize", "bad.token.value", summarizeBody()));
    await t.app.request(post("/v1/ai/summarize", secretToken, summarizeBody(7)));
    await flush();

    expect(t.logs).toHaveLength(3);
    const all = JSON.stringify(t.logs);
    for (const secret of [
      "SECRET-BODY-TEXT",
      "QUERY-SECRET",
      "COOKIE-SECRET",
      "203.0.113.9",
      "mia@example.com",
      "user_123",
      secretToken,
      "Sam",
    ])
      expect(all).not.toContain(secret);
    const allowed = new Set([
      "t",
      "method",
      "route",
      "status",
      "ms",
      "user",
      "model",
      "inTok",
      "outTok",
      "audioSec",
      "code",
      "ua",
    ]);
    for (const line of t.logs) for (const k of Object.keys(line)) expect(allowed.has(k), k).toBe(true);

    const [ok, unauth, mismatch] = t.logs;
    expect(ok).toMatchObject({
      method: "POST",
      route: "/v1/ai/:feature",
      status: 200,
      user: expect.stringMatching(/^[0-9a-f]{12}$/),
      model: "anthropic/claude-sonnet-5.5",
      inTok: 1000,
      outTok: 50,
      ua: "AkatsukiWeb/1.0",
    });
    expect(unauth).toMatchObject({ status: 401, code: "unauthenticated" });
    expect(unauth).not.toHaveProperty("user");
    expect(mismatch).toMatchObject({ status: 409, code: "prompt_version_mismatch" });
  });

  it("does not log provider error text", async () => {
    const t = await testApp({
      languageModel: () => {
        throw new Error("upstream leaked SECRET-PROVIDER-TEXT");
      },
    });
    const res = await t.app.request(post("/v1/ai/summarize", t.token, summarizeBody()));
    await res.text();
    await flush();
    expect(JSON.stringify(t.logs)).not.toContain("SECRET-PROVIDER-TEXT");
  });
});

describe("Stripe webhook (§13.2)", () => {
  const WHSEC = "whsec_test_only_not_a_real_secret";
  const sub = {
    id: "sub_1",
    object: "subscription",
    status: "trialing",
    cancel_at_period_end: false,
    trial_end: 1_800_000_000,
    metadata: { userId: "user_123" },
    items: { data: [{ price: { id: "price_1" }, current_period_end: 1_800_000_000 }] },
  } as unknown as Stripe.Subscription;

  const setup = async () => {
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
        return sub;
      },
    };
    const t = await testApp({ billing });
    return { ...t, fetched };
  };
  const event = (type: string, object: Record<string, unknown>, id = "evt_1") =>
    JSON.stringify({
      id,
      object: "event",
      type,
      data: { object },
      created: 0,
      livemode: false,
      api_version: "x",
    });
  const sign = (payload: string, secret = WHSEC) =>
    new Stripe("sk_test_dummy").webhooks.generateTestHeaderString({ payload, secret });

  it("rejects a missing or invalid signature without touching Convex", async () => {
    const t = await setup();
    const payload = event("customer.subscription.updated", { customer: "cus_1" });
    const variants: Record<string, string>[] = [
      {},
      { "stripe-signature": "t=1,v1=deadbeef" },
      { "stripe-signature": sign(payload, "whsec_wrong") },
    ];
    for (const headers of variants) {
      const res = await t.app.request(post("/hooks/stripe", null, payload, headers));
      expect(res.status).toBe(401);
    }
    // A valid signature over a different body fails too.
    const res = await t.app.request(
      post("/hooks/stripe", null, payload.replace("cus_1", "cus_2"), { "stripe-signature": sign(payload) }),
    );
    expect(res.status).toBe(401);
    expect(t.convex.applied).toEqual([]);
    expect(t.convex.events.size).toBe(0);
  });

  it("re-fetches the subscription and applies the snapshot, then records the event", async () => {
    const t = await setup();
    const payload = event("checkout.session.completed", {
      customer: "cus_1",
      client_reference_id: "user_123",
    });
    const res = await t.app.request(
      post("/hooks/stripe", null, payload, { "stripe-signature": sign(payload) }),
    );
    expect(res.status).toBe(200);
    expect(t.fetched).toEqual(["cus_1"]);
    expect(t.convex.applied).toEqual([
      {
        stripeCustomerId: "cus_1",
        userId: "user_123",
        snapshot: {
          id: "sub_1",
          status: "trialing",
          priceId: "price_1",
          trialEnd: 1_800_000_000_000,
          currentPeriodEnd: 1_800_000_000_000,
          cancelAtPeriodEnd: false,
        },
        fetchedAt: expect.any(Number),
      },
    ]);
    expect(t.convex.events.has("evt_1")).toBe(true);
  });

  it("sends the trial reminder once per event id", async () => {
    const t = await setup();
    const payload = event("customer.subscription.trial_will_end", { customer: "cus_9" }, "evt_trial");
    for (let i = 0; i < 2; i++)
      expect(
        (await t.app.request(post("/hooks/stripe", null, payload, { "stripe-signature": sign(payload) })))
          .status,
      ).toBe(200);
    expect(t.convex.trialEnds).toEqual(["cus_9"]);
  });

  it("toSnapshot maps unknown statuses to not entitled", () => {
    expect(toSnapshot({ ...sub, status: "future_status" } as unknown as Stripe.Subscription).status).toBe(
      "incomplete",
    );
  });
});

describe("billing routes", () => {
  it("checkout creates the customer once, offers the trial only once, and refuses subscribed users", async () => {
    const created: unknown[] = [];
    const billing = {
      constructEvent: () => {
        throw new Error("unused");
      },
      latestSubscription: async () => null,
      createCustomer: async () => "cus_new",
      createCheckout: async (a: unknown) => {
        created.push(a);
        return "https://checkout.stripe.com/c/test";
      },
      createPortal: async () => "https://billing.stripe.com/p/test",
      cancel: async () => {},
    };
    const t = await testApp({ billing });
    t.convex.entitlement = { state: "none", hostedSync: false, managedAi: false, until: 0 };
    let res = await t.app.request(post("/v1/billing/checkout", t.token, "{}"));
    expect(await json(res)).toEqual({ url: "https://checkout.stripe.com/c/test" });
    expect(t.convex.stripeCustomerId).toBe("cus_new");
    expect(created).toEqual([{ customerId: "cus_new", userId: "user_123", trialEligible: true }]);
    t.convex.trialUsedAt = 1;
    await t.app.request(post("/v1/billing/checkout", t.token, "{}"));
    expect(created.at(-1)).toMatchObject({ trialEligible: false });
    t.convex.entitlement = { state: "active", hostedSync: true, managedAi: true, until: 1 };
    res = await t.app.request(post("/v1/billing/checkout", t.token, "{}"));
    expect(res.status).toBe(409);
    res = await t.app.request(post("/v1/billing/portal", t.token, "{}"));
    expect(await json(res)).toEqual({ url: "https://billing.stripe.com/p/test" });
    res = await t.app.request(post("/v1/billing/cancel", t.token, JSON.stringify({ immediately: true })));
    expect(res.status).toBe(204);
  });

  it("billing routes are absent when AKATSUKI_BILLING=off", async () => {
    const t = await testApp({ config: { corsOrigins: [], billing: false } });
    expect((await t.app.request(post("/v1/billing/checkout", t.token, "{}"))).status).toBe(404);
    expect((await t.app.request(post("/hooks/stripe", null, "{}", { "stripe-signature": "x" }))).status).toBe(
      404,
    );
  });
});

describe("CORS", () => {
  it("allows only exact configured origins", async () => {
    const { app } = await testApp();
    const pre = (origin: string) =>
      app.request(
        new Request("http://api.test/v1/ai/summarize", {
          method: "OPTIONS",
          headers: { origin, "access-control-request-method": "POST" },
        }),
      );
    expect((await pre("https://app.akatsuki.vc")).headers.get("access-control-allow-origin")).toBe(
      "https://app.akatsuki.vc",
    );
    expect((await pre("http://localhost:5173")).headers.get("access-control-allow-origin")).toBe(
      "http://localhost:5173",
    );
    for (const o of ["https://evil.example.com", "https://app.akatsuki.vc.evil.example", "null"])
      expect((await pre(o)).headers.get("access-control-allow-origin")).toBeNull();
  });
});

describe("transcription upload", () => {
  const wav = (seconds: number) => {
    const rate = 8000;
    const data = rate * 2 * seconds;
    const b = new Uint8Array(44 + data);
    const v = new DataView(b.buffer);
    b.set(new TextEncoder().encode("RIFF"), 0);
    v.setUint32(4, 36 + data, true);
    b.set(new TextEncoder().encode("WAVEfmt "), 8);
    v.setUint32(16, 16, true);
    v.setUint16(20, 1, true);
    v.setUint16(22, 1, true);
    v.setUint32(24, rate, true);
    v.setUint32(28, rate * 2, true);
    v.setUint16(32, 2, true);
    v.setUint16(34, 16, true);
    b.set(new TextEncoder().encode("data"), 36);
    v.setUint32(40, data, true);
    return b;
  };

  it("transcribes an upload, meters audio seconds and checks inputs", async () => {
    const { MockTranscriptionModelV4 } = await import("ai/test");
    const t = await testApp({
      transcriptionModel: () =>
        new MockTranscriptionModelV4({
          doGenerate: async () => ({
            text: "hello",
            segments: [],
            language: "en",
            durationInSeconds: undefined,
            warnings: [],
            response: { timestamp: new Date(0), modelId: "gpt-4o-transcribe" },
          }),
        }),
    });
    const form = (file: Blob | null, extra: Record<string, string> = {}) => {
      const f = new FormData();
      if (file) f.set("audio", file, "a.wav");
      for (const [k, v] of Object.entries(extra)) f.set(k, v);
      return new Request("http://api.test/v1/ai/transcribe", {
        method: "POST",
        headers: { authorization: `Bearer ${t.token}` },
        body: f,
      });
    };
    let res = await t.app.request(form(new Blob([wav(3)], { type: "audio/wav" }), { language: "en-GB" }));
    expect(res.status).toBe(200);
    expect(await json(res)).toEqual({ text: "hello", durationSec: 3, language: "en" });
    expect(t.convex.usage.at(-1)).toMatchObject({
      feature: "dictation",
      model: "gpt-4o-transcribe",
      audioSeconds: 3,
      costMicros: 300,
    });

    res = await t.app.request(form(new Blob([wav(1)], { type: "text/plain" })));
    expect(res.status).toBe(400);
    res = await t.app.request(form(null));
    expect(res.status).toBe(400);
    res = await t.app.request(form(new Blob([wav(1)], { type: "audio/wav" }), { prompt: "p".repeat(501) }));
    expect(res.status).toBe(400);
    res = await t.app.request(form(new Blob([wav(601)], { type: "audio/wav" })));
    expect(res.status).toBe(413);
  });

  it("enforces the 120 min/day dictation cap from the Convex daily bucket (D-343)", async () => {
    const { MockTranscriptionModelV4 } = await import("ai/test");
    const calls: unknown[] = [];
    const t = await testApp({
      transcriptionModel: () =>
        new MockTranscriptionModelV4({
          doGenerate: async () => {
            calls.push(1);
            return {
              text: "x",
              segments: [],
              language: undefined,
              durationInSeconds: 7,
              warnings: [],
              response: { timestamp: new Date(0), modelId: "gpt-4o-transcribe" },
            };
          },
        }),
    });
    const upload = (seconds: number) => {
      const f = new FormData();
      f.set("audio", new Blob([wav(seconds)], { type: "audio/wav" }), "a.wav");
      return new Request("http://api.test/v1/ai/transcribe", {
        method: "POST",
        headers: { authorization: `Bearer ${t.token}` },
        body: f,
      });
    };
    // The precheck carries the estimated length so the bucket can refuse before OpenAI is called.
    let res = await t.app.request(upload(5));
    expect(res.status).toBe(200);
    expect(t.convex.prechecks.at(-1)).toMatchObject({
      feature: "dictation",
      tier: "transcription",
      audioSeconds: 5,
    });
    // The provider's duration is what gets recorded (and taken from the bucket in Convex).
    expect(t.convex.usage.at(-1)).toMatchObject({ feature: "dictation", audioSeconds: 7 });

    t.convex.precheck = {
      ok: false,
      code: "rate_limited",
      retryAfterMs: 3_600_000,
      limit: "transcription_day",
    };
    res = await t.app.request(upload(5));
    expect(res.status).toBe(429);
    expect(res.headers.get("retry-after")).toBe("3600");
    const err = (await json(res)).error;
    expect(err).toMatchObject({ code: "rate_limited", retryable: true, retryAfterMs: 3_600_000 });
    expect(err.message).toMatch(/limited to 120 minutes a day\. It resets at \d{4}-/);
    expect(calls).toHaveLength(1);
    await flush();
    expect(t.logs.at(-1)).toMatchObject({ status: 429, code: "transcription_day_cap" });
  });

  it("estimates audio length", () => {
    expect(audioSecondsEstimate(new Uint8Array(40_000), "audio/webm")).toEqual({ seconds: 10, exact: false });
  });
});

describe("worker-only triage (D-048)", () => {
  const items = [
    {
      threadId: "j57abc",
      from: { email: "mia@example.com" },
      to: [],
      subject: "Head count",
      snippet: "Confirm?",
      isBulk: false,
      hasAttachments: false,
    },
  ];

  it("requires a worker service token with aud akatsuki-api", async () => {
    const { publicKey, privateKey } = await generateKeyPair("ES256");
    const jwks = JSON.stringify({ keys: [{ ...(await exportJWK(publicKey)), kid: "w1", alg: "ES256" }] });
    const verifyWorker = createWorkerVerifier({
      workerServiceJwks: jwks,
      workerIssuer: "https://sync.akatsuki.vc",
    });
    const answer = JSON.stringify({ items: [{ key: "1", category: "important", needsReply: true }] });
    const t = await testApp({ verifyWorker, languageModel: () => mockModel([answer]).model });
    const mint = (aud: string, sub = "worker:1") =>
      new SignJWT({})
        .setProtectedHeader({ alg: "ES256", kid: "w1" })
        .setIssuer("https://sync.akatsuki.vc")
        .setAudience(aud)
        .setSubject(sub)
        .setIssuedAt()
        .setExpirationTime("10m")
        .sign(privateKey);
    const body = JSON.stringify({ userId: "user_123", items });

    expect((await t.app.request(post("/internal/v1/triage", t.token, body))).status).toBe(401); // a user JWT
    expect(
      (await t.app.request(post("/internal/v1/triage", await mint("akatsuki-service"), body))).status,
    ).toBe(401);
    expect(
      (await t.app.request(post("/internal/v1/triage", await mint("akatsuki-api", "api"), body))).status,
    ).toBe(401);
    const res = await t.app.request(post("/internal/v1/triage", await mint("akatsuki-api"), body));
    expect(res.status).toBe(200);
    expect(await json(res)).toEqual({
      results: [
        {
          threadId: "j57abc",
          category: "important",
          needsReply: true,
          model: "anthropic/claude-haiku-5.5",
          promptVersion: 1,
        },
      ],
    });
    expect(t.convex.usage.at(-1)).toMatchObject({
      userId: "user_123",
      feature: "triage",
      model: "anthropic/claude-haiku-5.5",
    });
    const tooMany = JSON.stringify({ userId: "u", items: Array.from({ length: 26 }, () => items[0]) });
    expect(
      (await t.app.request(post("/internal/v1/triage", await mint("akatsuki-api"), tooMany))).status,
    ).toBe(400);
  });

  it("is not served when WORKER_SERVICE_JWKS is not configured", async () => {
    const t = await testApp();
    expect((await t.app.request(post("/internal/v1/triage", "x.y.z", "{}"))).status).toBe(404);
  });
});

describe("service identity and config", () => {
  it("mints ES256 service tokens for Convex (iss API_ISSUER, aud akatsuki-service, sub api, 10 min)", async () => {
    const { privateKey, publicKey } = await generateKeyPair("ES256", { extractable: true });
    const { exportPKCS8, jwtVerify } = await import("jose");
    let now = 1_792_483_200_000;
    const source = createServiceTokenSource({
      privateKeyPem: await exportPKCS8(privateKey),
      kid: "api-1",
      issuer: "https://api.akatsuki.vc",
      now: () => now,
    });
    const token = await source();
    const { payload, protectedHeader } = await jwtVerify(token, publicKey, {
      currentDate: new Date(now),
      issuer: "https://api.akatsuki.vc",
      audience: "akatsuki-service",
    });
    expect(protectedHeader).toMatchObject({ alg: "ES256", kid: "api-1" });
    expect(payload.sub).toBe("api");
    expect(payload.exp! - payload.iat!).toBe(600);
    expect(await source()).toBe(token);
    now += 5 * 60 * 1000 + 1;
    expect(await source()).not.toBe(token);
  });

  it("loadConfig reports missing variables by name only", () => {
    expect(() =>
      loadConfig({ CONVEX_URL: "https://x.convex.cloud", API_SERVICE_PRIVATE_KEY: "SECRET-VALUE" }),
    ).toThrow(/CONVEX_SITE_URL, API_SERVICE_KID/);
    try {
      loadConfig({ API_SERVICE_PRIVATE_KEY: "SECRET-VALUE" });
    } catch (e) {
      expect(String(e)).not.toContain("SECRET-VALUE");
    }
    const c = loadConfig({
      CONVEX_URL: "https://x.convex.cloud",
      CONVEX_SITE_URL: "https://x.convex.site/",
      API_SERVICE_PRIVATE_KEY: "k",
      API_SERVICE_KID: "kid",
      BIND_ADDR: "0.0.0.0:8080",
      AKATSUKI_BILLING: "off",
    });
    expect(c).toMatchObject({
      convexSiteUrl: "https://x.convex.site",
      port: 8080,
      host: "0.0.0.0",
      billing: false,
    });
    expect(c.corsOrigins).toContain("https://app.akatsuki.vc");
  });

  it("helpers: month boundary and sliding window", () => {
    expect(nextMonthStart(Date.UTC(2026, 11, 31, 23))).toBe(Date.UTC(2027, 0, 1));
    let now = 0;
    const l = new SlidingWindowLimiter(2, 1000, () => now);
    expect(l.take("a").ok).toBe(true);
    expect(l.take("a").ok).toBe(true);
    expect(l.take("a")).toEqual({ ok: false, retryAfterMs: 1000 });
    now = 1000;
    expect(l.take("a").ok).toBe(true);
  });
});

describe("managed AI charges what was streamed (D-406)", () => {
  const composeBody = JSON.stringify({
    promptVersion: 1,
    vars: { instruction: "hi", transcript: null, draft: null, voice: null, userName: "Sam" },
  });
  // 40 deltas of 100 characters, then a finish with usage that the client never waits for.
  const slowModel = (failAfter?: number) =>
    new MockLanguageModelV4({
      doStream: async () => ({
        stream: simulateReadableStream({
          initialDelayInMs: 0,
          chunkDelayInMs: 5,
          chunks: [
            { type: "text-start", id: "t" },
            ...Array.from({ length: 40 }, (_, i) =>
              failAfter !== undefined && i === failAfter
                ? ({ type: "error", error: new Error("upstream reset") } as const)
                : ({ type: "text-delta", id: "t", delta: "x".repeat(100) } as const),
            ),
            { type: "text-end", id: "t" },
            {
              type: "finish",
              finishReason: { unified: "stop", raw: undefined },
              logprobs: undefined,
              usage: {
                inputTokens: { total: 10, noCache: 10, cacheRead: undefined, cacheWrite: undefined },
                outputTokens: { total: 1000, text: 1000, reasoning: undefined },
              },
            },
          ] as never,
        }),
      }),
    });

  it("a client that disconnects before done is charged for the deltas it read", async () => {
    const t = await testApp({ languageModel: () => slowModel() });
    const res = await t.app.request(post("/v1/ai/compose", t.token, composeBody));
    expect(res.status).toBe(200);
    const reader = res.body!.getReader();
    const decoder = new TextDecoder();
    let seen = "";
    while ((seen.match(/event: delta/g) ?? []).length < 20) {
      const { value, done } = await reader.read();
      if (done) break;
      seen += decoder.decode(value);
    }
    await reader.cancel();
    await flush();
    await flush();
    expect(t.convex.usage).toHaveLength(1);
    const u = t.convex.usage[0]!;
    // At least the 20 deltas read (20 × 100 characters ≈ 500 tokens), never 0 output.
    expect(u.outputTokens).toBeGreaterThanOrEqual(500);
    expect(u.inputTokens).toBeGreaterThan(0);
    expect(u.costMicros).toBeGreaterThan(0);
  });

  it("a provider error part mid-stream charges the usage the provider still reported", async () => {
    const t = await testApp({ languageModel: () => slowModel(10) });
    const res = await t.app.request(post("/v1/ai/compose", t.token, composeBody));
    const sse = await res.text();
    expect(sse).toContain("event: error");
    await flush();
    expect(t.convex.usage).toHaveLength(1);
    expect(t.convex.usage[0]!).toMatchObject({ inputTokens: 10, outputTokens: 1000 });
  });

  it("a stream that dies mid-way charges the streamed output (estimated)", async () => {
    const dying = new MockLanguageModelV4({
      doStream: async () => {
        let i = 0;
        return {
          stream: new ReadableStream({
            pull(controller) {
              if (i === 0) controller.enqueue({ type: "text-start", id: "t" });
              else if (i <= 10) controller.enqueue({ type: "text-delta", id: "t", delta: "y".repeat(100) });
              else controller.error(new Error("connection reset"));
              i++;
            },
          }) as never,
        };
      },
    });
    const t = await testApp({ languageModel: () => dying });
    const res = await t.app.request(post("/v1/ai/compose", t.token, composeBody));
    const sse = await res.text();
    expect(sse).toContain("event: error");
    await flush();
    expect(t.convex.usage).toHaveLength(1);
    expect(t.convex.usage[0]!.outputTokens).toBe(250); // 10 deltas × 100 characters / 4
  });

  it("estimates about four characters a token", () => {
    expect(abortedUsage(0, 0)).toEqual({ inputTokens: 0, outputTokens: 0 });
    expect(abortedUsage(9, 2001)).toEqual({ inputTokens: 3, outputTokens: 501 });
  });
});
