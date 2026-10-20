// Pure modules: routes, virtual window, ids, env, search suggestions, IMAP discovery, snooze times, outbox.
import { describe, expect, it, vi } from "vitest";
import { formatRoute, labelKeyOfView, parseRoute, type Route } from "../src/app/routes.ts";
import { parseLoginProviders, readEnv } from "../src/env.ts";
import { UUID_V7, uuidv7 } from "../src/lib/ids.ts";
import { memoryStore } from "../src/lib/kv.ts";
import { Outbox, type OutboxEntry } from "../src/lib/outbox.ts";
import { applySuggestion, lastToken, suggest } from "../src/mail/searchSuggest.ts";
import { parseSnoozeInput, snoozePresets, wallToUtc, zonedParts } from "../src/mail/snoozeTimes.ts";
import { nearEnd, scrollToReveal, windowFor } from "../src/mail/virtual.ts";
import { discoverImap, emailDomain, guess, parseDiscovery } from "../src/onboarding/imapDiscovery.ts";

describe("routes", () => {
  const cases: [string, Route][] = [
    ["/", { name: "mail", view: { kind: "inbox" } }],
    ["/inbox", { name: "mail", view: { kind: "inbox" } }],
    ["/inbox/split/abc123", { name: "mail", view: { kind: "inbox", split: "split:abc123" } }],
    ["/inbox/split/other", { name: "mail", view: { kind: "inbox", split: "split:other" } }],
    ["/starred/t/thr1", { name: "mail", view: { kind: "sys", name: "starred" }, threadId: "thr1" }],
    ["/label/l1?account=acc1", { name: "mail", view: { kind: "label", labelId: "l1" }, accountId: "acc1" }],
    ["/reminders", { name: "mail", view: { kind: "reminders" } }],
    [
      "/search?q=from%3Amara+is%3Aunread&t=thr2",
      { name: "search", q: "from:mara is:unread", threadId: "thr2" },
    ],
    ["/signin", { name: "signin" }],
    ["/onboarding/imap", { name: "onboarding", step: "imap" }],
    ["/settings/billing", { name: "settings", section: "billing" }],
    ["/compose", { name: "compose" }],
  ];
  it.each(cases)("parses %s", (url, route) => {
    const [path, search = ""] = url.split("?");
    expect(parseRoute(path!, search ? `?${search}` : "")).toEqual(route);
  });
  it("round-trips through formatRoute", () => {
    for (const [, route] of cases) {
      const f = formatRoute(route);
      const [path, search = ""] = f.split("?");
      expect(parseRoute(path!, search ? `?${search}` : "")).toEqual(route);
    }
  });
  it("rejects unknown paths and bad ids", () => {
    expect(parseRoute("/nope").name).toBe("not_found");
    expect(parseRoute("/inbox/t/../../x").name).toBe("not_found");
    expect(parseRoute("/label/").name).toBe("not_found");
    expect(parseRoute("/inbox", "?account=<script>")).toEqual({ name: "mail", view: { kind: "inbox" } });
  });
  it("maps views to list label keys", () => {
    expect(labelKeyOfView({ kind: "inbox" })).toBe("sys:inbox");
    expect(labelKeyOfView({ kind: "inbox", split: "split:x" })).toBe("split:x");
    expect(labelKeyOfView({ kind: "sys", name: "all" })).toBe("all");
    expect(labelKeyOfView({ kind: "sys", name: "trash" })).toBe("sys:trash");
    expect(labelKeyOfView({ kind: "label", labelId: "abc" })).toBe("l:abc");
    expect(labelKeyOfView({ kind: "reminders" })).toBeNull();
  });
});

describe("virtual window", () => {
  it("renders only the visible rows plus overscan", () => {
    const w = windowFor(10_000, 44, 44 * 500, 880, 8);
    expect(w.start).toBe(492);
    expect(w.end).toBe(500 + 21 + 8);
    expect(w.height).toBe(440_000);
    expect(windowFor(0, 44, 0, 800)).toEqual({ start: 0, end: 0, height: 0 });
  });
  it("reveals the cursor with context rows", () => {
    expect(scrollToReveal(5, 44, 0, 880)).toBeNull();
    expect(scrollToReveal(30, 44, 0, 880)).toBe(31 * 44 + 88 - 880);
    expect(scrollToReveal(10, 44, 600, 880)).toBe(10 * 44 - 88);
  });
  it("asks for more near the end", () => {
    expect(nearEnd({ start: 0, end: 40, height: 0 }, 50)).toBe(true);
    expect(nearEnd({ start: 0, end: 20, height: 0 }, 50)).toBe(false);
  });
});

describe("ids", () => {
  it("mints sortable UUIDv7", () => {
    const ids = Array.from({ length: 200 }, () => uuidv7(1_760_000_000_000));
    for (const id of ids) expect(id).toMatch(UUID_V7);
    expect([...ids].sort()).toEqual(ids);
    expect(new Set(ids).size).toBe(200);
  });
});

