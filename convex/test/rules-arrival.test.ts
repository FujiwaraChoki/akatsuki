// Filter rules and push notifications apply only to mail that arrives after the mailbox was connected (§4.4
// steps 5 and 8, D-402): initial sync, backfill, resync and search hits bring in history, not new mail.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

async function ruleOps(t: T, userId: string) {
  return await t.run(async (ctx) =>
    (await ctx.db.query("ops").collect()).filter(
      (o) => o.userId === userId && o.clientOpId.startsWith("rule:"),
    ),
  );
}

async function setup(t: T, userId: string, email?: string) {
  await entitledUser(t, userId);
  const { accountId } = await connectGmail(t, userId, email);
  // A user-wide rule (no accountId): mail from x.com goes to the trash.
  await asUser(t, userId).mutation(api.rules.save, {
    name: "x.com to trash",
    enabled: true,
    order: 0,
    rule: { match: "all", conds: [{ f: "from_domain", eq: "x.com" }] },
    actions: [{ kind: "trash" }],
  });
  const createdAt = await t.run(async (ctx) => (await ctx.db.get(accountId))!.createdAt);
  return { accountId, createdAt };
}

const upsert = (
  t: T,
  accountId: Id<"mailAccounts">,
  providerThreadId: string,
  date: number,
  m = message({ from: { name: "X", email: "news@x.com" }, date }),
) =>
  asWorker(t).mutation(api.worker.upsertThreads, {
    accountId,
    workerId: WORKER_ID,
    threads: [{ providerThreadId, complete: true, messages: [m] as never }],
  });

describe("rules on newly arrived mail only", () => {
  beforeEach(() => setEnv());
  afterEach(() => vi.unstubAllGlobals());

  it("initial sync, backfill and search hits of older mail run no rules", async () => {
    const t = makeTest();
    const { accountId, createdAt } = await setup(t, "u-rules-old");
    // InboxFirst / Window: mail from the 90 days before the connection.
    await upsert(t, accountId, "thr-window", createdAt - 3 * 86_400_000);
    // Backfill and a provider-search hit (GET /v1/search → SearchFetch → upsertThreads): older still.
    await upsert(t, accountId, "thr-backfill", createdAt - 200 * 86_400_000);
    await t.run(async (ctx) => ctx.db.patch(accountId, { status: "live" }));
    await upsert(t, accountId, "thr-search", createdAt - 900 * 86_400_000);
    expect(await ruleOps(t, "u-rules-old")).toEqual([]);
  });

  it("a second mailbox's history does not trigger an existing user-wide rule", async () => {
    const t = makeTest();
    const { createdAt } = await setup(t, "u-rules-second");
    const second = await connectGmail(t, "u-rules-second", "second@gmail.com");
    await upsert(t, second.accountId, "thr-second-old", createdAt - 86_400_000);
    expect(await ruleOps(t, "u-rules-second")).toEqual([]);
  });

  it("mail that arrives after the connection runs the rule once", async () => {
    const t = makeTest();
    const { accountId, createdAt } = await setup(t, "u-rules-new");
    const fresh = message({ from: { name: "X", email: "news@x.com" }, date: createdAt + 60_000 });
    await upsert(t, accountId, "thr-new", createdAt + 60_000, fresh);
    const ops = await ruleOps(t, "u-rules-new");
    expect(ops).toHaveLength(1);
    expect(ops[0]!.payload).toMatchObject({ kind: "thread", action: { kind: "trash" } });
    // Replaying the same page (a crash before the cursor moved) does not enqueue it again.
    await upsert(t, accountId, "thr-new", createdAt + 60_000, fresh);
    expect(await ruleOps(t, "u-rules-new")).toHaveLength(1);
  });

  it("push notifications only for newly arrived inbox mail on a live account", async () => {
    const t = makeTest();
    const { accountId, createdAt } = await setup(t, "u-push-arrival");
    await t.run(async (ctx) => {
      await ctx.db.patch(accountId, { status: "live" });
      await ctx.db.insert("devices", {
        userId: "u-push-arrival",
        deviceId: "phone-arrival",
        kind: "mobile",
        name: "Phone",
        appVersion: "0.1.0",
        expoPushToken: "ExponentPushToken[arrival]",
        lastSeenAt: Date.now(),
      });
    });
    const scheduled = async () =>
      await t.run(async (ctx) =>
        (await ctx.db.system.query("_scheduled_functions").collect()).filter((f) =>
          f.name.includes("notify"),
        ),
      );
    // A backfilled or searched old inbox thread: no "New mail" push.
    await upsert(t, accountId, "thr-push-old", createdAt - 30 * 86_400_000);
    expect(await scheduled()).toHaveLength(0);
    // A new message (from another domain, so the rule does not trash it).
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [
        {
          providerThreadId: "thr-push-new",
          complete: true,
          messages: [message({ date: createdAt + 1000 })] as never,
        },
      ],
    });
    expect(await scheduled()).toHaveLength(1);
  });
});
