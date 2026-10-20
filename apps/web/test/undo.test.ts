// The Convex undo optimistic update puts removed rows back into the cached list pages they belong to.
import { applyAction, applyContextFor, diffKeys, SYS } from "@akatsuki/mail-model";
import { MAIL_QUERIES } from "@akatsuki/mail-model/optimistic";
import { describe, expect, it } from "vitest";
import type { ThreadRow } from "../src/backend/types.ts";
import { reinsert, rememberUndo, snapshotFor, undoOptimisticUpdate } from "../src/mail/undoSnapshots.ts";

const row = (id: string, sortAt: number, keys: string[] = [SYS.inbox]): ThreadRow =>
  ({
    _id: id,
    accountId: "acc1",
    subject: id,
    snippet: "",
    participants: [],
    lastMessageAt: sortAt,
    sortAt,
    messageCount: 1,
    unreadCount: 0,
    draftCount: 0,
    hasAttachments: false,
    isBulk: false,
    labelKeys: keys,
    pendingOpCount: 0,
  }) as unknown as ThreadRow;

/** A minimal OptimisticLocalStore over a list of (args, value) pairs. */
function fakeStore(entries: { args: unknown; value: unknown }[]) {
  return {
    entries,
    getAllQueries(q: unknown) {
      return q === MAIL_QUERIES.listThreads ? entries : [];
    },
    getQuery() {
      return undefined;
    },
    setQuery(q: unknown, args: unknown, value: unknown) {
      if (q !== MAIL_QUERIES.listThreads) return;
      const e = entries.find((x) => x.args === args);
      if (e) e.value = value;
    },
  };
}

const inbox = { view: { labelKey: SYS.inbox }, paginationOpts: { numItems: 50, cursor: null } };

describe("undo reinsert", () => {
  it("restores a row into the first page in sortAt order", () => {
    const store = fakeStore([
      { args: inbox, value: { page: [row("a", 300), row("c", 100)], isDone: true, continueCursor: "" } },
    ]);
    reinsert(store as never, [row("b", 200)]);
    expect((store.entries[0]!.value as { page: ThreadRow[] }).page.map((r) => r._id)).toEqual([
      "a",
      "b",
      "c",
    ]);
  });
  it("skips views that would not contain the restored row and pages that do not cover it", () => {
    const starred = { view: { labelKey: SYS.starred }, paginationOpts: { numItems: 50, cursor: null } };
    const second = { view: { labelKey: SYS.inbox }, paginationOpts: { numItems: 50, cursor: "c1" } };
    const store = fakeStore([
      { args: starred, value: { page: [], isDone: true, continueCursor: "" } },
      { args: second, value: { page: [row("x", 50), row("y", 40)], isDone: false, continueCursor: "c2" } },
    ]);
    reinsert(store as never, [row("b", 200)]);
    expect((store.entries[0]!.value as { page: ThreadRow[] }).page).toEqual([]);
    expect((store.entries[1]!.value as { page: ThreadRow[] }).page.map((r) => r._id)).toEqual(["x", "y"]);
  });
  it("re-applies the inverse to cached rows and restores removed ones", () => {
    const before = row("t1", 500, [SYS.inbox, SYS.unread]);
    const archive = { kind: "modify" as const, add: [], remove: [SYS.inbox] };
    const ctx = () => applyContextFor("gmail", []);
    rememberUndo(
      "op-1",
      snapshotFor(
        [before],
        archive,
        () => "gmail",
        ctx,
        (k, a, c) => applyAction({ keys: k, deleted: false }, a, c).keys,
        diffKeys,
      ),
    );
    const store = fakeStore([{ args: inbox, value: { page: [], isDone: true, continueCursor: "" } }]);
    undoOptimisticUpdate(ctx)(store as never, { undoClientOpId: "op-1" });
    const page = (store.entries[0]!.value as { page: ThreadRow[] }).page;
    expect(page.map((r) => r._id)).toEqual(["t1"]);
    expect(page[0]!.pendingOpCount).toBe(1);
  });
});