describe("env", () => {
  it("defaults to the local harness in development only", () => {
    const dev = readEnv({ DEV: true, MODE: "development" });
    expect(dev.convexUrl).toBe("http://127.0.0.1:3410");
    expect(dev.convexSiteUrl).toBe("http://127.0.0.1:3411");
    const prod = readEnv({ DEV: false, MODE: "production" });
    expect(prod.convexUrl).toBeNull();
    expect(readEnv({ MODE: "production", VITE_CONVEX_URL: "https://x.convex.cloud/" }).convexUrl).toBe(
      "https://x.convex.cloud",
    );
  });
  it("parses login providers", () => {
    expect(parseLoginProviders("google, Microsoft,github,google")).toEqual(["google", "microsoft"]);
    expect(parseLoginProviders(undefined)).toEqual([]);
  });
});

describe("search suggestions", () => {
  const ctx = {
    contacts: [{ email: "mara@example.com", name: "Mara Okafor" }],
    labels: ["Receipts", "Big Clients"],
    splits: ["VIP", "News"],
  };
  it("offers operators, then values", () => {
    expect(suggest("is:un", ctx).map((s) => s.insert)).toEqual(["is:unread"]);
    expect(suggest("fr", ctx)[0]!.insert).toBe("from:");
    expect(suggest("from:ma", ctx)[0]!.insert).toBe("from:mara@example.com");
    expect(suggest("label:big", ctx)[0]!.insert).toBe('label:"Big Clients"');
    expect(suggest("in:v", ctx).map((s) => s.insert)).toContain("in:vip");
    expect(suggest("-has:a", ctx)[0]!.insert).toBe("-has:attachment");
  });
  it("offers the last search first in an empty field, and only there", () => {
    const withRecent = { ...ctx, recent: "from:lena" };
    expect(suggest("", withRecent)[0]).toEqual({
      label: "from:lena",
      insert: "from:lena",
      hint: "Last search",
    });
    expect(suggest("", withRecent)).toHaveLength(8);
    expect(suggest("", ctx)[0]!.insert).toBe("from:");
    expect(suggest("is:unread ", withRecent).some((s) => s.hint === "Last search")).toBe(false);
  });
  it("replaces only the token being typed", () => {
    expect(lastToken('subject:"two words" fr')).toEqual({ token: "fr", start: 20 });
    expect(applySuggestion("hello is:un", { label: "is:unread", insert: "is:unread" })).toBe(
      "hello is:unread ",
    );
    expect(applySuggestion("hello fr", { label: "from:", insert: "from:" })).toBe("hello from:");
  });
});

describe("IMAP discovery", () => {
  it("validates domains like the worker", () => {
    expect(emailDomain("a@Example.COM")).toBe("example.com");
    expect(emailDomain("a@localhost")).toBeNull();
    expect(emailDomain("a@10.0.0.1")).toBeNull();
    expect(emailDomain("a@x.internal")).toBeNull();
    expect(emailDomain("nope")).toBeNull();
  });
  it("uses the worker's answer when it has one", async () => {
    const d = await discoverImap("me@corp.example", async () => ({
      imap: { host: "mail.corp.example", port: 993, tls: "implicit" },
      smtp: { host: "mail.corp.example", port: 587, tls: "starttls" },
      username: "me",
      source: "autoconfig",
      appPasswordRequired: false,
    }));
    expect(d?.source).toBe("autoconfig");
    expect(d?.username).toBe("me");
  });
  it("falls back to a guess when the worker has no answer (D-396)", async () => {
    const failing = vi.fn(async () => {
      throw new Error("not_found");
    });
    expect(await discoverImap("me@icloud.com", failing)).toEqual(guess("me@icloud.com"));
    expect(failing).toHaveBeenCalledTimes(1);
    expect(await discoverImap("me@corp.example", null)).toEqual(guess("me@corp.example"));
    expect(guess("me@corp.example")?.smtp).toEqual({ host: "smtp.corp.example", port: 587, tls: "starttls" });
    expect(await discoverImap("bad", failing)).toBeNull();
    expect(failing).toHaveBeenCalledTimes(1);
  });
  it("rejects malformed worker answers", () => {
    expect(parseDiscovery({ imap: { host: "", port: 993, tls: "implicit" } })).toBeNull();
    expect(
      parseDiscovery({
        imap: { host: "a", port: 99999, tls: "implicit" },
        smtp: { host: "b", port: 1, tls: "starttls" },
      }),
    ).toBeNull();
    expect(parseDiscovery(null)).toBeNull();
  });
});

