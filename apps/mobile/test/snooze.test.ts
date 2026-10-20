import { describe, expect, it } from "vitest";
import {
  customTime,
  dayOptions,
  describeTime,
  sendLaterPresets,
  snoozePresets,
  TIME_SLOTS,
} from "../src/vm/snooze.ts";
import { addDays, wallTime, zonedInstant } from "../src/vm/zoned.ts";

const ZRH = "Europe/Zurich";
const NY = "America/New_York";
// Tuesday 2026-10-20 10:15 in Zurich (UTC+2, CEST).
const TUE = Date.UTC(2026, 9, 20, 8, 15);

describe("zoned time", () => {
  it("reads wall time in a zone", () => {
    expect(wallTime(TUE, ZRH)).toMatchObject({
      year: 2026,
      month: 10,
      day: 20,
      hour: 10,
      minute: 15,
      weekday: 2,
    });
    expect(wallTime(TUE, NY)).toMatchObject({ day: 20, hour: 4, minute: 15 });
  });

  it("converts wall time to an instant across DST", () => {
    // Zurich switches to CET on 2026-10-25 at 03:00 → 02:00.
    expect(zonedInstant(2026, 10, 24, 8, 0, ZRH)).toBe(Date.UTC(2026, 9, 24, 6, 0));
    expect(zonedInstant(2026, 10, 26, 8, 0, ZRH)).toBe(Date.UTC(2026, 9, 26, 7, 0));
    // Spring gap: 2026-03-29 02:30 does not exist in Zurich; it moves forward.
    const gap = zonedInstant(2026, 3, 29, 2, 30, ZRH);
    expect(wallTime(gap, ZRH).hour).toBe(3);
    // Fold: 2026-10-25 02:30 happens twice; the earlier instant wins.
    expect(zonedInstant(2026, 10, 25, 2, 30, ZRH)).toBe(Date.UTC(2026, 9, 25, 0, 30));
  });

  it("falls back to UTC for an unknown zone", () => {
    expect(zonedInstant(2026, 1, 1, 0, 0, "Mars/Olympus")).toBe(Date.UTC(2026, 0, 1));
  });

  it("adds days across month ends", () => {
    expect(addDays({ year: 2026, month: 12, day: 31 }, 1)).toMatchObject({ year: 2027, month: 1, day: 1 });
  });
});

describe("snooze presets", () => {
  it("offers Superhuman's set on a weekday morning", () => {
    const p = snoozePresets(TUE, ZRH);
    expect(p.map((x) => x.id)).toEqual([
      "later_today",
      "tomorrow",
      "this_weekend",
      "next_week",
      "next_month",
    ]);
    const by = Object.fromEntries(p.map((x) => [x.id, x]));
    expect(by.later_today!.at).toBe(Date.UTC(2026, 9, 20, 12, 0)); // 14:00 local
    expect(by.tomorrow!.at).toBe(Date.UTC(2026, 9, 21, 6, 0)); // Wed 08:00 CEST
    expect(by.this_weekend!.at).toBe(Date.UTC(2026, 9, 24, 6, 0)); // Sat 08:00
    expect(by.next_week!.at).toBe(Date.UTC(2026, 9, 26, 7, 0)); // Mon 08:00 CET
    expect(by.next_month!.at).toBe(Date.UTC(2026, 10, 20, 7, 0));
    expect(by.tomorrow!.hint).toBe("Tomorrow 08:00");
  });

  it("drops later today in the evening and this weekend on weekends", () => {
    const satEvening = Date.UTC(2026, 9, 24, 18, 30); // 20:30 Sat
    const ids = snoozePresets(satEvening, ZRH).map((x) => x.id);
    expect(ids).not.toContain("later_today");
    expect(ids).not.toContain("this_weekend");
    expect(ids).toContain("next_week");
    // Next week from Saturday is the coming Monday.
    const nw = snoozePresets(satEvening, ZRH).find((x) => x.id === "next_week")!;
    expect(wallTime(nw.at, ZRH)).toMatchObject({ day: 26, weekday: 1, hour: 8 });
  });

  it("next week from a Monday is the following Monday", () => {
    const mon = Date.UTC(2026, 9, 19, 7, 0);
    const nw = snoozePresets(mon, ZRH).find((x) => x.id === "next_week")!;
    expect(wallTime(nw.at, ZRH).day).toBe(26);
  });

  it("clamps next month to the month's length", () => {
    const jan31 = Date.UTC(2026, 0, 31, 9, 0);
    const nm = snoozePresets(jan31, "UTC").find((x) => x.id === "next_month")!;
    expect(wallTime(nm.at, "UTC")).toMatchObject({ month: 2, day: 28 });
  });

  it("every preset is in the future", () => {
    for (let h = 0; h < 24; h++) {
      const now = Date.UTC(2026, 9, 20, h, 40);
      for (const p of [...snoozePresets(now, NY), ...sendLaterPresets(now, NY)])
        expect(p.at).toBeGreaterThan(now);
    }
  });
});

describe("send later and custom times", () => {
  it("rounds 'in one hour' to five minutes", () => {
    const p = sendLaterPresets(TUE, ZRH);
    expect(p[0]!.at).toBe(Date.UTC(2026, 9, 20, 9, 15));
    expect(p.map((x) => x.id)).toEqual(["in_one_hour", "tomorrow", "tomorrow_afternoon", "next_week"]);
  });

  it("lists days and validates custom times", () => {
    const days = dayOptions(TUE, ZRH, 3);
    expect(days.map((d) => d.label)).toEqual(["Today", "Tomorrow", "Thu 22 Oct"]);
    expect(TIME_SLOTS).toHaveLength(48);
    expect(customTime(days[0]!, 9 * 60, TUE, ZRH)).toEqual({
      ok: false,
      error: "Pick a time in the future.",
    });
    expect(customTime(days[1]!, 9 * 60 + 30, TUE, ZRH)).toEqual({
      ok: true,
      at: Date.UTC(2026, 9, 21, 7, 30),
    });
  });

  it("describes times relative to now", () => {
    expect(describeTime(Date.UTC(2026, 9, 20, 16, 0), TUE, ZRH)).toBe("Today 18:00");
    expect(describeTime(Date.UTC(2026, 9, 23, 6, 0), TUE, ZRH)).toBe("Fri 08:00");
    expect(describeTime(Date.UTC(2026, 11, 1, 7, 0), TUE, ZRH)).toBe("Dec 1, 08:00");
  });
});
