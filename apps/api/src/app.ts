// apps/api routes (ARCHITECTURE §8). Stateless: nothing here persists or logs request or response content.
import {
  type AaspEvent,
  aaspReadableStream,
  audioCostMicros,
  collectFeature,
  estimateCostMicros,
  renderTriageItems,
  streamFeature,
  TRIAGE_BATCH_MAX,
  tableModel,
  tierFor,
  tokenCostMicros,
  transcribeAudio,
  triageResults,
  validateVars,
  varsChars,
} from "@akatsuki/ai";
import { getFeature, isFeatureId } from "@akatsuki/prompts";
import type { LanguageModel, TranscriptionModel } from "ai";
import { type Context, Hono, type MiddlewareHandler } from "hono";
import { bodyLimit } from "hono/body-limit";
import { cors } from "hono/cors";
import { z } from "zod";
import { AuthError, bearerToken, type VerifiedUser } from "./auth.ts";
import type { ConvexGateway, Tier } from "./convex.ts";
import type { AppEnv } from "./env.ts";
import { type ApiErrorCode, sendError } from "./errors.ts";
import { addLogMeta, type LogSink, requestLogger, stdoutSink } from "./log.ts";
import {
  type BillingProvider,
  eventCustomer,
  SNAPSHOT_EVENTS,
  StripeEventDedupe,
  toSnapshot,
} from "./stripe.ts";
import { audioSecondsEstimate, nextMonthStart, SlidingWindowLimiter } from "./util.ts";

export interface AppDeps {
  config: { corsOrigins: string[]; billing: boolean };
  verifyUser: (token: string) => Promise<VerifiedUser>;
  /** Present when WORKER_SERVICE_JWKS is configured. */
  verifyWorker?: (token: string) => Promise<{ sub: string }>;
  convex: ConvexGateway;
  /** Managed language model by routing.json `akatsuki` id (OpenRouter, ZDR). Absent without a key. */
  languageModel?: (modelId: string) => LanguageModel;
  /** Managed transcription model by id (OpenAI). Absent without a key. */
  transcriptionModel?: (modelId: string) => TranscriptionModel;
  billing?: BillingProvider;
  /** Per-process webhook dedupe (a fresh one per app unless a test injects one). */
  stripeDedupe?: StripeEventDedupe;
  log?: LogSink;
  now?: () => number;
}

export const AI_BODY_MAX = 256 * 1024;
/** Managed dictation cap (§13.4), enforced by the daily bucket in `ee.service.aiPrecheck` (D-343). */
export const TRANSCRIPTION_MINUTES_PER_DAY = 120;
export const AUDIO_BODY_MAX = 25 * 1024 * 1024 + 64 * 1024;
export const WEBHOOK_BODY_MAX = 1024 * 1024;
const JSON_BODY_MAX = 16 * 1024;
const AUDIO_TYPES = /^audio\/(webm|ogg|mp4|m4a|x-m4a|aac|wav|x-wav|wave|mpeg|mp3)(;.*)?$/i;
const BCP47 = /^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$/;

const limit = (maxSize: number): MiddlewareHandler<AppEnv> =>
  bodyLimit({ maxSize, onError: (c) => sendError(c, "payload_too_large") });

function precheckError(
  c: Context<AppEnv>,
  r: { code: string; retryAfterMs?: number; limit?: string },
  now: number,
) {
  if (r.limit === "transcription_day") {
    const resetsAt = new Date(now + (r.retryAfterMs ?? 0)).toISOString();
    addLogMeta(c, { code: "transcription_day_cap" });
    return sendError(
      c,
      "rate_limited",
      r.retryAfterMs,
      `Dictation is limited to ${TRANSCRIPTION_MINUTES_PER_DAY} minutes a day. It resets at ${resetsAt}.`,
    );
  }
  if (r.code === "ai_budget_exhausted") {
    const resetsAt = new Date(now + (r.retryAfterMs ?? 0)).toISOString();
    return sendError(c, "ai_budget_exhausted", r.retryAfterMs, `It resets at ${resetsAt}.`);
  }
  return sendError(c, r.code as ApiErrorCode, r.retryAfterMs);
}