describe("snooze times", () => {
  const tz = "Europe/Zurich";
  // Wednesday 2026-10-21 10:00 in Zurich (UTC+2).
  const now = wallToUtc(2026, 10, 21, 10, 0, tz);
  it("converts wall time in a zone, across DST", () => {
    expect(new Date(now).toISOString()).toBe("2026-10-21T08:00:00.000Z");
    // Zurich leaves summer time on 2026-10-25: 08:00 is UTC+1 then.
    expect(new Date(wallToUtc(2026, 10, 26, 8, 0, tz)).toISOString()).toBe("2026-10-26T07:00:00.000Z");
    expect(zonedParts(now, tz)).toMatchObject({ y: 2026, m: 10, d: 21, h: 10, weekday: 3 });
  });
  it("builds presets", () => {
    const p = Object.fromEntries(snoozePresets(now, tz).map((x) => [x.id, new Date(x.at).toISOString()]));
    expect(p.later).toBe("2026-10-21T11:00:00.000Z");
    expect(p.tomorrow).toBe("2026-10-22T06:00:00.000Z");
    expect(p.weekend).toBe("2026-10-24T06:00:00.000Z");
    expect(p.next_week).toBe("2026-10-26T07:00:00.000Z");
    const evening = wallToUtc(2026, 10, 24, 19, 0, tz); // Saturday evening
    const ids = snoozePresets(evening, tz).map((x) => x.id);
    expect(ids).not.toContain("later");
    expect(ids).not.toContain("weekend");
  });
  it("parses typed times", () => {
    const iso = (s: string) => {
      const t = parseSnoozeInput(s, now, tz);
      return t === null ? null : new Date(t).toISOString();
    };
    expect(iso("3h")).toBe("2026-10-21T11:00:00.000Z");
    expect(iso("in 2 days")).toBe("2026-10-23T08:00:00.000Z");
    expect(iso("tomorrow 3pm")).toBe("2026-10-22T13:00:00.000Z");
    expect(iso("fri 9:30")).toBe("2026-10-23T07:30:00.000Z");
    expect(iso("wed")).toBe("2026-10-28T07:00:00.000Z");
    expect(iso("2026-11-02 14:00")).toBe("2026-11-02T13:00:00.000Z");
    expect(iso("9am")).toBe("2026-10-22T07:00:00.000Z");
    expect(iso("2020-01-01")).toBeNull();
    expect(iso("whenever")).toBeNull();
    expect(iso("25:00")).toBeNull();
  });
});

describe("durable outbox", () => {
  const entry = (s: ReturnType<typeof memoryStore<OutboxEntry>>) => s.entries();
  it("forgets an entry once the mutation resolves", async () => {
    const store = memoryStore<OutboxEntry>();
    const o = new Outbox(store);
    await o.run("apply", "op1", { a: 1 }, async () => "ok");
    expect(await entry(store)).toEqual([]);
  });
  it("keeps it on a transient failure and replays it in order", async () => {
    const store = memoryStore<OutboxEntry>();
    const o = new Outbox(store);
    const transient = Object.assign(new Error("x"), { data: { code: "rate_limited" } });
    await expect(o.run("apply", "op1", { n: 1 }, async () => Promise.reject(transient))).rejects.toBe(
      transient,
    );
    await expect(o.run("undo", "op2", { n: 2 }, async () => Promise.reject(transient))).rejects.toBe(
      transient,
    );
    expect((await o.pending()).map((e) => e.clientOpId)).toEqual(["op1", "op2"]);
    const calls: string[] = [];
    const m = {
      apply: async (a: unknown) => void calls.push(`apply:${JSON.stringify(a)}`),
      undo: async (a: unknown) => void calls.push(`undo:${JSON.stringify(a)}`),
      snooze: async () => {},
      unsnooze: async () => {},
    };
    expect(await o.replay(m as never)).toBe(2);
    expect(calls).toEqual(['apply:{"n":1}', 'undo:{"n":2}']);
    expect(await o.pending()).toEqual([]);
  });
  it("drops entries that can never succeed", async () => {
    const store = memoryStore<OutboxEntry>();
    const o = new Outbox(store);
    const permanent = Object.assign(new Error("x"), { data: { code: "not_found" } });
    await expect(o.run("apply", "op1", {}, async () => Promise.reject(permanent))).rejects.toBe(permanent);
    expect(await o.pending()).toEqual([]);
  });
});

describe("sender labels", () => {
  it("puts other people first and me last", async () => {
    const { sendersLabel } = await import("../src/lib/format.ts");
    const me = new Set(["you@example.com"]);
    const you = { name: "You", email: "you@example.com" };
    const mara = { name: "Mara Okafor", email: "mara@example.com" };
    const jun = { name: "Jun Tanaka", email: "jun@example.net" };
    const ledger = { name: "The Weekly Ledger", email: "digest@news.example.com" };
    expect(sendersLabel({ lastFrom: you, participants: [mara, you] }, me)).toBe("Mara Okafor, me");
    expect(sendersLabel({ lastFrom: ledger, participants: [ledger] }, me)).toBe("The Weekly Ledger");
    expect(sendersLabel({ lastFrom: mara, participants: [mara, jun, you] }, me)).toBe("Mara, Jun, me");
    expect(sendersLabel({ participants: [] }, me)).toBe("(no sender)");
  });
});
