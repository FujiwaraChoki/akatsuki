// Shared convex-test harness: env, components, identities and seed data.
import betterAuthTest from "@convex-dev/better-auth/test";
import rateLimiterTest from "@convex-dev/rate-limiter/test";
import { convexTest } from "convex-test";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import schema from "../schema";
import { modules } from "../test.modules";

export const SITE = "https://test.convex.site";
export const WORKER_ISS = "https://sync.akatsuki.test";
export const API_ISS = "https://api.akatsuki.test";
export const WORKER_ID = "worker-test-1";

export function setEnv(overrides: Record<string, string | undefined> = {}) {
  process.env.CONVEX_SITE_URL = SITE;
  process.env.WORKER_ISSUER = WORKER_ISS;
  process.env.API_ISSUER = API_ISS;
  process.env.EMAIL_TRANSPORT = "none";
  delete process.env.AKATSUKI_BILLING;
  delete process.env.AKATSUKI_MANAGED_AI;
  for (const [k, v] of Object.entries(overrides)) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
}

export function makeTest() {
  const t = convexTest(schema, modules);
  betterAuthTest.register(t, "betterAuth");
  rateLimiterTest.register(t, "rateLimiter");
  return t;
}
export type T = ReturnType<typeof makeTest>;

export const asUser = (t: T, userId: string, email = `${userId}@example.com`) =>
  t.withIdentity({ subject: userId, issuer: SITE, email, name: userId });
export const asWorker = (t: T) => t.withIdentity({ subject: `worker:${WORKER_ID}`, issuer: WORKER_ISS });
export const asApi = (t: T) => t.withIdentity({ subject: "api", issuer: API_ISS });

/** Creates the user's rows and makes them entitled through the apps/api Stripe path. */
export async function entitledUser(t: T, userId: string) {
  await asUser(t, userId).mutation(api.me.bootstrap, { timezone: "Europe/Zurich" });
  const customer = `cus_${userId}`;
  await asApi(t).mutation(api.ee.service.linkStripeCustomer, { userId, stripeCustomerId: customer });
  await asApi(t).mutation(api.ee.service.applyStripeSubscription, {
    stripeCustomerId: customer,
    userId,
    snapshot: {
      id: `sub_${userId}`,
      status: "active",
      priceId: "price_test",
      currentPeriodEnd: Date.now() + 20 * 86_400_000, // keeps scheduled re-evaluation inside setTimeout range
      cancelAtPeriodEnd: false,
    },
    fetchedAt: Date.now(),
  });
}

export const GMAIL_CAPS = { multiLabel: true, categories: true, serverSearch: true, deleteForever: false };
const nonce = () => new Uint8Array(24).buffer as ArrayBuffer;

/** Connects a Gmail mailbox for `userId` through the worker functions and takes its lease. */
export async function connectGmail(t: T, userId: string, email = `${userId}@gmail.com`) {
  const w = asWorker(t);
  const { accountId } = await w.mutation(api.worker.createAccount, {
    userId,
    provider: "gmail",
    providerAccountId: email,
    email,
    caps: GMAIL_CAPS,
    identities: [{ email, isDefault: true }],
    secret: {
      kind: "oauth",
      keyId: "k1",
      nonce: nonce(),
      ciphertext: new Uint8Array([1, 2, 3]).buffer as ArrayBuffer,
    },
  });
  const claim = await w.mutation(api.worker.claimAccount, {
    accountId,
    workerId: WORKER_ID,
    leaseMs: 60_000,
  });
  if (!claim.ok) throw new Error("claim failed");
  const { labelKeyByProviderId } = await w.mutation(api.worker.upsertLabels, {
    accountId,
    workerId: WORKER_ID,
    complete: true,
    labels: [
      {
        providerLabelId: "INBOX",
        name: "INBOX",
        kind: "system",
        role: "inbox",
        exclusive: false,
        hidden: false,
      },
      {
        providerLabelId: "UNREAD",
        name: "UNREAD",
        kind: "system",
        role: "unread",
        exclusive: false,
        hidden: false,
      },
      {
        providerLabelId: "STARRED",
        name: "STARRED",
        kind: "system",
        role: "starred",
        exclusive: false,
        hidden: false,
      },
      {
        providerLabelId: "TRASH",
        name: "TRASH",
        kind: "system",
        role: "trash",
        exclusive: false,
        hidden: false,
      },
      { providerLabelId: "Label_1", name: "Receipts", kind: "user", exclusive: false, hidden: false },
    ],
  });
  return { accountId, labelKeyByProviderId };
}

let msgSeq = 0;
export function message(over: Partial<Record<string, unknown>> = {}) {
  msgSeq++;
  return {
    providerMessageId: `m${msgSeq}`,
    references: [],
    from: { name: "Ada", email: "ada@example.com" },
    to: [{ email: "me@gmail.com" }],
    cc: [],
    bcc: [],
    replyTo: [],
    subject: "Quarterly numbers",
    snippet: "Here are the numbers for Q3",
    date: 1_760_000_000_000 + msgSeq * 1000,
    labelKeys: ["sys:inbox", "sys:unread"],
    isUnread: true,
    isDraft: false,
    isOutbound: false,
    hasAttachments: false,
    ...over,
  };
}

/** Upserts one Gmail thread with the given messages and returns its Convex id. */
export async function seedThread(
  t: T,
  accountId: Id<"mailAccounts">,
  providerThreadId: string,
  messages: ReturnType<typeof message>[],
) {
  await asWorker(t).mutation(api.worker.upsertThreads, {
    accountId,
    workerId: WORKER_ID,
    threads: [{ providerThreadId, complete: true, messages: messages as never }],
  });
  return await t.run(async (ctx) => {
    const th = await ctx.db
      .query("threads")
      .withIndex("by_account_provider", (q) =>
        q.eq("accountId", accountId).eq("providerThreadId", providerThreadId),
      )
      .unique();
    return th!._id;
  });
}

export const opId = (n = 1) => `0192f3a0-0000-7000-8000-${String(n).padStart(12, "0")}`;
