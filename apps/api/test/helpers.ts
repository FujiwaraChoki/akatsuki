// Test doubles: a JWKS with a throwaway RSA key, an in-memory Convex gateway, mock AI models. No network.
import { simulateReadableStream } from "ai";
import { MockLanguageModelV4 } from "ai/test";
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT } from "jose";
import { type AppDeps, createApp } from "../src/app.ts";
import { createUserVerifier } from "../src/auth.ts";
import type { ConvexGateway, Entitlement, PrecheckResult, UsageRecord } from "../src/convex.ts";
import type { LogLine } from "../src/log.ts";

export const SITE = "https://test-deployment.convex.site";

export async function userKeys() {
  const { publicKey, privateKey } = await generateKeyPair("RS256");
  const jwk = { ...(await exportJWK(publicKey)), kid: "k1", alg: "RS256" };
  const jwks = createLocalJWKSet({ keys: [jwk] });
  const sign = (
    claims: Record<string, unknown> = {},
    opts: {
      iss?: string;
      aud?: string;
      exp?: number;
      sub?: string | null;
      key?: CryptoKey;
      alg?: string;
    } = {},
  ) => {
    const now = Math.floor(Date.now() / 1000);
    const jwt = new SignJWT({ email: "sam@example.org", ...claims })
      .setProtectedHeader({ alg: opts.alg ?? "RS256", kid: "k1" })
      .setIssuer(opts.iss ?? SITE)
      .setAudience(opts.aud ?? "convex")
      .setIssuedAt(now)
      .setExpirationTime(opts.exp ?? now + 900);
    if (opts.sub !== null) jwt.setSubject(opts.sub ?? "user_123");
    return jwt.sign(opts.key ?? privateKey);
  };
  return { jwks, sign, verifyUser: createUserVerifier({ convexSiteUrl: SITE, jwks }) };
}

export class FakeConvex implements ConvexGateway {
  entitlement: Entitlement = {
    state: "trialing",
    hostedSync: true,
    managedAi: true,
    until: Date.now() + 1e9,
  };
  precheck: PrecheckResult | null = null;
  usage: UsageRecord[] = [];
  prechecks: unknown[] = [];
  events = new Set<string>();
  applied: unknown[] = [];
  trialEnds: string[] = [];
  stripeCustomerId: string | undefined;
  trialUsedAt: number | undefined;

  async aiPrecheck(a: { userId: string; feature: string; tier: string; estCostMicros: number }) {
    this.prechecks.push(a);
    if (this.precheck) return this.precheck;
    return this.entitlement.managedAi
      ? ({ ok: true, remainingMicros: 1_000_000 } as const)
      : ({ ok: false, code: "not_entitled" } as const);
  }
  async aiRecordUsage(a: UsageRecord) {
    this.usage.push(a);
    return { remainingMicros: 1 };
  }
  async billingIdentity() {
    return {
      email: "sam@example.org",
      entitlement: this.entitlement,
      ...(this.stripeCustomerId ? { stripeCustomerId: this.stripeCustomerId } : {}),
      ...(this.trialUsedAt ? { trialUsedAt: this.trialUsedAt } : {}),
    };
  }
  async linkStripeCustomer(a: { stripeCustomerId: string }) {
    this.stripeCustomerId = a.stripeCustomerId;
    return null;
  }
  async recordStripeEvent(a: { eventId: string }) {
    const duplicate = this.events.has(a.eventId);
    this.events.add(a.eventId);
    return { duplicate };
  }
  async applyStripeSubscription(a: unknown) {
    this.applied.push(a);
    return null;
  }
  async trialWillEnd(a: { stripeCustomerId: string }) {
    this.trialEnds.push(a.stripeCustomerId);
    return null;
  }
  async userAiUsage() {
    return { period: "2026-10", usedMicros: 1500, limitMicros: 1_000_000 };
  }
}

export function mockModel(pieces: string[]) {
  const calls: unknown[] = [];
  const model = new MockLanguageModelV4({
    doStream: async (options) => {
      calls.push(options);
      return {
        stream: simulateReadableStream({
          chunks: [
            { type: "text-start", id: "t" },
            ...pieces.map((delta) => ({ type: "text-delta" as const, id: "t", delta })),
            { type: "text-end", id: "t" },
            {
              type: "finish",
              finishReason: { unified: "stop", raw: undefined },
              logprobs: undefined,
              usage: {
                inputTokens: { total: 1000, noCache: 1000, cacheRead: undefined, cacheWrite: undefined },
                outputTokens: { total: 50, text: 50, reasoning: undefined },
              },
            },
          ],
        }),
      };
    },
  });
  return { model, calls };
}

export async function testApp(overrides: Partial<AppDeps> = {}) {
  const keys = await userKeys();
  const convex = new FakeConvex();
  const logs: LogLine[] = [];
  const models: string[] = [];
  const app = createApp({
    config: { corsOrigins: ["https://app.akatsuki.vc", "http://localhost:5173"], billing: true },
    verifyUser: keys.verifyUser,
    convex,
    languageModel: (id) => {
      models.push(id);
      return mockModel(['{"tldr":"t","bullets":[],"asks":[],"deadlines":[]}']).model;
    },
    log: (l) => logs.push(l),
    ...overrides,
  });
  return { app, convex, logs, models, keys, token: await keys.sign() };
}

export const flush = () => new Promise((r) => setTimeout(r, 20));
