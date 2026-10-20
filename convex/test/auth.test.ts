import { ConvexError } from "convex/values";
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import {
  asApi,
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  opId,
  seedThread,
  setEnv,
} from "./setup.helpers";

const code = async (p: Promise<unknown>) => {
  try {
    await p;
  } catch (e) {
    return e instanceof ConvexError ? (e.data as { code: string }).code : `non-app: ${(e as Error).message}`;
  }
  return "no error";
};

describe("auth wrappers", () => {
  beforeEach(() => setEnv());

  it("rejects calls without an identity", async () => {
    const t = makeTest();
    expect(await code(t.query(api.me.get, {}))).toBe("unauthenticated");
    expect(await code(t.query(api.splits.list, {}))).toBe("unauthenticated");
    expect(await code(t.query(api.worker.assignedAccounts, { shard: 0 }))).toBe("unauthenticated");
  });

  it("rejects identities from the wrong issuer", async () => {
    const t = makeTest();
    const stranger = t.withIdentity({ subject: "u1", issuer: "https://evil.example" });
    expect(await code(stranger.query(api.me.get, {}))).toBe("unauthenticated");
    // A user token is not a worker or apps/api token, and the services cannot act as each other.
    expect(await code(asUser(t, "u1").query(api.worker.assignedAccounts, { shard: 0 }))).toBe(
      "unauthenticated",
    );
    expect(await code(asUser(t, "u1").query(api.ee.service.billingIdentity, { userId: "u1" }))).toBe(
      "unauthenticated",
    );
    expect(await code(asApi(t).query(api.worker.assignedAccounts, { shard: 0 }))).toBe("unauthenticated");
    expect(await code(asWorker(t).query(api.ee.service.billingIdentity, { userId: "u1" }))).toBe(
      "unauthenticated",
    );
  });

  it("bootstraps a user and gates mail functions on entitlement", async () => {
    const t = makeTest();
    const a = asUser(t, "alice");
    expect(await a.query(api.me.get, {})).toBeNull();
    await a.mutation(api.me.bootstrap, { timezone: "Europe/Zurich" });
    await a.mutation(api.me.bootstrap, { timezone: "Europe/Zurich" }); // idempotent
    const me = await a.query(api.me.get, {});
    expect(me?.profile.onboarding).toBe("checkout");
    expect(me?.entitlement.hostedSync).toBe(false);
    expect((await a.query(api.ai.getSettings, {}))?.consent.autoTriage).toBe(false);
    // No free hosted tier: signed in but not entitled cannot use mail functions.
    expect(await code(a.query(api.splits.list, {}))).toBe("not_entitled");
    await entitledUser(t, "alice");
    const splits = await a.query(api.splits.list, {});
    expect(splits.map((s) => s.name)).toEqual(["VIP", "News"]);
    expect((await a.query(api.me.get, {}))?.entitlement.state).toBe("active");
  });

  it("treats every user as entitled when billing is off (self-host)", async () => {
    setEnv({ AKATSUKI_BILLING: "off" });
    const t = makeTest();
    const a = asUser(t, "self");
    await a.mutation(api.me.bootstrap, { timezone: "UTC" });
    expect((await a.query(api.me.get, {}))?.entitlement.state).toBe("selfhost");
    expect(await a.query(api.splits.list, {})).toHaveLength(2);
  });

  it("does not let user B read or change user A's data", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await entitledUser(t, "bob");
    const alice = asUser(t, "alice");
    const bob = asUser(t, "bob");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-a", [message()]);
    const { snippetId } = await alice.mutation(api.snippets.save, {
      name: "Thanks",
      shortcut: "thx",
      bodyHtml: "<p>Thanks!</p>",
      variables: [],
    });

    // Alice sees her own data.
    expect((await alice.query(api.mail.getThread, { threadId })).thread._id).toBe(threadId);
    expect(await alice.query(api.accounts.list, {})).toHaveLength(1);

    // Bob gets not_found (never "forbidden"), and his lists are empty.
    expect(await code(bob.query(api.mail.getThread, { threadId }))).toBe("not_found");
    expect(
      await code(bob.query(api.mail.olderMessages, { threadId, beforeDate: Date.now(), limit: 10 })),
    ).toBe("not_found");
    expect(await code(bob.query(api.labels.list, { accountId }))).toBe("not_found");
    expect(await code(bob.query(api.sync.changes, { accountId, sinceRev: 0, limit: 10 }))).toBe("not_found");
    expect(
      await code(
        bob.query(api.mail.listThreads, {
          view: { labelKey: "all", accountId },
          paginationOpts: { numItems: 10, cursor: null },
        }),
      ),
    ).toBe("not_found");
    expect(
      (
        await bob.query(api.mail.listThreads, {
          view: { labelKey: "sys:inbox" },
          paginationOpts: { numItems: 10, cursor: null },
        })
      ).page,
    ).toEqual([]);
    expect(await bob.query(api.accounts.list, {})).toEqual([]);
    expect(await bob.query(api.snippets.list, {})).toEqual([]);
    expect(
      await code(
        bob.mutation(api.mail.apply, {
          clientOpId: opId(1),
          threadIds: [threadId],
          action: { kind: "trash" },
        }),
      ),
    ).toBe("not_found");
    expect(await code(bob.mutation(api.snippets.remove, { snippetId }))).toBe("not_found");
    expect(await code(bob.mutation(api.accounts.remove, { accountId }))).toBe("not_found");
    expect(await code(bob.mutation(api.accounts.update, { accountId, displayName: "mine now" }))).toBe(
      "not_found",
    );

    // Alice's thread is untouched.
    const view = await alice.query(api.mail.getThread, { threadId });
    expect(view.thread.labelKeys).toContain("sys:inbox");
    expect(view.pendingOps).toEqual([]);
  });

  it("refuses to connect a mailbox already owned by another user", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    await entitledUser(t, "bob");
    await connectGmail(t, "alice", "shared@gmail.com");
    expect(await code(connectGmail(t, "bob", "shared@gmail.com"))).toBe("already_connected");
  });

  it("refuses to connect a mailbox for a user who is not entitled", async () => {
    const t = makeTest();
    await asUser(t, "carol").mutation(api.me.bootstrap, { timezone: "UTC" });
    expect(await code(connectGmail(t, "carol"))).toBe("not_entitled");
  });

  it("requires the account lease for worker writes", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    expect(
      await code(
        asWorker(t).mutation(api.worker.upsertThreads, { accountId, workerId: "someone-else", threads: [] }),
      ),
    ).toBe("lease_lost");
  });
});
