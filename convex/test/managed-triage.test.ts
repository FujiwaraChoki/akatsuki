// Managed triage on the worker (§12.5, D-390): worker.claimTriage → apps/api → worker.applyTriage.
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  seedThread,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

const PROFILE = "6f0c9a52-3b1e-4c1d-9a8e-2f4b5c6d7e8f";

async function managedRoute(t: T, userId: string, kind: "akatsuki" | "ollama" = "akatsuki") {
  await asUser(t, userId).mutation(api.ai.saveSettings, {
    profiles: [
      {
        profileId: PROFILE,
        label: kind === "akatsuki" ? "Akatsuki AI" : "Local",
        kind,
        baseUrl: kind === "ollama" ? "http://localhost:11434" : undefined,
        models: {},
        keyOnDevices: [],
      },
    ],
    routes: [{ feature: "default", profileId: PROFILE, enabled: true }],
    consent: { grantedAt: Date.now(), version: 1, autoTriage: true, syncVoiceProfile: false },
  });
}

const recent = (over: Record<string, unknown> = {}) => message({ date: Date.now() - 60_000, ...over });

async function triageOf(t: T, id: Id<"threads">) {
  return await t.run(async (ctx) => {
    const th = await ctx.db.get(id);
    return { state: th!.triageState, triage: th!.triage, lease: th!.triageLeaseUntil };
  });
}

describe("worker.claimTriage", () => {
  beforeEach(() => setEnv());

  it("claims new inbox threads of a managed-route user and applyTriage stores the result", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await managedRoute(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-1", [recent()]);
    expect((await triageOf(t, threadId)).state).toBe("none");

    const items = await asWorker(t).mutation(api.worker.claimTriage, {
      accountId,
      workerId: WORKER_ID,
      threadIds: [threadId],
      limit: 25,
    });
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ threadId, subject: "Quarterly numbers", isBulk: false });
    expect(Object.keys(items[0]!).sort()).toEqual(
      ["from", "hasAttachments", "isBulk", "snippet", "subject", "threadId", "to"].sort(),
    );
    expect((await triageOf(t, threadId)).state).toBe("pending");

    // Leased: a second claim does not hand it out again.
    expect(
      await asWorker(t).mutation(api.worker.claimTriage, {
        accountId,
        workerId: WORKER_ID,
        threadIds: [threadId],
        limit: 25,
      }),
    ).toEqual([]);

    await asWorker(t).mutation(api.worker.applyTriage, {
      results: [
        {
          threadId,
          category: "important",
          needsReply: true,
          model: "anthropic/claude-haiku-5.5",
          promptVersion: 1,
        },
      ],
    });
    const after = await triageOf(t, threadId);
    expect(after.state).toBe("done");
    expect(after.triage).toMatchObject({ category: "important", needsReply: true, source: "managed" });
  });

  it("sweeps untriaged and lease-expired threads when no ids are given", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await managedRoute(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    const a = await seedThread(t, accountId, "t-a", [recent()]);
    const b = await seedThread(t, accountId, "t-b", [recent()]);
    const old = await seedThread(t, accountId, "t-old", [message({ date: Date.now() - 30 * 86_400_000 })]);
    await t.run(async (ctx) => {
      await ctx.db.patch(b, { triageState: "pending", triageLeaseUntil: Date.now() - 1 });
    });
    const items = await asWorker(t).mutation(api.worker.claimTriage, {
      accountId,
      workerId: WORKER_ID,
      limit: 25,
    });
    expect(items.map((i) => i.threadId).sort()).toEqual([a, b].sort());
    expect((await triageOf(t, old)).state).toBe("none");
  });

  it("returns nothing for BYO routes, other accounts and non-inbox threads", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await managedRoute(t, "alice", "ollama");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-1", [recent()]);
    expect(
      await asWorker(t).mutation(api.worker.claimTriage, {
        accountId,
        workerId: WORKER_ID,
        threadIds: [threadId],
        limit: 25,
      }),
    ).toEqual([]);

    await managedRoute(t, "alice");
    const archived = await seedThread(t, accountId, "t-2", [recent({ labelKeys: ["sys:unread"] })]);
    await entitledUser(t, "bob");
    await managedRoute(t, "bob");
    const bob = await connectGmail(t, "bob");
    const bobThread = await seedThread(t, bob.accountId, "t-b", [recent()]);
    const items = await asWorker(t).mutation(api.worker.claimTriage, {
      accountId,
      workerId: WORKER_ID,
      threadIds: [archived, bobThread, threadId],
      limit: 25,
    });
    expect(items.map((i) => i.threadId)).toEqual([threadId]);
  });

  it("is worker-only, checks the lease and the limits", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await managedRoute(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    await expect(
      asUser(t, "alice").mutation(api.worker.claimTriage, { accountId, workerId: WORKER_ID, limit: 5 }),
    ).rejects.toThrow();
    await expect(
      asWorker(t).mutation(api.worker.claimTriage, { accountId, workerId: "someone-else", limit: 5 }),
    ).rejects.toThrow(/lease/);
    await expect(
      asWorker(t).mutation(api.worker.claimTriage, { accountId, workerId: WORKER_ID, limit: 26 }),
    ).rejects.toThrow();
  });
});
