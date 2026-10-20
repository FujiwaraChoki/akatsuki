import { describe, expect, it } from "vitest";
import { countLabel, displayName, fullDate, listDate, participantsLabel } from "../src/vm/format.ts";
import { shouldBuzz, swipeResult, swipeSides, swipeStage } from "../src/vm/swipe.ts";
import {
  countKeys,
  cycleTab,
  MAILBOXES,
  mailboxById,
  mailboxForShortcut,
  moveSelection,
  selectionAfterRemoval,
  splitTabs,
  type ThreadRowLike,
  threadRowModel,
} from "../src/vm/threadList.ts";

const NOW = Date.UTC(2026, 9, 20, 12, 0); // Tue
const ME = new Set(["me@example.com"]);

const row = (over: Partial<ThreadRowLike> = {}): ThreadRowLike => ({
  _id: "t1",
  accountId: "a1",
  subject: "Quarterly numbers",
  snippet: "Here are the numbers",
  participants: [{ name: "Ana Lopez", email: "ana@example.com" }],
  lastMessageAt: Date.UTC(2026, 9, 20, 9, 5),
  sortAt: Date.UTC(2026, 9, 20, 9, 5),
  messageCount: 1,
  unreadCount: 1,
  draftCount: 0,
  hasAttachments: false,
  labelKeys: ["sys:inbox", "sys:unread"],
  pendingOpCount: 0,
  ...over,
});

describe("format", () => {
  it("names senders", () => {
    expect(displayName({ email: "bob@x.io" })).toBe("bob");
    expect(displayName({ name: '"Bob B"', email: "bob@x.io" })).toBe("Bob B");
    expect(displayName(undefined)).toBe("Unknown sender");
    expect(participantsLabel([{ email: "me@example.com" }], undefined, 1, ME)).toBe("me");
    // One message: the sender only, even though the user is a participant.
    expect(
      participantsLabel(
        [{ name: "Ana Lopez", email: "ana@x" }, { email: "me@example.com" }],
        { name: "Ana Lopez", email: "ana@x" },
        1,
        ME,
      ),
    ).toBe("Ana Lopez");
    expect(
      participantsLabel(
        [
          { name: "Ana Lopez", email: "ana@x" },
          { email: "me@example.com" },
          { name: "Bo Li", email: "b@x" },
          { email: "z@x" },
        ],
        undefined,
        5,
        ME,
      ),
    ).toBe("Ana, me, Bo, … (5)");
  });

  it("formats list dates in the user's zone", () => {
    expect(listDate(Date.UTC(2026, 9, 20, 9, 5), NOW, "Europe/Zurich")).toBe("11:05");
    expect(listDate(Date.UTC(2026, 9, 17, 9, 5), NOW, "UTC")).toBe("Sat");
    expect(listDate(Date.UTC(2026, 2, 3, 9, 5), NOW, "UTC")).toBe("Mar 3");
    expect(listDate(Date.UTC(2025, 2, 3, 9, 5), NOW, "UTC")).toBe("2025-03-03");
    expect(fullDate(Date.UTC(2025, 2, 3, 9, 5), NOW, "UTC")).toBe("Mon, Mar 3 2025, 09:05");
  });

  it("caps counts at 99+", () => {
    expect(countLabel(0)).toBeNull();
    expect(countLabel(7)).toBe("7");
    expect(countLabel(100)).toBe("99+");
  });
});

describe("split tabs", () => {
  it("is one inbox tab without splits", () => {
    expect(splitTabs([], { "sys:inbox": 3 })).toEqual([{ key: "sys:inbox", title: "Inbox", count: "3" }]);
  });

  it("starts with the whole inbox, then splits in order, then Other", () => {
    const tabs = splitTabs(
      [
        { _id: "s2", name: "News", order: 2, showCount: false, labelKey: "split:s2" },
        { _id: "s1", name: "Important", order: 1, showCount: true, labelKey: "split:s1" },
      ],
      { "sys:inbox": 12, "split:s1": 4, "split:s2": 9, "split:other": 120 },
    );
    expect(tabs).toEqual([
      { key: "sys:inbox", title: "Inbox", count: "12" },
      { key: "split:s1", title: "Important", count: "4" },
      { key: "split:s2", title: "News", count: null },
      { key: "split:other", title: "Other", count: "99+" },
    ]);
    expect(countKeys(tabs)).toEqual(["sys:inbox", "split:s1", "split:s2", "split:other"]);
    expect(cycleTab(tabs, "split:other", 1)).toBe("sys:inbox");
    expect(cycleTab(tabs, "sys:inbox", -1)).toBe("split:other");
  });
});

