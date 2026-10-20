// Threads that synced before AI triage was turned on (triageState "off") become claimable (§12.5, D-413):
// when new inbound mail arrives in them, and, for the last 14 days, as soon as triage is turned on.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  connectGmail,
  makeTest,
  message,
  seedThread,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

const PROFILE = "6f0c9a52-3b1e-4c1d-9a8e-2f4b5c6d7e8f";
const DAY = 86_400_000;

async function turnOnTriage(t: T, userId: string, kind: "akatsuki" | "ollama") {
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

const bootstrap = (t: T, userId: string) =>
  asUser(t, userId).mutation(api.me.bootstrap, { timezone: "Europe/Zurich" });

const stateOf = (t: T, id: Id<"threads">) => t.run(async (ctx) => (await ctx.db.get(id))!.triageState);

describe("triage turned on after the first sync", () => {
  beforeEach(() => {
    // Self-host billing: no entitlement re-evaluation timers, so running the scheduler does not move the clock.
    setEnv({ AKATSUKI_BILLING: "off", AKATSUKI_MANAGED_AI: "on" });
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("new inbound mail in an untriaged conversation makes it claimable by the worker", async () => {
    const t = makeTest();
    await bootstrap(t, "u-tri-new");
    const { accountId } = await connectGmail(t, "u-tri-new");
    // Initial sync while AI is off: an old conversation (outside the 14-day window).
    const old = message({ providerMessageId: "old-1", date: Date.now() - 40 * DAY });
    const threadId = await seedThread(t, accountId, "thr-old", [old]);
    expect(await stateOf(t, threadId)).toBe("off");

    await turnOnTriage(t, "u-tri-new", "akatsuki");
    await t.finishAllScheduledFunctions(vi.runAllTimers);
    expect(await stateOf(t, threadId)).toBe("off"); // older than 14 days: not part of the catch-up

    // A reply arrives in that conversation.
    const res = await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [
        {
          providerThreadId: "thr-old",
          complete: false,
          messages: [message({ providerMessageId: "new-1", date: Date.now() - 60_000 })] as never,
        },
      ],
    });
    expect(res.newInbound).toEqual([{ threadId, isInbox: true }]);
    expect(await stateOf(t, threadId)).toBe("none");
    const items = await asWorker(t).mutation(api.worker.claimTriage, {
      accountId,
      workerId: WORKER_ID,
      threadIds: [threadId],
      limit: 25,
    });
    expect(items.map((i) => i.threadId)).toEqual([threadId]);
  });

  it("turning triage on makes the last 14 days claimable for a BYO client", async () => {
    const t = makeTest();
    await bootstrap(t, "u-tri-byo");
    const { accountId } = await connectGmail(t, "u-tri-byo");
    const recent = await seedThread(t, accountId, "thr-recent", [message({ date: Date.now() - 2 * DAY })]);
    const old = await seedThread(t, accountId, "thr-older", [message({ date: Date.now() - 30 * DAY })]);
    expect(await stateOf(t, recent)).toBe("off");

    await turnOnTriage(t, "u-tri-byo", "ollama");
    await t.finishAllScheduledFunctions(vi.runAllTimers);
    expect(await stateOf(t, recent)).toBe("none");
    expect(await stateOf(t, old)).toBe("off");

    const claimed = await asUser(t, "u-tri-byo").mutation(api.ai.claimTriage, {
      deviceId: "dev-1",
      limit: 25,
    });
    expect(claimed.map((c) => c.threadId)).toEqual([recent]);
  });

  it("new mail while triage stays off leaves the conversation off", async () => {
    const t = makeTest();
    await bootstrap(t, "u-tri-off");
    const { accountId } = await connectGmail(t, "u-tri-off");
    const threadId = await seedThread(t, accountId, "thr-off", [message({ date: Date.now() - 40 * DAY })]);
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [{ providerThreadId: "thr-off", complete: false, messages: [message()] as never }],
    });
    expect(await stateOf(t, threadId)).toBe("off");
  });
});

describe("ai.saveSettings and the managed provider", () => {
  beforeEach(() => setEnv());

  it("refuses a base URL on an akatsuki profile (it would receive the session JWT, D-415)", async () => {
    const t = makeTest();
    await bootstrap(t, "u-ai-url");
    const save = (baseUrl?: string) =>
      asUser(t, "u-ai-url").mutation(api.ai.saveSettings, {
        profiles: [
          {
            profileId: PROFILE,
            label: "Akatsuki AI",
            kind: "akatsuki",
            baseUrl,
            models: {},
            keyOnDevices: [],
          },
        ],
        routes: [{ feature: "default", profileId: PROFILE, enabled: true }],
        consent: { grantedAt: Date.now(), version: 1, autoTriage: false, syncVoiceProfile: false },
      });
    await expect(save("https://evil.example")).rejects.toThrow(/no base URL/);
    await save();
    const s = await asUser(t, "u-ai-url").query(api.ai.getSettings, {});
    expect(s?.profiles[0]?.baseUrl).toBeUndefined();
  });
});
