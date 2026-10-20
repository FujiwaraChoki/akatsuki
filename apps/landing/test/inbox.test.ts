// The demo inbox's pure selectors and renderers: the server's first paint and the client's
// re-renders share them, so they are the contract the keyboard demo runs on.
import { describe, expect, it } from "vitest";
import { THREADS } from "@/lib/inbox-fixture";
import {
  current,
  inboxCount,
  initialState,
  matches,
  plural,
  rowLabel,
  rowsHtml,
  splitCount,
  visible,
} from "@/lib/inbox-render";
import { comboCaps, sequenceCaps } from "@/lib/shortcuts";

describe("demo inbox", () => {
  it("starts on the Important split with the first thread selected", () => {
    const s = initialState();
    expect(s.split).toBe("Important");
    expect(current(s)?.id).toBe("t1");
    expect(visible(s).every((t) => t.split === "Important" && t.loc === "inbox")).toBe(true);
    expect(inboxCount(s)).toBe(splitCount(s, "Important"));
  });

  it("drops an archived thread from the list and the count", () => {
    const s = initialState();
    const before = inboxCount(s);
    const t1 = s.threads.find((t) => t.id === "t1");
    if (t1) t1.loc = "archived";
    expect(inboxCount(s)).toBe(before - 1);
    expect(visible(s).some((t) => t.id === "t1")).toBe(false);
  });

  it("filters with search operators", () => {
    const t = initialState().threads;
    expect(t.filter((x) => matches(x, "has:attachment")).every((x) => x.clip)).toBe(true);
    expect(t.filter((x) => matches(x, "is:unread")).every((x) => x.unread)).toBe(true);
    expect(t.some((x) => matches(x, "from:hana"))).toBe(true);
  });

  it("labels rows for screen readers and renders them as listbox options", () => {
    const s = initialState();
    const first = s.threads[0];
    expect(first && rowLabel(first)).toMatch(/^Hana Mori, Offsite agenda for Thursday, 09:41, unread/);
    expect(rowsHtml(s)).toContain('role="option"');
    expect(plural(1, "key", "keys")).toBe("1 key");
    expect(plural(3, "key", "keys")).toBe("3 keys");
  });

  it("uses only fictional people (no real addresses)", () => {
    for (const t of THREADS) expect(t.addr).not.toMatch(/samihindi|gmail\.com|outlook\.com/);
  });
});

describe("keycaps", () => {
  it("prints the platform's modifiers", () => {
    expect(comboCaps("cmd+k", "mac")).toEqual(["⌘", "K"]);
    expect(comboCaps("shift+h", "other")).toEqual(["Shift", "H"]);
    expect(sequenceCaps("g i", "other")).toEqual([["G"], ["I"]]);
  });
});
