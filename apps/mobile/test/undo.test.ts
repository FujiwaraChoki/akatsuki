import { applyContextFor } from "@akatsuki/mail-model";
import { applyOptimisticUpdate } from "@akatsuki/mail-model/optimistic";
import type { OptimisticLocalStore } from "convex/browser";
import { getFunctionName } from "convex/server";
import { describe, expect, it } from "vitest";
import { rememberForUndo, undoOptimisticUpdate } from "../src/vm/undoOptimistic.ts";

type Entry = { name: string; args: Record<string, unknown>; value: unknown };

/** A minimal OptimisticLocalStore over a list of cached queries. */
function fakeStore(entries: Entry[]): OptimisticLocalStore & { entries: Entry[] } {
  const key = (name: string, args: unknown) => `${name}:${JSON.stringify(args)}`;
  return {
    entries,
    getQuery(ref: never, args: Record<string, unknown>) {
      return entries.find((e) => key(e.name, e.args) === key(getFunctionName(ref), args))?.value;
    },
    getAllQueries(ref: never) {
      return entries
        .filter((e) => e.name === getFunctionName(ref))
        .map((e) => ({ args: e.args, value: e.value }));
    },
    setQuery(ref: never, args: Record<string, unknown>, value: unknown) {
      const e = entries.find((x) => key(x.name, x.args) === key(getFunctionName(ref), args));
      if (e) e.value = value;
      else entries.push({ name: getFunctionName(ref), args, value });
    },
  } as unknown as OptimisticLocalStore & { entries: Entry[] };
}

const row = (id: string, sortAt: number, keys: string[]) => ({
  _id: id,
  accountId: "acc",
  labelKeys: keys,
  unreadCount: keys.includes("sys:unread") ? 1 : 0,
  pendingOpCount: 0,
  sortAt,
});

const inboxArgs = { view: { labelKey: "sys:inbox" }, paginationOpts: { numItems: 30, cursor: null } };
const ctx = () => applyContextFor("gmail", []);

describe("instant undo", () => {
  it("puts an archived row back in place", () => {
    const store = fakeStore([
      {
        name: "mail:listThreads",
        args: inboxArgs,
        value: {
          page: [
            row("a", 3, ["sys:inbox"]),
            row("b", 2, ["sys:inbox", "sys:unread"]),
            row("c", 1, ["sys:inbox"]),
          ],
          isDone: true,
          continueCursor: "",
        },
      },
    ]);
    const archive = { kind: "modify" as const, add: [], remove: ["sys:inbox"] };
    rememberForUndo(store, "op1", ["b"], archive);
    applyOptimisticUpdate({ contextFor: ctx })(store, { threadIds: ["b"], action: archive });
    const page = () => (store.entries[0]!.value as { page: { _id: string; labelKeys: string[] }[] }).page;
    expect(page().map((r) => r._id)).toEqual(["a", "c"]);
    undoOptimisticUpdate({ contextFor: ctx, isFolderAccount: () => false })(store, { undoClientOpId: "op1" });
    expect(page().map((r) => r._id)).toEqual(["a", "b", "c"]);
    expect(page()[1]!.labelKeys).toEqual(["sys:inbox", "sys:unread"]);
  });

  it("reverts a star in place and ignores unknown ops", () => {
    const store = fakeStore([
      {
        name: "mail:listThreads",
        args: inboxArgs,
        value: { page: [row("a", 3, ["sys:inbox"])], isDone: true, continueCursor: "" },
      },
    ]);
    const star = { kind: "modify" as const, add: ["sys:starred"], remove: [] };
    rememberForUndo(store, "op2", ["a"], star);
    applyOptimisticUpdate({ contextFor: ctx })(store, { threadIds: ["a"], action: star });
    const first = () => (store.entries[0]!.value as { page: { labelKeys: string[] }[] }).page[0]!;
    expect(first().labelKeys).toEqual(["sys:inbox", "sys:starred"]);
    const undo = undoOptimisticUpdate({ contextFor: ctx, isFolderAccount: () => false });
    undo(store, { undoClientOpId: "op2" });
    expect(first().labelKeys).toEqual(["sys:inbox"]);
    undo(store, { undoClientOpId: "nope" });
    expect(first().labelKeys).toEqual(["sys:inbox"]);
  });

  it("does not insert into a page the row does not belong to", () => {
    const store = fakeStore([
      {
        name: "mail:listThreads",
        args: inboxArgs,
        value: { page: [row("a", 10, ["sys:inbox"])], isDone: false, continueCursor: "x" },
      },
    ]);
    const archive = { kind: "modify" as const, add: [], remove: ["sys:inbox"] };
    // "old" is not cached anywhere, so nothing is remembered and nothing comes back.
    rememberForUndo(store, "op3", ["old"], archive);
    undoOptimisticUpdate({ contextFor: ctx, isFolderAccount: () => false })(store, { undoClientOpId: "op3" });
    expect((store.entries[0]!.value as { page: unknown[] }).page).toHaveLength(1);
  });
});
