// mail.undo of an op the worker already claimed: the inverse comes from @akatsuki/mail-model (D-250), the same
// rule crates/core::model::apply_ops::inverse_action and packages/mail-model/fixtures/apply-ops.json pin.
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import type { Doc } from "../_generated/dataModel";
import { inverseAction } from "../lib/ops";
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
  WORKER_ID,
} from "./setup.helpers";

const page = { numItems: 50, cursor: null };

describe("mail.undo after claim", () => {
  beforeEach(() => setEnv());

  it("enqueues the inverse of an inflight archive and the thread is back in the inbox", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-1", [message()]);
    const alice = asUser(t, "alice");
    const archive = await alice.mutation(api.mail.apply, {
      clientOpId: opId(1),
      threadIds: [threadId],
      action: { kind: "modify", add: [], remove: ["sys:inbox"] },
    });
    await asWorker(t).mutation(api.worker.claimOps, {
      accountId,
      workerId: WORKER_ID,
      limit: 10,
      leaseMs: 60_000,
    });
    const undo = await alice.mutation(api.mail.undo, { clientOpId: opId(2), undoClientOpId: opId(1) });
    expect(undo.mode).toBe("inverse");
    const inverse = await t.run((ctx) => ctx.db.get(undo.opIds[0]!));
    expect(inverse?.payload).toEqual({
      kind: "thread",
      action: { kind: "modify", add: ["sys:inbox"], remove: [] },
    });
    expect(inverse?.inverseOf).toBe(archive.opIds[0]);
    const inbox = await alice.query(api.mail.listThreads, {
      view: { labelKey: "sys:inbox" },
      paginationOpts: page,
    });
    expect(inbox.page.map((r) => r._id)).toEqual([threadId]);
  });
});

describe("inverseAction (convex/lib/ops)", () => {
  const op = (action: Doc<"ops">["payload"], effect?: Doc<"ops">["effect"]) =>
    ({ payload: action, effect }) as Doc<"ops">;
  const folder = { caps: { multiLabel: false } } as Doc<"mailAccounts">;
  const gmail = { caps: { multiLabel: true } } as Doc<"mailAccounts">;

  it("moves back on folder providers, including untrash", () => {
    const trash = op(
      { kind: "thread", action: { kind: "trash" } },
      { added: ["sys:trash"], removed: ["l:archive01"] },
    );
    expect(inverseAction(trash, folder)).toEqual({ kind: "move", to: "l:archive01" });
    const untrash = op(
      { kind: "thread", action: { kind: "untrash" } },
      { added: ["sys:inbox"], removed: ["sys:trash"] },
    );
    expect(inverseAction(untrash, folder)).toEqual({ kind: "move", to: "sys:trash" });
    expect(inverseAction(untrash, gmail)).toEqual({
      kind: "modify",
      add: ["sys:trash"],
      remove: ["sys:inbox"],
    });
  });

  it("has no inverse for delete_forever, empty effects and ops without an effect", () => {
    const del = op(
      { kind: "thread", action: { kind: "delete_forever" } },
      { added: [], removed: ["sys:inbox"] },
    );
    expect(inverseAction(del, folder)).toBeNull();
    expect(
      inverseAction(op({ kind: "thread", action: { kind: "trash" } }, { added: [], removed: [] }), gmail),
    ).toBeNull();
    expect(inverseAction(op({ kind: "thread", action: { kind: "trash" } }), gmail)).toBeNull();
  });
});
