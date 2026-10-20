import { beforeEach, describe, expect, it } from "vitest";
import { api, internal } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  opId,
  seedThread,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

const page = { numItems: 50, cursor: null };

async function setup() {
  const t = makeTest();
  await entitledUser(t, "alice");
  const { accountId, labelKeyByProviderId } = await connectGmail(t, "alice");
  const threadId = await seedThread(t, accountId, "t-1", [message()]);
  return { t, alice: asUser(t, "alice"), accountId, threadId, receiptsKey: labelKeyByProviderId.Label_1! };
}

const inbox = (t: T) =>
  asUser(t, "alice").query(api.mail.listThreads, { view: { labelKey: "sys:inbox" }, paginationOpts: page });

describe("ops enqueue (mail.apply)", () => {
  beforeEach(() => setEnv());

  it("seeds a thread from the worker into the inbox list", async () => {
    const { t, alice, threadId } = await setup();
    const rows = (await inbox(t)).page;
    expect(rows.map((r) => r._id)).toEqual([threadId]);
    expect(rows[0]!.labelKeys).toEqual(["sys:inbox", "sys:unread"]);
    expect(rows[0]!.split).toBe("split:other");
    expect(await alice.query(api.mail.unreadCounts, { labelKeys: ["sys:inbox", "sys:starred"] })).toEqual({
      "sys:inbox": 1,
      "sys:starred": 0,
    });
  });

  it("archives optimistically: one queued op, thread leaves the inbox at once", async () => {
    const { t, alice, threadId, accountId } = await setup();
    const res = await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "modify", add: [], remove: ["sys:inbox"] },
    });
    expect(res.opIds).toHaveLength(1);
    expect(res.rev[accountId]).toBeGreaterThan(0);
    expect((await inbox(t)).page).toEqual([]);
    const view = await alice.query(api.mail.getThread, { threadId });
    expect(view.thread.labelKeys).toEqual(["sys:unread"]);
    expect(view.thread.pendingOpCount).toBe(1);
    expect(view.pendingOps[0]).toMatchObject({ clientOpId: opId(1), state: "queued", payloadKind: "thread" });
    const op = await t.run((ctx) => ctx.db.get(res.opIds[0]!));
    expect(op?.effect).toEqual({ added: [], removed: ["sys:inbox"] });

    // Replaying the same gesture is a no-op returning the same ids.
    const again = await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "modify", add: [], remove: ["sys:inbox"] },
    });
    expect(again.opIds).toEqual(res.opIds);
  });

  it("rejects split keys, unknown keys and actions outside the account caps", async () => {
    const { alice, threadId } = await setup();
    const bad = [
      { kind: "modify" as const, add: ["split:other"], remove: [] },
      { kind: "modify" as const, add: ["sys:bogus"], remove: [] },
      { kind: "move" as const, to: "sys:inbox" }, // gmail is multiLabel
      { kind: "delete_forever" as const }, // gmail.modify scope
    ];
    for (const [i, action] of bad.entries()) {
      await expect(
        alice.mutation(api.mail.apply, { clientOpId: opId(10 + i), threadIds: [threadId], action }),
      ).rejects.toThrow();
    }
  });

  it("undo cancels a queued op and the thread reappears", async () => {
    const { t, alice, threadId } = await setup();
    await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "trash" },
    });
    expect((await inbox(t)).page).toHaveLength(0);
    const u = await alice.mutation(api.mail.undo, { clientOpId: opId(2), undoClientOpId: opId(1) });
    expect(u.mode).toBe("cancelled");
    expect((await inbox(t)).page.map((r) => r._id)).toEqual([threadId]);
    expect((await alice.query(api.mail.getThread, { threadId })).thread.pendingOpCount).toBe(0);
  });

  it("worker claims, applies and completes; a permanent failure rolls back by construction", async () => {
    const { t, alice, threadId, accountId } = await setup();
    const w = asWorker(t);
    const star = await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "modify", add: ["sys:starred"], remove: [] },
    });
    const archive = await alice.mutation(api.mail.apply, {
      clientOpId: opId(2),
      threadIds: [threadId],
      action: { kind: "modify", add: [], remove: ["sys:inbox"] },
    });
    expect((await w.query(api.worker.pendingOpAccounts, { shard: 0 })).map((p) => p.accountId)).toEqual([
      accountId,
    ]);

    const claimed = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    expect(claimed.map((c) => c.op._id)).toEqual([star.opIds[0], archive.opIds[0]]);
    expect(claimed[0]!.thread?.providerThreadId).toBe("t-1");
    expect(claimed[0]!.labels["sys:starred"]?.providerLabelId).toBe("STARRED");
    // Nothing else is claimable while they are inflight.
    expect(
      await w.mutation(api.worker.claimOps, { accountId, workerId: WORKER_ID, limit: 10, leaseMs: 60_000 }),
    ).toEqual([]);

    // Star succeeds (with post-op provider state); archive fails permanently.
    await w.mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [
        { opId: star.opIds[0]!, ok: true, outcome: "applied" },
        {
          opId: archive.opIds[0]!,
          ok: false,
          error: { code: "provider_invalid", message: "Rejected.", retryable: false },
          countAttempt: true,
        },
      ],
      threads: [
        {
          providerThreadId: "t-1",
          complete: true,
          messages: [
            message({ providerMessageId: "m1", labelKeys: ["sys:inbox", "sys:unread", "sys:starred"] }),
          ] as never,
        },
      ],
    });
    const view = await alice.query(api.mail.getThread, { threadId });
    expect(view.thread.labelKeys).toEqual(["sys:inbox", "sys:starred", "sys:unread"]);
    expect(view.thread.pendingOpCount).toBe(0);
    expect((await inbox(t)).page.map((r) => r._id)).toEqual([threadId]);
    const failures = await alice.query(api.ops.failures, {});
    expect(failures).toHaveLength(1);
    expect(failures[0]).toMatchObject({ _id: archive.opIds[0], state: "failed", payloadKind: "thread" });
    await alice.mutation(api.ops.ackFailure, { opIds: [archive.opIds[0]!] });
    expect(await alice.query(api.ops.failures, {})).toEqual([]);

    // Retry re-enqueues the same action as a new op.
    const retry = await alice.mutation(api.ops.retry, { opId: archive.opIds[0]!, clientOpId: opId(3) });
    expect(retry.opIds).toHaveLength(1);
    expect((await inbox(t)).page).toEqual([]);
  });

  it("a transient failure requeues with backoff and keeps the optimistic state", async () => {
    const { t, alice, threadId, accountId } = await setup();
    const w = asWorker(t);
    const r = await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "trash" },
    });
    await w.mutation(api.worker.claimOps, { accountId, workerId: WORKER_ID, limit: 10, leaseMs: 60_000 });
    await w.mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [
        {
          opId: r.opIds[0]!,
          ok: false,
          error: { code: "provider_transient", message: "Try again.", retryable: true },
          countAttempt: true,
        },
      ],
    });
    const op = await t.run((ctx) => ctx.db.get(r.opIds[0]! as Id<"ops">));
    expect(op?.state).toBe("queued");
    expect(op?.attempt).toBe(1);
    expect(op!.availableAt).toBeGreaterThan(Date.now());
    expect((await alice.query(api.mail.getThread, { threadId })).thread.labelKeys).toContain("sys:trash");
  });

  it("label ops: create returns an optimistic label key and blocks thread ops until created", async () => {
    const { t, alice, threadId, accountId } = await setup();
    const w = asWorker(t);
    const created = await alice.mutation(api.labels.create, {
      clientOpId: opId(1),
      accountId,
      name: "Projects",
    });
    expect(created.labelKey).toBe(`l:${created.labelId}`);
    const apply = await alice.mutation(api.mail.apply, {
      clientOpId: opId(2),
      threadIds: [threadId],
      action: { kind: "modify", add: [created.labelKey], remove: [] },
    });
    expect((await alice.query(api.mail.getThread, { threadId })).thread.labelKeys).toContain(
      created.labelKey,
    );
    const first = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    expect(first.map((c) => c.op._id)).toEqual([created.opId]); // the thread op waits for label_create
    await w.mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [{ opId: created.opId, ok: true, outcome: "applied", providerLabelId: "Label_9" }],
    });
    const second = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    expect(second.map((c) => c.op._id)).toEqual(apply.opIds);
    expect(second[0]!.labels[created.labelKey]?.providerLabelId).toBe("Label_9");
  });

  it("snooze moves the thread out of the inbox and a timer brings it back", async () => {
    const { t, alice, threadId } = await setup();
    const until = Date.now() + 3_600_000;
    const { timerIds } = await alice.mutation(api.mail.snooze, {
      clientOpId: opId(1),
      threadIds: [threadId],
      until,
      returnUnread: true,
    });
    expect(timerIds).toHaveLength(1);
    expect((await inbox(t)).page).toEqual([]);
    const snoozed = await alice.query(api.mail.listThreads, {
      view: { labelKey: "sys:snoozed" },
      paginationOpts: page,
    });
    expect(snoozed.page[0]?.snoozedUntil).toBe(until);
    expect(await alice.query(api.mail.listTimers, { kind: "snooze" })).toHaveLength(1);
    await alice.mutation(api.mail.unsnooze, { clientOpId: opId(2), threadIds: [threadId] });
    expect((await inbox(t)).page.map((r) => r._id)).toEqual([threadId]);
    expect(await alice.query(api.mail.listTimers, { kind: "snooze" })).toEqual([]);
  });

  it("the desktop feed returns changes after a rev", async () => {
    const { alice, accountId, threadId } = await setup();
    const first = await alice.query(api.sync.changes, { accountId, sinceRev: 0, limit: 500 });
    expect(first.threads.map((x) => x._id)).toContain(threadId);
    expect(first.resetRequired).toBe(false);
    const empty = await alice.query(api.sync.changes, { accountId, sinceRev: first.upToRev, limit: 500 });
    expect(empty.threads).toEqual([]);
    await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "trash" },
    });
    const next = await alice.query(api.sync.changes, { accountId, sinceRev: first.upToRev, limit: 500 });
    expect(next.threads.map((x) => x._id)).toEqual([threadId]);
    expect(next.upToRev).toBeGreaterThan(first.upToRev);
  });

  it("the feed asks for a re-pull once tombstones past sinceRev were collected (D-391)", async () => {
    const { t, alice, accountId } = await setup();
    const before = await alice.query(api.sync.changes, { accountId, sinceRev: 0, limit: 500 });
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [],
      deletes: ["t-1"],
    });
    const after = await alice.query(api.sync.changes, { accountId, sinceRev: before.upToRev, limit: 500 });
    expect(after.tombstones.map((x) => x.table)).toContain("threads");
    expect(after.resetRequired).toBe(false);

    // 31 days later the cron deletes those tombstones.
    await t.run(async (ctx) => {
      for (const row of await ctx.db.query("tombstones").collect())
        await ctx.db.patch(row._id, { at: Date.now() - 31 * 86_400_000 });
    });
    await t.mutation(internal.sync.gcTombstones, {});
    expect(await t.run(async (ctx) => (await ctx.db.query("tombstones").collect()).length)).toBe(0);

    const stale = await alice.query(api.sync.changes, { accountId, sinceRev: before.upToRev, limit: 500 });
    expect(stale.resetRequired).toBe(true);
    // A mirror that already saw the deletions, and a fresh pull from 0, are fine.
    const current = await alice.query(api.sync.changes, { accountId, sinceRev: after.upToRev, limit: 500 });
    expect(current.resetRequired).toBe(false);
    const fresh = await alice.query(api.sync.changes, { accountId, sinceRev: 0, limit: 500 });
    expect(fresh.resetRequired).toBe(false);
  });

  it("drafts.discard and sends.cancel enqueue and tombstone correctly", async () => {
    const { t, alice, accountId } = await setup();
    const w = asWorker(t);
    const d = await w.mutation(api.worker.recordDraft, {
      userId: "alice",
      accountId,
      clientDraftId: "draft-1",
      baseVersion: 0,
      providerDraftId: "r1",
      providerMessageId: "pm1",
      rfcMessageId: "0192@gmail.com",
      subject: "Hi",
      to: [{ email: "bob@example.com" }],
      cc: [],
      bcc: [],
    });
    expect(d).toMatchObject({ version: 1 });
    expect(
      await w.mutation(api.worker.recordDraft, {
        userId: "alice",
        accountId,
        clientDraftId: "draft-1",
        baseVersion: 0,
        providerDraftId: "r1",
        providerMessageId: "pm1",
        rfcMessageId: "0192@gmail.com",
        subject: "Hi",
        to: [],
        cc: [],
        bcc: [],
      }),
    ).toEqual({ conflict: true, version: 1 });
    // Undo pressed before the send reached the worker: tombstone, then scheduleSend sees it.
    expect(await alice.mutation(api.sends.cancel, { clientSendId: "send-0001" })).toEqual({
      result: "cancelled",
    });
    const draftId = (d as { draftId: Id<"drafts"> }).draftId;
    const s = await w.mutation(api.worker.scheduleSend, {
      userId: "alice",
      accountId,
      clientSendId: "send-0001",
      draftId,
      kind: "undo",
      sendAt: Date.now() + 10_000,
      preview: { subject: "Hi", to: [] },
    });
    expect(s.state).toBe("cancelled");
    const s2 = await w.mutation(api.worker.scheduleSend, {
      userId: "alice",
      accountId,
      clientSendId: "send-0002",
      draftId,
      kind: "undo",
      sendAt: Date.now() + 10_000,
      preview: { subject: "Hi", to: [] },
    });
    expect(s2.state).toBe("scheduled");
    expect(await alice.mutation(api.sends.cancel, { clientSendId: "send-0002" })).toEqual({
      result: "cancelled",
    });
    const discard = await alice.mutation(api.drafts.discard, {
      clientOpId: opId(5),
      clientDraftId: "draft-1",
    });
    expect((await t.run((ctx) => ctx.db.get(discard.opId)))?.payload.kind).toBe("delete_draft");
  });

  it("a reply's send and draft ops never queue behind a backed-off thread op (§3.2, §5.4)", async () => {
    const { t, alice, accountId, threadId } = await setup();
    const w = asWorker(t);
    // A thread op on the conversation, backing off for 15 minutes.
    await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "trash" },
    });
    await t.run(async (ctx) => {
      const op = (await ctx.db.query("ops").collect())[0]!;
      await ctx.db.patch(op._id, { availableAt: Date.now() + 15 * 60_000 });
    });
    const d = (await w.mutation(api.worker.recordDraft, {
      userId: "alice",
      accountId,
      clientDraftId: "draft-r",
      baseVersion: 0,
      providerDraftId: "r9",
      providerMessageId: "pm9",
      rfcMessageId: "0193@gmail.com",
      threadId,
      subject: "Re: Hi",
      to: [{ email: "bob@example.com" }],
      cc: [],
      bcc: [],
    })) as { draftId: Id<"drafts"> };
    const discard = await alice.mutation(api.drafts.discard, {
      clientOpId: opId(2),
      clientDraftId: "draft-r",
    });
    const op = await t.run((ctx) => ctx.db.get(discard.opId));
    expect(op?.threadId).toBeUndefined();
    const claimed = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    expect(claimed.map((c) => c.op.payload.kind)).toEqual(["delete_draft"]);
    // The thread does not count the draft op as pending.
    expect((await t.run((ctx) => ctx.db.get(threadId)))?.pendingOpCount).toBe(1);
    expect(d.draftId).toBeDefined();
  });
});
