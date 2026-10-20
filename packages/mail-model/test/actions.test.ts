// Action helpers (ARCHITECTURE §5.1 table) and their interplay with applyOps and inverseAction.
import { describe, expect, it } from "vitest";
import {
  ActionUnavailableError,
  actionsFor,
  applyContextFor,
  applyOps,
  diffKeys,
  inverseAction,
  isLabelKey,
  labelKeyOf,
  SYS,
  threadAggregates,
  threadKeysFromMessages,
  threadSearchText,
} from "../src/index.ts";

const gmail = actionsFor({ provider: "gmail", caps: { deleteForever: false } });
const graphLabels = [
  { _id: "archive01", exclusive: true, role: "archive" },
  { _id: "folder001", exclusive: true },
  { _id: "category1", exclusive: false },
];
const graph = actionsFor({ provider: "graph", caps: { deleteForever: true } }, graphLabels);
const graphCtx = applyContextFor("graph", ["archive01", "folder001"]);

describe("actionsFor on Gmail", () => {
  it("builds label deltas", () => {
    expect(gmail.archive()).toEqual({ kind: "modify", add: [], remove: [SYS.inbox] });
    expect(gmail.moveToInbox()).toEqual({ kind: "modify", add: [SYS.inbox], remove: [] });
    expect(gmail.spam()).toEqual({ kind: "modify", add: [SYS.spam], remove: [SYS.inbox] });
    expect(gmail.notSpam()).toEqual({ kind: "modify", add: [SYS.inbox], remove: [SYS.spam] });
    expect(gmail.snooze()).toEqual({ kind: "modify", add: [SYS.snoozed], remove: [SYS.inbox] });
    expect(gmail.addLabel("category1")).toEqual({ kind: "modify", add: ["l:category1"], remove: [] });
    expect(gmail.removeLabel("category1")).toEqual({ kind: "modify", add: [], remove: ["l:category1"] });
    expect(gmail.toggleRead(true)).toEqual(gmail.markRead());
    expect(gmail.toggleRead(false)).toEqual(gmail.markUnread());
    expect(gmail.toggleStar(true)).toEqual(gmail.unstar());
    expect(gmail.toggleStar(false)).toEqual(gmail.star());
    expect(gmail.unsubscribe("mailto")).toEqual({ kind: "unsubscribe", method: "mailto" });
  });

  it("refuses delete forever and moves", () => {
    expect(gmail.available("deleteForever")).toBe(false);
    expect(() => gmail.deleteForever()).toThrow(ActionUnavailableError);
    expect(() => gmail.moveTo(SYS.inbox)).toThrow(/folder-based/);
    expect(gmail.available("archive")).toBe(true);
  });
});

describe("actionsFor on folder providers", () => {
  it("turns locations into moves", () => {
    expect(graph.folders).toBe(true);
    expect(graph.archive()).toEqual({ kind: "move", to: "l:archive01" });
    expect(graph.moveToInbox()).toEqual({ kind: "move", to: SYS.inbox });
    expect(graph.spam()).toEqual({ kind: "move", to: SYS.spam });
    expect(graph.notSpam()).toEqual({ kind: "move", to: SYS.inbox });
    expect(graph.snooze()).toEqual({ kind: "move", to: SYS.snoozed });
    expect(graph.addLabel("folder001")).toEqual({ kind: "move", to: "l:folder001" });
    expect(graph.removeLabel("folder001")).toEqual({ kind: "move", to: SYS.inbox });
    expect(graph.addLabel("category1")).toEqual({ kind: "modify", add: ["l:category1"], remove: [] });
    expect(graph.deleteForever()).toEqual({ kind: "delete_forever" });
    expect(graph.markRead()).toEqual(gmail.markRead());
  });

  it("needs an archive folder to archive", () => {
    const imap = actionsFor({ provider: "imap" }, []);
    expect(imap.available("archive")).toBe(false);
    expect(() => imap.archive()).toThrow(expect.objectContaining({ code: "invalid_action" }));
    expect(imap.available("deleteForever")).toBe(true);
  });

  it("archive then undo returns the thread to the inbox", () => {
    const synced = [SYS.inbox, SYS.unread];
    const archive = graph.archive();
    const after = applyOps(synced, [archive], graphCtx);
    expect(after.keys).toEqual(["l:archive01", SYS.unread]);
    const inverse = inverseAction(archive, diffKeys(synced, after.keys), true);
    expect(inverse).toEqual({ kind: "move", to: SYS.inbox });
    expect(applyOps(synced, [archive, inverse!], graphCtx).keys).toEqual([SYS.inbox, SYS.unread]);
  });

  it("every helper builds valid label keys", () => {
    for (const a of [graph.archive(), graph.spam(), graph.snooze(), gmail.addLabel("category1")]) {
      const keys = a.kind === "move" ? [a.to] : a.kind === "modify" ? [...a.add, ...a.remove] : [];
      for (const k of keys) expect(isLabelKey(k)).toBe(true);
    }
  });
});

describe("labelKeyOf", () => {
  it("addresses system roles as sys: and everything else as l:", () => {
    expect(labelKeyOf({ _id: "abcdefgh1", role: "inbox" })).toBe("sys:inbox");
    expect(labelKeyOf({ _id: "abcdefgh1", role: "archive" })).toBe("l:abcdefgh1");
    expect(labelKeyOf({ _id: "abcdefgh1" })).toBe("l:abcdefgh1");
  });
});

describe("thread aggregates", () => {
  const msg = (over: Partial<Parameters<typeof threadAggregates>[0][number]>) => ({
    to: [],
    cc: [],
    subject: "Offsite",
    snippet: "",
    date: 0,
    labelKeys: [SYS.inbox],
    isUnread: false,
    isDraft: false,
    isOutbound: false,
    hasAttachments: false,
    ...over,
  });

  it("derives thread keys from messages", () => {
    expect(
      threadKeysFromMessages([
        { labelKeys: [SYS.inbox, SYS.unread], isUnread: false },
        { labelKeys: ["l:category1"], isUnread: true },
      ]),
    ).toEqual(["l:category1", SYS.inbox, SYS.unread]);
    expect(threadKeysFromMessages([{ labelKeys: [SYS.unread], isUnread: false }])).toEqual([]);
  });

  it("aggregates newest-first participants, counts and list ids", () => {
    const a = threadAggregates([
      msg({ date: 2, from: { email: "Ada@Example.com", name: "Ada" }, snippet: "second", isUnread: true }),
      msg({ date: 1, from: { email: "sam@example.org" }, subject: "Offsite", listId: "news.example.com" }),
      msg({ date: 3, isDraft: true, isOutbound: true, snippet: "draft" }),
    ]);
    expect(a.subject).toBe("Offsite");
    expect(a.snippet).toBe("second");
    expect(a.participants).toEqual([{ name: "Ada", email: "ada@example.com" }, { email: "sam@example.org" }]);
    expect(a).toMatchObject({ messageCount: 2, unreadCount: 1, draftCount: 1, lastMessageAt: 2 });
    expect(a).toMatchObject({ isBulk: true, listId: "news.example.com", lastInboundAt: 2 });
    expect(a.lastOutboundAt).toBeUndefined();
  });

  it("caps search text", () => {
    expect(threadSearchText("s", [{ email: "a@b.c", name: "A" }], ["1", "2", "3", "4"])).toBe(
      "s\nA a@b.c\n2\n3\n4",
    );
    expect(threadSearchText("x".repeat(2000), [], []).length).toBe(1500);
  });
});
