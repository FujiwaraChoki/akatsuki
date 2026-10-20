// Push notification privacy (ARCHITECTURE §4.5, §14.2, D-351): "private" is the default and sends counts only.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { internal } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import { connectGmail, entitledUser, makeTest, message, seedThread, setEnv, type T } from "./setup.helpers";

type PushMessage = { to: string; title: string; body: string; data?: { threadId: string } };

async function seed(t: T, userId: string) {
  await entitledUser(t, userId);
  const { accountId } = await connectGmail(t, userId);
  const threadId = await seedThread(t, accountId, "thr-push-1", [
    message({ from: { name: "Ada Lovelace", email: "ada@example.com" }, subject: "Engine notes" }),
  ]);
  // The device is added after the seed, so upsertThreads schedules no real push.
  await t.run(async (ctx) => {
    await ctx.db.insert("devices", {
      userId,
      deviceId: "phone-1",
      kind: "mobile",
      name: "Phone",
      appVersion: "0.1.0",
      expoPushToken: "ExponentPushToken[test]",
      lastSeenAt: Date.now(),
    });
  });
  return threadId;
}

function stubExpo(): PushMessage[] {
  const sent: PushMessage[] = [];
  vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
    sent.push(...(JSON.parse(String(init.body)) as PushMessage[]));
    return new Response("{}", { status: 200 });
  });
  return sent;
}

async function setMode(t: T, userId: string, mode: "off" | "private" | "full" | "none") {
  await t.run(async (ctx) => {
    const p = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (!p) throw new Error("no profile");
    if (mode === "none") await ctx.db.delete(p._id);
    else await ctx.db.patch(p._id, { prefs: { ...p.prefs, notifications: mode } });
  });
}

describe("push notifications", () => {
  beforeEach(() => setEnv());
  afterEach(() => vi.unstubAllGlobals());

  it("defaults to private for new users", async () => {
    const t = makeTest();
    const threadId = await seed(t, "u-push-default");
    const r = await t.query(internal.notify.pushTargets, { userId: "u-push-default", threadIds: [threadId] });
    expect(r.mode).toBe("private");
    expect(r.items).toEqual([]);
  });

  it("treats a missing profile as private, not off and not full", async () => {
    const t = makeTest();
    const threadId = await seed(t, "u-push-noprofile");
    await setMode(t, "u-push-noprofile", "none");
    const r = await t.query(internal.notify.pushTargets, {
      userId: "u-push-noprofile",
      threadIds: [threadId],
    });
    expect(r.mode).toBe("private");
    expect(r.items).toEqual([]);

    const sent = stubExpo();
    await t.action(internal.notify.newMail, { userId: "u-push-noprofile", threadIds: [threadId] });
    expect(sent).toEqual([{ to: "ExponentPushToken[test]", title: "Akatsuki", body: "New mail" }]);
  });

  it("private sends a count only; full sends sender and subject; off sends nothing", async () => {
    const t = makeTest();
    const userId = "u-push-modes";
    const threadId: Id<"threads"> = await seed(t, userId);

    let sent = stubExpo();
    await t.action(internal.notify.newMail, { userId, threadIds: [threadId] });
    expect(JSON.stringify(sent)).not.toContain("Engine notes");
    expect(JSON.stringify(sent)).not.toContain("Ada");
    expect(sent[0]?.body).toBe("New mail");

    await setMode(t, userId, "full");
    sent = stubExpo();
    await t.action(internal.notify.newMail, { userId, threadIds: [threadId] });
    expect(sent).toEqual([
      { to: "ExponentPushToken[test]", title: "Ada Lovelace", body: "Engine notes", data: { threadId } },
    ]);

    await setMode(t, userId, "off");
    sent = stubExpo();
    await t.action(internal.notify.newMail, { userId, threadIds: [threadId] });
    expect(sent).toEqual([]);
  });
});
