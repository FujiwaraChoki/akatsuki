// IMAP threads are partial (D-223): an upsert with `complete: false` never drops messages it does not list, and
// messages that are gone at the provider are named in `deletedMessages` (D-410).
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
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

async function threadMessages(t: T, threadId: Id<"threads">) {
  return await t.run(async (ctx) =>
    (
      await ctx.db
        .query("messages")
        .withIndex("by_thread_date", (q) => q.eq("threadId", threadId))
        .collect()
    ).map((m) => m.providerMessageId),
  );
}

describe("partial thread upserts", () => {
  beforeEach(() => setEnv());

  it("a delta that lists one new reply keeps the older messages", async () => {
    const t = makeTest();
    await entitledUser(t, "u-partial");
    const { accountId } = await connectGmail(t, "u-partial");
    const a = message({ providerMessageId: "INBOX:7:1" });
    const b = message({ providerMessageId: "INBOX:7:2" });
    const threadId = await seedThread(t, accountId, "imap-thread", [a, b]);
    // After a worker restart the IMAP index holds only the new reply.
    const c = message({ providerMessageId: "INBOX:7:3" });
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [{ providerThreadId: "imap-thread", complete: false, messages: [c] as never }],
    });
    expect((await threadMessages(t, threadId)).sort()).toEqual(["INBOX:7:1", "INBOX:7:2", "INBOX:7:3"]);
    const thread = await t.run(async (ctx) => ctx.db.get(threadId));
    expect(thread?.messageCount).toBe(3);
  });

  it("deleted messages are removed from their thread, and an emptied thread goes away", async () => {
    const t = makeTest();
    await entitledUser(t, "u-gone");
    const { accountId } = await connectGmail(t, "u-gone");
    const keep = await seedThread(t, accountId, "t-keep", [
      message({ providerMessageId: "INBOX:7:10" }),
      message({ providerMessageId: "INBOX:7:11" }),
    ]);
    const empty = await seedThread(t, accountId, "t-empty", [message({ providerMessageId: "INBOX:7:12" })]);
    const moved = message({ providerMessageId: "Archive:9:1" });
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      // The message moved INBOX → Archive: a new id in the same thread, the old one vanished.
      threads: [{ providerThreadId: "t-keep", complete: false, messages: [moved] as never }],
      deletedMessages: ["INBOX:7:11", "INBOX:7:12", "unknown:1:1"],
    });
    expect((await threadMessages(t, keep)).sort()).toEqual(["Archive:9:1", "INBOX:7:10"]);
    expect(await t.run(async (ctx) => ctx.db.get(empty))).toBeNull();
  });

  it("refuses more than 400 deleted messages", async () => {
    const t = makeTest();
    await entitledUser(t, "u-many");
    const { accountId } = await connectGmail(t, "u-many");
    await expect(
      asWorker(t).mutation(api.worker.upsertThreads, {
        accountId,
        workerId: WORKER_ID,
        threads: [],
        deletedMessages: Array.from({ length: 401 }, (_, i) => `INBOX:1:${i}`),
      }),
    ).rejects.toThrow(/400/);
  });
});