describe("mailboxes", () => {
  it("maps go.* shortcuts to views", () => {
    expect(mailboxForShortcut("go.trash")?.labelKey).toBe("sys:trash");
    expect(mailboxById("nope").id).toBe("inbox");
    expect(new Set(MAILBOXES.map((m) => m.shortcut)).size).toBe(MAILBOXES.length);
  });
});

describe("thread row model", () => {
  it("summarises a row", () => {
    const m = threadRowModel(
      row({ labelKeys: ["sys:inbox", "sys:starred"], unreadCount: 0, pendingOpCount: 1 }),
      {
        now: NOW,
        timeZone: "UTC",
        selfEmails: ME,
      },
    );
    expect(m).toMatchObject({
      sender: "Ana Lopez",
      unread: false,
      starred: true,
      pending: true,
      date: "09:05",
    });
    expect(m.accessibilityLabel).toContain("From Ana Lopez.");
  });

  it("marks unread and shows snooze badges", () => {
    const m = threadRowModel(row({ subject: "  ", snoozedUntil: NOW + 1000 }), {
      now: NOW,
      timeZone: "UTC",
      selfEmails: ME,
      describe: () => "Tue 08:00",
    });
    expect(m.unread).toBe(true);
    expect(m.subject).toBe("(no subject)");
    expect(m.badge).toBe("Snoozed until Tue 08:00");
  });

  it("moves the selection", () => {
    expect(moveSelection(0, -1, 1)).toBe(-1);
    expect(moveSelection(3, -1, 1)).toBe(0);
    expect(moveSelection(3, -1, -1)).toBe(2);
    expect(moveSelection(3, 2, 1)).toBe(2);
    expect(selectionAfterRemoval(["a", "b"], 2)).toBe(1);
    expect(selectionAfterRemoval([], 0)).toBe(-1);
  });
});

describe("swipe actions", () => {
  it("archives inbox threads and snoozes or trashes them", () => {
    const s = swipeSides("sys:inbox", ["sys:inbox"]);
    expect(s.leading?.id).toBe("archive");
    expect(s.leading?.removes).toBe(true);
    expect(s.trailing.map((a) => a?.id)).toEqual(["snooze", "trash"]);
    expect(swipeResult(120, 400, s)?.id).toBe("archive");
    expect(swipeResult(-120, 400, s)?.id).toBe("snooze");
    expect(swipeResult(-300, 400, s)?.id).toBe("trash");
    expect(swipeResult(40, 400, s)).toBeNull();
  });

  it("restores from trash and spam", () => {
    const t = swipeSides("sys:trash", ["sys:trash"]);
    expect(t.leading?.id).toBe("untrash");
    expect(t.trailing).toEqual([null, null]);
    expect(swipeStage(-300, 400, t)).toBe(0);
    expect(swipeSides("sys:spam", ["sys:spam"]).leading?.id).toBe("notSpam");
  });

  it("moves archived threads back to the inbox without removing them from All mail", () => {
    const s = swipeSides("all", []);
    expect(s.leading).toEqual({ id: "moveToInbox", label: "Move to Inbox", removes: false });
  });

  it("buzzes on every stage change", () => {
    const s = swipeSides("split:s1", ["sys:inbox"]);
    expect(s.leading?.removes).toBe(true);
    expect(swipeStage(-100, 400, s)).toBe(1);
    expect(swipeStage(-260, 400, s)).toBe(2);
    expect(shouldBuzz(1, 2)).toBe(true);
    expect(shouldBuzz(2, 2)).toBe(false);
  });
});