export function createApp(deps: AppDeps) {
  const now = deps.now ?? Date.now;
  const app = new Hono<AppEnv>();
  const checkoutLimiter = new SlidingWindowLimiter(10, 60 * 60 * 1000, now);
  const stripeDedupe = deps.stripeDedupe ?? new StripeEventDedupe({ now });

  app.use("*", requestLogger(deps.log ?? stdoutSink, now));
  app.use("*", async (c, next) => {
    await next();
    c.res.headers.set("cache-control", "no-store");
    c.res.headers.set("x-content-type-options", "nosniff");
  });
  app.use(
    "/v1/*",
    cors({
      origin: (origin) => (deps.config.corsOrigins.includes(origin) ? origin : null),
      allowMethods: ["GET", "POST", "OPTIONS"],
      allowHeaders: ["authorization", "content-type"],
      maxAge: 600,
      credentials: false,
    }),
  );

  app.onError((err, c) => {
    if (err instanceof AuthError) return sendError(c, "unauthenticated");
    // Only the error class name reaches the log, never its message (it may carry provider text).
    addLogMeta(c, { code: `internal:${err.name}`.slice(0, 40) });
    return sendError(c, "internal");
  });
  app.notFound((c) => sendError(c, "not_found"));

  app.get("/healthz", (c) => c.json({ ok: true }));

  // ───────────── user routes: Better Auth JWT (§6.5) ─────────────
  app.use("/v1/*", async (c, next) => {
    if (c.req.method === "OPTIONS") return next();
    const token = bearerToken(c.req.header("authorization"));
    if (!token) return sendError(c, "unauthenticated");
    let user: VerifiedUser;
    try {
      user = await deps.verifyUser(token);
    } catch {
      return sendError(c, "unauthenticated");
    }
    c.set("userId", user.userId);
    if (user.email) c.set("userEmail", user.email);
    c.set("userToken", token);
    return next();
  });

  app.get("/v1/ai/quota", async (c) => {
    const usage = await deps.convex.userAiUsage(c.get("userToken")!);
    return c.json({ ...usage, resetsAt: nextMonthStart(now()) });
  });

  app.post("/v1/ai/transcribe", limit(AUDIO_BODY_MAX), async (c) => {
    const userId = c.get("userId")!;
    if (!deps.transcriptionModel) return sendError(c, "provider_unavailable");
    let form: FormData;
    try {
      form = await c.req.formData();
    } catch {
      return sendError(c, "invalid_argument");
    }
    const audio = form.get("audio");
    const language = form.get("language");
    const prompt = form.get("prompt");
    if (!(audio instanceof File) || audio.size === 0) return sendError(c, "invalid_argument");
    if (audio.size > 25 * 1024 * 1024) return sendError(c, "payload_too_large");
    if (!AUDIO_TYPES.test(audio.type)) return sendError(c, "invalid_argument");
    if (language !== null && (typeof language !== "string" || !BCP47.test(language)))
      return sendError(c, "invalid_argument");
    if (prompt !== null && (typeof prompt !== "string" || Array.from(prompt).length > 500))
      return sendError(c, "invalid_argument");

    const modelId = tableModel("transcription", "akatsuki")!;
    const bytes = new Uint8Array(await audio.arrayBuffer());
    const estSeconds = audioSecondsEstimate(bytes, audio.type);
    if (estSeconds.exact && estSeconds.seconds > 600) return sendError(c, "payload_too_large");
    const pre = await deps.convex.aiPrecheck({
      userId,
      feature: "dictation",
      tier: "transcription",
      estCostMicros: audioCostMicros(modelId, Math.min(600, estSeconds.seconds)),
      audioSeconds: Math.max(1, Math.ceil(Math.min(600, estSeconds.seconds))),
    });
    if (!pre.ok) return precheckError(c, pre, now());

    let result: Awaited<ReturnType<typeof transcribeAudio>>;
    try {
      result = await transcribeAudio(deps.transcriptionModel(modelId), bytes, {
        language: typeof language === "string" ? language : undefined,
        prompt: typeof prompt === "string" ? prompt : undefined,
        abortSignal: c.req.raw.signal,
      });
    } catch {
      return sendError(c, "provider_unavailable");
    }
    if (result.durationSec > 600) addLogMeta(c, { code: "audio_over_10_min" });
    const seconds = Math.ceil(result.durationSec > 0 ? result.durationSec : estSeconds.seconds);
    await deps.convex.aiRecordUsage({
      userId,
      feature: "dictation",
      model: modelId,
      inputTokens: 0,
      outputTokens: 0,
      audioSeconds: seconds,
      costMicros: audioCostMicros(modelId, seconds),
    });
    addLogMeta(c, { model: modelId, audioSeconds: seconds });
    return c.json({
      text: result.text,
      durationSec: result.durationSec || seconds,
      language: result.language,
    });
  });

  const zAiBody = z.object({
    promptVersion: z.number().int().nonnegative(),
    vars: z.record(z.string(), z.unknown()),
  });

  app.post("/v1/ai/:feature", limit(AI_BODY_MAX), async (c) => {
    const userId = c.get("userId")!;
    const feature = c.req.param("feature");
    // Only prompt-pack features: raw prompts are never accepted (§8 step 2).
    if (!isFeatureId(feature)) return sendError(c, "not_found");
    if (!deps.languageModel) return sendError(c, "provider_unavailable");
    let raw: unknown;
    try {
      raw = await c.req.json();
    } catch {
      return sendError(c, "invalid_argument");
    }
    const body = zAiBody.safeParse(raw);
    if (!body.success) return sendError(c, "invalid_argument");
    const f = getFeature(feature);
    const vars = validateVars(feature, body.data.vars);
    if (!vars.ok) return sendError(c, "invalid_vars");
    if (body.data.promptVersion !== f.meta.version) return sendError(c, "prompt_version_mismatch");
    const inputChars = varsChars(vars.value);
    if (inputChars > f.meta.maxInputChars) return sendError(c, "payload_too_large");

    const tier: Tier = tierFor(feature);
    const modelId = tableModel(tier, "akatsuki")!;
    const est = estimateCostMicros(modelId, inputChars + Array.from(f.system).length, f.meta.maxOutputTokens);
    const pre = await deps.convex.aiPrecheck({ userId, feature, tier, estCostMicros: est });
    if (!pre.ok) return precheckError(c, pre, now());
    let model: LanguageModel;
    try {
      model = deps.languageModel(modelId);
    } catch (e) {
      addLogMeta(c, { code: `model_init:${(e as Error).name}`.slice(0, 40) });
      return sendError(c, "provider_unavailable");
    }

    // Usage is reported by the model only when an attempt finishes. Until then, what was streamed is counted
    // here, so a client that reads the deltas and disconnects before `done` (or a provider error mid-stream)
    // is still charged for the output it received, not just the input (D-406).
    let recorded = false;
    let attemptModel = modelId;
    let streamedChars = 0;
    const promptChars = inputChars + Array.from(f.system).length;
    const record = async (model: string, usage: { inputTokens: number; outputTokens: number }) => {
      recorded = true;
      addLogMeta(c, { model, inputTokens: usage.inputTokens, outputTokens: usage.outputTokens });
      try {
        await deps.convex.aiRecordUsage({
          userId,
          feature,
          model,
          inputTokens: usage.inputTokens,
          outputTokens: usage.outputTokens,
          audioSeconds: 0,
          costMicros: tokenCostMicros(model, usage),
        });
      } catch (e) {
        addLogMeta(c, { code: `usage_not_recorded:${(e as Error).name}`.slice(0, 40) });
      }
    };

    const signal = c.req.raw.signal;
    let finish!: () => void;
    c.set(
      "streamDone",
      new Promise<void>((r) => {
        finish = r;
      }),
    );
    async function* events(): AsyncGenerator<AaspEvent> {
      try {
        for await (const e of streamFeature({
          feature: feature as Parameters<typeof streamFeature>[0]["feature"],
          vars: vars.ok ? vars.value : {},
          choice: { model, modelId, kind: "akatsuki", tier },
          abortSignal: signal,
          onUsage: ({ modelId: m, usage }) => record(m, usage),
        })) {
          if (e.event === "meta") {
            // A new attempt (escalation): the previous one was recorded through onUsage.
            recorded = false;
            attemptModel = e.data.model;
            streamedChars = 0;
          } else if (e.event === "delta") streamedChars += e.data.text.length;
          else if (e.event === "partial") streamedChars = JSON.stringify(e.data.object ?? null).length;
          else if (e.event === "error") addLogMeta(c, { code: e.data.code });
          yield e;
        }
      } catch (e) {
        addLogMeta(c, { code: `stream:${(e as Error).name}`.slice(0, 40) });
        yield {
          event: "error",
          data: { code: "internal", message: "Something went wrong.", retryable: true },
        };
      } finally {
        // An attempt without reported usage (client gone, provider error mid-stream) is charged an estimate:
        // the prompt it sent and the output it streamed, about four characters a token.
        if (!recorded) {
          addLogMeta(c, { code: signal.aborted ? "client_aborted" : "no_usage" });
          await record(attemptModel, abortedUsage(promptChars, streamedChars));
        }
        finish();
      }
    }
    return new Response(aaspReadableStream(events()), {
      status: 200,
      headers: { "content-type": "text/event-stream; charset=utf-8", "x-accel-buffering": "no" },
    });
  });

  // ───────────── billing (§13.1) ─────────────
  const requireBilling: MiddlewareHandler<AppEnv> = async (c, next) => {
    if (!deps.config.billing) return sendError(c, "not_found");
    if (!deps.billing) return sendError(c, "provider_unavailable");
    return next();
  };
  const emptyJson = async (c: Context<AppEnv>) => {
    try {
      const text = await c.req.text();
      return text.trim() === "" ? {} : JSON.parse(text);
    } catch {
      return undefined;
    }
  };

  app.post("/v1/billing/checkout", requireBilling, limit(JSON_BODY_MAX), async (c) => {
    const userId = c.get("userId")!;
    const rl = checkoutLimiter.take(userId);
    if (!rl.ok) return sendError(c, "rate_limited", rl.retryAfterMs);
    const ident = await deps.convex.billingIdentity({ userId });
    if (["trialing", "active", "grace"].includes(ident.entitlement.state))
      return sendError(c, "already_subscribed");
    let customerId = ident.stripeCustomerId;
    if (!customerId) {
      customerId = await deps.billing!.createCustomer({ email: c.get("userEmail") ?? ident.email, userId });
      await deps.convex.linkStripeCustomer({ userId, stripeCustomerId: customerId });
    }
    const url = await deps.billing!.createCheckout({
      customerId,
      userId,
      trialEligible: ident.trialUsedAt === undefined,
    });
    return c.json({ url });
  });

  app.post("/v1/billing/portal", requireBilling, limit(JSON_BODY_MAX), async (c) => {
    const ident = await deps.convex.billingIdentity({ userId: c.get("userId")! });
    if (!ident.stripeCustomerId) return sendError(c, "not_found");
    return c.json({ url: await deps.billing!.createPortal(ident.stripeCustomerId) });
  });

  app.post("/v1/billing/cancel", requireBilling, limit(JSON_BODY_MAX), async (c) => {
    const body = z.object({ immediately: z.boolean() }).safeParse(await emptyJson(c));
    if (!body.success) return sendError(c, "invalid_argument");
    const ident = await deps.convex.billingIdentity({ userId: c.get("userId")! });
    if (ident.stripeCustomerId) await deps.billing!.cancel(ident.stripeCustomerId, body.data.immediately);
    return c.body(null, 204);
  });

  // ───────────── Stripe webhook (§13.2) ─────────────
  app.post("/hooks/stripe", limit(WEBHOOK_BODY_MAX), async (c) => {
    if (!deps.config.billing) return sendError(c, "not_found");
    if (!deps.billing) return sendError(c, "provider_unavailable");
    const signature = c.req.header("stripe-signature");
    if (!signature) return sendError(c, "unauthenticated");
    const raw = await c.req.text();
    let event: ReturnType<BillingProvider["constructEvent"]>;
    try {
      event = deps.billing.constructEvent(raw, signature);
    } catch {
      return sendError(c, "unauthenticated");
    }
    addLogMeta(c, { code: `evt:${event.type}`.slice(0, 40) });
    const billing = deps.billing;

    const outcome = await stripeDedupe.run(event.id, async () => {
      if (event.type === "customer.subscription.trial_will_end") {
        // Sends an email, so it is deduplicated in Convex before acting.
        const { duplicate } = await deps.convex.recordStripeEvent({ eventId: event.id, type: event.type });
        const { customerId } = eventCustomer(event);
        if (!duplicate && customerId) await deps.convex.trialWillEnd({ stripeCustomerId: customerId });
        return;
      }
      if (SNAPSHOT_EVENTS.has(event.type)) {
        // Applying a full re-fetched snapshot is idempotent, so it runs before the event is marked processed:
        // a failure returns 500 and Stripe's retry is not mistaken for a duplicate (DECISIONS D-122).
        const { customerId, userId } = eventCustomer(event);
        if (customerId) {
          const fetchedAt = now();
          const sub = await billing.latestSubscription(customerId);
          await deps.convex.applyStripeSubscription({
            stripeCustomerId: customerId,
            ...(userId ? { userId } : {}),
            snapshot: sub ? toSnapshot(sub) : null,
            fetchedAt,
          });
        }
      }
      await deps.convex.recordStripeEvent({ eventId: event.id, type: event.type });
    });
    if (outcome === "duplicate") addLogMeta(c, { code: "evt_duplicate" });
    return c.json({ received: true });
  });

  // ───────────── worker-only (§12.5, D-048) ─────────────
  app.use("/internal/*", async (c, next) => {
    if (!deps.verifyWorker) return sendError(c, "not_found");
    const token = bearerToken(c.req.header("authorization"));
    if (!token) return sendError(c, "unauthenticated");
    try {
      c.set("workerSub", (await deps.verifyWorker(token)).sub);
    } catch {
      return sendError(c, "unauthenticated");
    }
    return next();
  });

  const zAddr = z.object({ name: z.string().max(200).nullish(), email: z.string().max(320) });
  const zTriageBody = z.object({
    userId: z.string().min(1).max(200),
    items: z
      .array(
        z.object({
          threadId: z.string().min(1).max(64),
          from: zAddr.nullish(),
          to: z.array(zAddr).max(100),
          subject: z.string().max(998),
          snippet: z.string().max(200),
          listId: z.string().max(500).nullish(),
          isBulk: z.boolean(),
          hasAttachments: z.boolean(),
        }),
      )
      .min(1)
      .max(TRIAGE_BATCH_MAX),
  });

  app.post("/internal/v1/triage", limit(AI_BODY_MAX), async (c) => {
    if (!deps.languageModel) return sendError(c, "provider_unavailable");
    let raw: unknown;
    try {
      raw = await c.req.json();
    } catch {
      return sendError(c, "invalid_argument");
    }
    const body = zTriageBody.safeParse(raw);
    if (!body.success) return sendError(c, "invalid_argument");
    const { userId, items } = body.data;
    c.set("userId", userId);
    const { text, keys } = renderTriageItems(items);
    const modelId = tableModel("fast", "akatsuki")!;
    const f = getFeature("triage");
    const pre = await deps.convex.aiPrecheck({
      userId,
      feature: "triage",
      tier: "fast",
      estCostMicros: estimateCostMicros(
        modelId,
        Array.from(text).length + Array.from(f.system).length,
        f.meta.maxOutputTokens,
      ),
    });
    if (!pre.ok) return precheckError(c, pre, now());
    try {
      const res = await collectFeature(
        streamFeature({
          feature: "triage",
          vars: { items: text },
          choice: { model: deps.languageModel(modelId), modelId, kind: "akatsuki", tier: "fast" },
          abortSignal: c.req.raw.signal,
          onUsage: async ({ modelId: m, usage }) => {
            addLogMeta(c, { model: m, inputTokens: usage.inputTokens, outputTokens: usage.outputTokens });
            await deps.convex.aiRecordUsage({
              userId,
              feature: "triage",
              model: m,
              inputTokens: usage.inputTokens,
              outputTokens: usage.outputTokens,
              audioSeconds: 0,
              costMicros: tokenCostMicros(m, usage),
            });
          },
        }),
      );
      const results = triageResults(
        keys,
        res.object as Parameters<typeof triageResults>[1],
        res.model ?? modelId,
      );
      return c.json({ results });
    } catch (e) {
      const code = (e as { code?: string }).code;
      return sendError(c, code === "invalid_vars" ? "invalid_argument" : "provider_unavailable");
    }
  });

  return app;
}

/** The usage charged for an attempt the model did not report on (D-406): ~4 characters per token. */
export function abortedUsage(promptChars: number, streamedChars: number) {
  return { inputTokens: Math.ceil(promptChars / 4), outputTokens: Math.ceil(streamedChars / 4) };
}
