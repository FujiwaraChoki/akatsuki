// optimisticApply against an in-memory OptimisticLocalStore (ARCHITECTURE §16.1).
import type { OptimisticLocalStore } from "convex/browser";
import { type FunctionReference, getFunctionName } from "convex/server";
import { describe, expect, it } from "vitest";
import { actionsFor, applyContextFor, SYS } from "../src/index.ts";
import {
  applyOptimisticUpdate,
  MAIL_QUERIES,
  type OptimisticThreadRow,
  optimisticApply,
  snoozeOptimisticUpdate,
  viewContains,
} from "../src/optimistic.ts";

class FakeStore implements OptimisticLocalStore {
  readonly entries = new Map<string, { name: string; args: Record<string, unknown>; value: unknown }>();
  sets = 0;
  put(query: FunctionReference<"query">, args: Record<string, unknown>, value: unknown) {
    const name = getFunctionName(query);
    this.entries.set(`${name}|${JSON.stringify(args)}`, { name, args, value });
  }
  // biome-ignore lint/suspicious/noExplicitAny: test double for a generic interface
  getQuery(query: any, ...args: any[]): any {
    return this.entries.get(`${getFunctionName(query)}|${JSON.stringify(args[0] ?? {})}`)?.value;
  }
  // biome-ignore lint/suspicious/noExplicitAny: test double for a generic interface
  getAllQueries(query: any): any[] {
    const name = getFunctionName(query);
    return [...this.entries.values()]
      .filter((e) => e.name === name)
      .map(({ args, value }) => ({ args, value }));
  }
  // biome-ignore lint/suspicious/noExplicitAny: test double for a generic interface
  setQuery(query: any, args: any, value: any): void {
    this.sets++;
    this.put(query, args, value);
  }
}

const row = (
  id: string,
  labelKeys: string[],
  over: Partial<OptimisticThreadRow> = {},
): OptimisticThreadRow => ({
  _id: id,
  accountId: "acct0001",
  labelKeys,
  unreadCount: labelKeys.includes(SYS.unread) ? 1 : 0,
  pendingOpCount: 0,
  ...over,
});
const page = (rows: OptimisticThreadRow[]) => ({ page: rows, isDone: true, continueCursor: "c" });
const pagination = { numItems: 50, cursor: null };
const inboxArgs = { view: { labelKey: SYS.inbox }, paginationOpts: pagination };
const starredArgs = { view: { labelKey: SYS.starred }, paginationOpts: pagination };
const gmailCtx = { contextFor: () => applyContextFor("gmail", []) };

function seeded() {
  const store = new FakeStore();
  store.put(
    MAIL_QUERIES.listThreads,
    inboxArgs,
    page([
      row("t1", [SYS.inbox, SYS.unread], { split: "split:other" }),
      row("t2", [SYS.inbox, SYS.starred]),
      row("t3", [SYS.inbox]),
    ]),
  );
  store.put(MAIL_QUERIES.listThreads, starredArgs, page([row("t2", [SYS.inbox, SYS.starred])]));
  store.put(
    MAIL_QUERIES.unreadCounts,
    { labelKeys: [SYS.inbox, "split:other"] },
    {
      [SYS.inbox]: 4,
      "split:other": 2,
    },
  );
  store.put(MAIL_QUERIES.getThread, { threadId: "t1" }, { thread: row("t1", [SYS.inbox, SYS.unread]) });
  return store;
}

const list = (store: FakeStore, args: unknown) =>
  (store.getQuery(MAIL_QUERIES.listThreads, args) as ReturnType<typeof page>).page;

