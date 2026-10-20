// Archive on folder accounts without an archive folder (§5.1, D-392): a placeholder label row, a lazy
// label_create, and the move waits for it.
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  entitledUser,
  makeTest,
  message,
  opId,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

const IMAP_CAPS = { multiLabel: false, categories: false, serverSearch: true, deleteForever: true };
const nonce = () => new Uint8Array(24).buffer as ArrayBuffer;
const folder = (providerLabelId: string, role?: "inbox" | "trash" | "archive") => ({
  providerLabelId,
  name: providerLabelId,
  kind: role === "archive" || !role ? ("folder" as const) : ("system" as const),
  role,
  exclusive: true,
  hidden: false,
});

async function connectImap(t: T, userId: string) {
  const w = asWorker(t);
  const email = `${userId}@example.org`;
  const { accountId } = await w.mutation(api.worker.createAccount, {
    userId,
    provider: "imap",
    providerAccountId: `${email}@imap.example.org`,
    email,
    imap: {
      host: "imap.example.org",
      port: 993,
      tls: "implicit",
      smtpHost: "smtp.example.org",
      smtpPort: 465,
      smtpTls: "implicit",
      capabilities: [],
      sentAutoSaved: false,
    },
    caps: IMAP_CAPS,
    identities: [{ email, isDefault: true }],
    secret: {
      kind: "imap",
      keyId: "k1",
      nonce: nonce(),
      ciphertext: new Uint8Array([1]).buffer as ArrayBuffer,
    },
  });
  const claim = await w.mutation(api.worker.claimAccount, {
    accountId,
    workerId: WORKER_ID,
    leaseMs: 60_000,
  });
  if (!claim.ok) throw new Error("claim failed");
  await w.mutation(api.worker.upsertLabels, {
    accountId,
    workerId: WORKER_ID,
    complete: true,
    labels: [folder("INBOX", "inbox"), folder("Trash", "trash"), folder("Projects")],
  });
  await w.mutation(api.worker.upsertThreads, {
    accountId,
    workerId: WORKER_ID,
    threads: [
      { providerThreadId: "t-1", complete: true, messages: [message({ labelKeys: ["sys:inbox"] })] as never },
    ],
  });
  const threadId = await t.run(
    async (ctx) =>
      (await ctx.db
        .query("threads")
        .withIndex("by_account_provider", (q) => q.eq("accountId", accountId).eq("providerThreadId", "t-1"))
        .unique())!._id,
  );
  return { accountId, threadId };
}

async function archiveLabel(t: T, accountId: Id<"mailAccounts">) {
  const labels = await asUser(t, "alice").query(api.labels.list, { accountId });
  return labels.filter((l) => l.role === "archive");
}

describe("archive folder on folder accounts", () => {
  beforeEach(() => setEnv());

  it("adds one placeholder Archive folder when the server has none, and keeps it across syncs", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectImap(t, "alice");
    const archive = await archiveLabel(t, accountId);
    expect(archive).toHaveLength(1);
    expect(archive[0]).toMatchObject({ name: "Archive", exclusive: true, kind: "folder" });
    expect(archive[0]!.providerLabelId).toBeUndefined();
    await asWorker(t).mutation(api.worker.upsertLabels, {
      accountId,
      workerId: WORKER_ID,
      complete: true,
      labels: [folder("INBOX", "inbox"), folder("Trash", "trash")],
    });
    expect(await archiveLabel(t, accountId)).toHaveLength(1);
  });

  it("archive creates the folder lazily, the move waits for it and then runs", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId, threadId } = await connectImap(t, "alice");
    const [archive] = await archiveLabel(t, accountId);
    const key = `l:${archive!._id}`;
    await asUser(t, "alice").mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "move", to: key },
    });
    // Optimistically archived at once.
    const thread = await t.run(async (ctx) => (await ctx.db.get(threadId))!);
    expect(thread.labelKeys).toContain(key);
    expect(thread.labelKeys).not.toContain("sys:inbox");

    const w = asWorker(t);
    const first = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 100,
      leaseMs: 60_000,
    });
    expect(first.map((o) => o.op.payload.kind)).toEqual(["label_create"]);
    expect(first[0]!.labels[key]).toMatchObject({ name: "Archive", role: "archive" });

    // A second archive does not queue a second label_create.
    await asUser(t, "alice").mutation(api.mail.apply, {
      clientOpId: opId(2),
      threadIds: [threadId],
      action: { kind: "move", to: "sys:inbox" },
    });
    const ops = await t.run(async (ctx) => await ctx.db.query("ops").collect());
    expect(ops.filter((o) => o.payload.kind === "label_create")).toHaveLength(1);

    await w.mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [{ opId: first[0]!.op._id, ok: true, outcome: "applied", providerLabelId: "Archive" }],
      labels: [folder("Archive", "archive")],
    });
    const after = await archiveLabel(t, accountId);
    expect(after).toHaveLength(1);
    expect(after[0]).toMatchObject({ _id: archive!._id, providerLabelId: "Archive" });

    const next = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 100,
      leaseMs: 60_000,
    });
    // Both thread ops, in order: the archive, then the move back that queued behind it (FIFO per thread).
    expect(next.map((o) => o.op.payload.kind)).toEqual(["thread", "thread"]);
    expect(next.map((o) => o.op.clientOpId)).toEqual([opId(1), opId(2)]);
    expect(next[0]!.labels[key]).toMatchObject({ providerLabelId: "Archive" });
  });

  it("a failed folder create fails the waiting move, which rolls back", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId, threadId } = await connectImap(t, "alice");
    const [archive] = await archiveLabel(t, accountId);
    await asUser(t, "alice").mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "move", to: `l:${archive!._id}` },
    });
    const w = asWorker(t);
    const [create] = await w.mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 100,
      leaseMs: 60_000,
    });
    await w.mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [
        {
          opId: create!.op._id,
          ok: false,
          error: { code: "invalid_action", message: "Folder cannot be created.", retryable: false },
          countAttempt: true,
        },
      ],
    });
    const thread = await t.run(async (ctx) => (await ctx.db.get(threadId))!);
    expect(thread.labelKeys).toContain("sys:inbox");
    const ops = await t.run(async (ctx) => await ctx.db.query("ops").collect());
    expect(ops.map((o) => o.state).sort()).toEqual(["failed", "failed"]);
  });
});
