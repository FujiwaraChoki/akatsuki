// §9.7: one "reauth_needed" email, 24 h after an account enters reauth_required, from either path (the
// worker's status report or an op's auth failure), and none if the account was reconnected by then.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import { REAUTH_EMAIL_AFTER_MS } from "../accounts";
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

async function jobs(t: T) {
  return await t.run(async (ctx) =>
    (await ctx.db.system.query("_scheduled_functions").collect()).map((j) => ({
      name: j.name,
      state: j.state.kind,
      args: j.args[0] as Record<string, unknown>,
    })),
  );
}

const status = (t: T, accountId: Id<"mailAccounts">, s: "reauth_required" | "live") =>
  asWorker(t).mutation(api.worker.setAccountStatus, { accountId, workerId: WORKER_ID, status: s });

describe("reauth email", () => {
  beforeEach(() => {
    setEnv();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("is sent 24 h later, once, when the account still needs reconnecting", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    await status(t, accountId, "reauth_required");
    expect((await jobs(t)).filter((j) => j.name.includes("email"))).toEqual([]);
    await status(t, accountId, "live");
    await status(t, accountId, "reauth_required");
    const reminders = (await jobs(t)).filter(
      (j) => j.name.includes("reauthReminder") && j.state === "pending",
    );
    expect(reminders).toHaveLength(1);

    vi.advanceTimersByTime(REAUTH_EMAIL_AFTER_MS + 1000);
    await t.finishInProgressScheduledFunctions();
    const emails = (await jobs(t)).filter((j) => j.name.includes("email"));
    expect(emails).toHaveLength(1);
    expect(emails[0]!.args).toMatchObject({ template: "reauth_needed", to: "alice@example.com" });
  });

  it("is not sent when the account was reconnected, and an op's auth failure also schedules it", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-1", [message()]);
    await asUser(t, "alice").mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "trash" },
    });
    const [op] = await asWorker(t).mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    await asWorker(t).mutation(api.worker.completeOps, {
      accountId,
      workerId: WORKER_ID,
      results: [
        {
          opId: op!.op._id,
          ok: false,
          error: { code: "reauth_required", message: "Reconnect.", retryable: true },
          countAttempt: false,
        },
      ],
    });
    expect((await jobs(t)).filter((j) => j.name.includes("reauthReminder"))).toHaveLength(1);
    await status(t, accountId, "live"); // reconnected within the day
    vi.advanceTimersByTime(REAUTH_EMAIL_AFTER_MS + 1000);
    await t.finishInProgressScheduledFunctions();
    expect((await jobs(t)).filter((j) => j.name.includes("email"))).toEqual([]);
  });
});