describe("optimisticApply", () => {
  it("archive removes the rows from the inbox and decrements unread counts", () => {
    const store = seeded();
    optimisticApply(store, ["t1", "t2"], actionsFor({ provider: "gmail" }).archive(), gmailCtx);
    expect(list(store, inboxArgs).map((r) => r._id)).toEqual(["t3"]);
    const starred = list(store, starredArgs);
    expect(starred.map((r) => [r._id, r.labelKeys, r.pendingOpCount])).toEqual([["t2", [SYS.starred], 1]]);
    expect(store.getQuery(MAIL_QUERIES.unreadCounts, { labelKeys: [SYS.inbox, "split:other"] })).toEqual({
      [SYS.inbox]: 3,
      "split:other": 1,
    });
    const t1 = store.getQuery(MAIL_QUERIES.getThread, { threadId: "t1" }) as { thread: OptimisticThreadRow };
    expect(t1.thread.labelKeys).toEqual([SYS.unread]);
    expect(t1.thread.split).toBeUndefined();
  });

  it("mark read patches the row and moves the counts", () => {
    const store = seeded();
    applyOptimisticUpdate(gmailCtx)(store, {
      threadIds: ["t1"],
      action: actionsFor({ provider: "gmail" }).markRead(),
    });
    expect(list(store, inboxArgs)[0]).toMatchObject({ _id: "t1", labelKeys: [SYS.inbox], unreadCount: 0 });
    expect(store.getQuery(MAIL_QUERIES.unreadCounts, { labelKeys: [SYS.inbox, "split:other"] })).toEqual({
      [SYS.inbox]: 3,
      "split:other": 1,
    });
    // Marking unread again counts up.
    optimisticApply(store, ["t1"], actionsFor({ provider: "gmail" }).markUnread(), gmailCtx);
    expect(list(store, inboxArgs)[0]).toMatchObject({ unreadCount: 1, pendingOpCount: 2 });
    expect(store.getQuery(MAIL_QUERIES.unreadCounts, { labelKeys: [SYS.inbox, "split:other"] })).toEqual({
      [SYS.inbox]: 4,
      "split:other": 2,
    });
  });

  it("leaves capped counts and untouched queries alone", () => {
    const store = new FakeStore();
    store.put(MAIL_QUERIES.listThreads, inboxArgs, page([row("t1", [SYS.inbox, SYS.unread])]));
    store.put(MAIL_QUERIES.unreadCounts, { labelKeys: [SYS.inbox] }, { [SYS.inbox]: 100 });
    store.put(MAIL_QUERIES.listThreads, starredArgs, page([row("t9", [SYS.starred])]));
    store.sets = 0;
    optimisticApply(store, ["t1"], { kind: "trash" }, gmailCtx);
    expect(list(store, inboxArgs)).toEqual([]);
    expect(store.getQuery(MAIL_QUERIES.unreadCounts, { labelKeys: [SYS.inbox] })).toEqual({
      [SYS.inbox]: 100,
    });
    expect(store.sets).toBe(1);
  });

  it("uses each account's context (folder moves) and per-row actions for snooze", () => {
    const store = new FakeStore();
    const args = { view: { labelKey: "l:archive01" }, paginationOpts: pagination };
    store.put(MAIL_QUERIES.listThreads, args, page([row("t1", ["l:archive01"], { accountId: "imap0001" })]));
    store.put(MAIL_QUERIES.listThreads, inboxArgs, page([row("t2", [SYS.inbox], { accountId: "gmail001" })]));
    const contextFor = (a: string) =>
      a === "imap0001" ? applyContextFor("imap", ["archive01"]) : applyContextFor("gmail", []);
    snoozeOptimisticUpdate({
      contextFor,
      snoozeActionFor: (a) => actionsFor({ provider: a === "imap0001" ? "imap" : "gmail" }).snooze(),
    })(store, { threadIds: ["t1", "t2"] });
    expect(list(store, args)).toEqual([]);
    expect(list(store, inboxArgs)).toEqual([]);
  });

  it("deleted threads leave every view, including all mail", () => {
    const store = new FakeStore();
    const all = { view: { labelKey: "all" }, paginationOpts: pagination };
    store.put(MAIL_QUERIES.listThreads, all, page([row("t1", [SYS.inbox])]));
    optimisticApply(store, ["t1"], { kind: "delete_forever" }, gmailCtx);
    expect(list(store, all)).toEqual([]);
  });
});

describe("viewContains", () => {
  const v = (keys: string[], deleted = false) => ({ keys, deleted });
  it("mirrors mail.listThreads", () => {
    expect(viewContains({ labelKey: "all" }, v([]), { accountId: "a" })).toBe(true);
    expect(viewContains({ labelKey: "all", unreadOnly: true }, v([]), { accountId: "a" })).toBe(false);
    expect(viewContains({ labelKey: SYS.inbox, accountId: "b" }, v([SYS.inbox]), { accountId: "a" })).toBe(
      false,
    );
    expect(
      viewContains({ labelKey: "split:abcdefgh" }, v([SYS.inbox]), {
        accountId: "a",
        split: "split:abcdefgh",
      }),
    ).toBe(true);
    expect(
      viewContains({ labelKey: "split:abcdefgh" }, v([]), { accountId: "a", split: "split:abcdefgh" }),
    ).toBe(false);
    expect(viewContains({ labelKey: SYS.inbox }, v([SYS.inbox], true), { accountId: "a" })).toBe(false);
  });
});
