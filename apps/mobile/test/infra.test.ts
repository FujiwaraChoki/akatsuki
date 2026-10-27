import { SHORTCUTS } from "@akatsuki/shortcuts";
import { color, tokens } from "@akatsuki/tokens";
import { describe, expect, it } from "vitest";
import { FONT_FILES, fontFamily, makeTheme, resolveMode, textStyle } from "../src/theme/theme.ts";
import { readEnv } from "../src/vm/env.ts";
import { UUID_RE, uuidv4, uuidv7 } from "../src/vm/ids.ts";
import {
  createMobileMatcher,
  keyPlatform,
  MOBILE_SHORTCUTS,
  MOBILE_UNSUPPORTED,
  mobileCommands,
  toKeyEventLike,
} from "../src/vm/keyboard.ts";
import { bodySize, isFresh, MemoryLru, planEviction } from "../src/vm/lru.ts";
import { createOutbox, isTransient, memoryOutboxStorage, OUTBOX_MAX_AGE_MS } from "../src/vm/outbox.ts";
import {
  CREATE_LABEL_ID,
  filterPalette,
  fuzzyScore,
  movePaletteCursor,
  withCreateLabel,
} from "../src/vm/palette.ts";
import {
  insertOperator,
  OPERATOR_HINTS,
  searchModel,
  searchNow,
  unsupportedNotice,
} from "../src/vm/search.ts";

const seq = (n: number) => Uint8Array.from({ length: n }, (_, i) => (i * 37 + 11) & 0xff);

describe("ids", () => {
  it("mints RFC 9562 v7 and v4 ids", () => {
    const id = uuidv7(Date.UTC(2026, 9, 20), seq);
    expect(id).toMatch(UUID_RE);
    expect(id[14]).toBe("7");
    expect("89ab").toContain(id[19]!);
    expect(id.slice(0, 13).replace("-", "")).toBe(Date.UTC(2026, 9, 20).toString(16).padStart(12, "0"));
    const v4 = uuidv4(seq);
    expect(v4).toMatch(UUID_RE);
    expect(v4[14]).toBe("4");
    expect(uuidv7(1, seq) < uuidv7(2, seq)).toBe(true);
  });
});

describe("env", () => {
  it("accepts https and local http, and reports what is missing", () => {
    const ok = readEnv({
      EXPO_PUBLIC_CONVEX_URL: "https://x.convex.cloud",
      EXPO_PUBLIC_CONVEX_SITE_URL: "http://192.168.1.5:3411/",
      EXPO_PUBLIC_SYNC_URL: "https://sync.akatsuki.vc",
      EXPO_PUBLIC_API_URL: "https://api.akatsuki.vc",
      EXPO_PUBLIC_LOGIN_PROVIDERS: "google, Microsoft,github,google",
    });
    expect(ok.env).toEqual({
      convexUrl: "https://x.convex.cloud",
      convexSiteUrl: "http://192.168.1.5:3411",
      syncUrl: "https://sync.akatsuki.vc",
      apiUrl: "https://api.akatsuki.vc",
      loginProviders: ["google", "microsoft"],
    });
    const bad = readEnv({
      EXPO_PUBLIC_CONVEX_URL: "http://example.com",
      EXPO_PUBLIC_API_URL: "https://u:p@api.x",
    });
    expect(bad.env).toBeNull();
    expect(bad.missing).toEqual([
      "EXPO_PUBLIC_CONVEX_URL",
      "EXPO_PUBLIC_CONVEX_SITE_URL",
      "EXPO_PUBLIC_SYNC_URL",
      "EXPO_PUBLIC_API_URL",
    ]);
  });
});

describe("keyboard", () => {
  it("adapts native events", () => {
    expect(toKeyEventLike({ key: "KeyE", character: "e" })).toMatchObject({ key: "e", code: "KeyE" });
    expect(toKeyEventLike({ key: "Digit3", character: null, shiftKey: true })).toMatchObject({ key: "#" });
    expect(toKeyEventLike({ key: "ArrowDown" }).key).toBe("ArrowDown");
    expect(toKeyEventLike({ key: "KeyK", character: "\u000b", metaKey: true })).toMatchObject({
      key: "k",
      metaKey: true,
    });
    expect(keyPlatform("ios")).toBe("mac");
    expect(keyPlatform("android")).toBe("linux");
  });

  it("matches shared bindings, including sequences, and ignores what mobile lacks", () => {
    const m = createMobileMatcher("mac", "superhuman");
    const list = new Set(["list"] as const);
    expect(m.handle(toKeyEventLike({ key: "KeyE", character: "e" }), list)).toEqual({
      kind: "match",
      id: "thread.archive",
    });
    expect(m.handle(toKeyEventLike({ key: "KeyG", character: "g" }), list).kind).toBe("pending");
    expect(m.handle(toKeyEventLike({ key: "KeyI", character: "i" }), list)).toEqual({
      kind: "match",
      id: "go.inbox",
    });
    expect(m.handle(toKeyEventLike({ key: "KeyK", character: "k", metaKey: true }), list)).toEqual({
      kind: "match",
      id: "palette.open",
    });
    expect(m.handle(toKeyEventLike({ key: "KeyX", character: "x" }), list)).toEqual({
      kind: "match",
      id: "select.toggle",
    });
    // Shortcuts mobile lacks are ignored (Open original has no key; checked through the unsupported list).
    expect(MOBILE_UNSUPPORTED.has("thread.openOriginal")).toBe(true);
    // Single letters do nothing while typing in the composer.
    expect(
      m.handle(toKeyEventLike({ key: "KeyE", character: "e" }), new Set(["compose"] as const)).kind,
    ).toBe("none");
  });

  it("covers every shared shortcut one way or the other", () => {
    for (const s of SHORTCUTS) expect(MOBILE_SHORTCUTS.has(s.id) !== MOBILE_UNSUPPORTED.has(s.id)).toBe(true);
    for (const id of MOBILE_SHORTCUTS) expect(SHORTCUTS.some((s) => s.id === id)).toBe(true);
  });

  it("lists palette commands for the active scope", () => {
    const list = mobileCommands("mac", "superhuman", new Set(["list"]));
    const ids = list.map((c) => c.id);
    expect(ids).toContain("thread.archive");
    expect(ids).toContain("go.inbox");
    expect(ids).not.toContain("compose.send");
    expect(ids).toContain("select.all");
    expect(list.find((c) => c.id === "settings.open")?.hint).toBe("⌘,");
    const compose = mobileCommands("linux", "superhuman", new Set(["compose"])).map((c) => c.id);
    expect(compose).toContain("compose.send");
    expect(compose).not.toContain("thread.archive");
  });
});

describe("palette", () => {
  const items = [
    { id: "a", label: "Archive", group: "Thread", hint: "E" },
    { id: "b", label: "Go to Inbox", group: "Navigation", hint: "G I" },
    { id: "c", label: "Mark read or unread", group: "Thread", hint: "U" },
  ];
  it("ranks prefix and word matches", () => {
    expect(filterPalette(items, "").map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(filterPalette(items, "inb").map((i) => i.id)).toEqual(["b"]);
    expect(filterPalette(items, "ar")[0]!.id).toBe("a");
    expect(filterPalette(items, "mru").map((i) => i.id)).toEqual(["c"]);
    expect(fuzzyScore("zz", "Archive")).toBeNull();
    expect(movePaletteCursor(3, 2, 1)).toBe(0);
    expect(movePaletteCursor(3, 0, -1)).toBe(2);
  });
});

describe("search", () => {
  it("rounds now and builds chips", () => {
    expect(searchNow(Date.UTC(2026, 9, 20, 12, 3, 45, 9))).toBe(Date.UTC(2026, 9, 20, 12, 3));
    const m = searchModel(
      'from:ana is:unread -"weekly report" before:2026-10-01',
      Date.UTC(2026, 9, 20),
      "UTC",
    );
    expect(m.chips).toEqual([
      { label: "from: ana", negated: false },
      { label: "is: unread", negated: false },
      { label: '"weekly report"', negated: true },
      { label: "before: 2026-10-01", negated: false },
    ]);
    expect(m.errors).toEqual([]);
    expect(searchModel("   ", 0, "UTC").empty).toBe(true);
    expect(searchModel("is:sideways", 0, "UTC").errors.length).toBeGreaterThan(0);
    expect(unsupportedNotice([])).toBeNull();
    expect(unsupportedNotice(["filename:"])).toContain("Search the full mailbox");
    expect(insertOperator("budget ", OPERATOR_HINTS[0]!.insert)).toBe("budget from:");
  });
});

describe("outbox", () => {
  it("forgets calls that resolved and keeps transient failures for replay", async () => {
    const storage = memoryOutboxStorage();
    let t = 1000;
    const ob = createOutbox(storage, () => t);
    await expect(ob.run("mail.apply", "op1", { a: 1 }, async () => "ok")).resolves.toBe("ok");
    expect(await ob.pending()).toEqual([]);
    await expect(
      ob.run("mail.apply", "op2", { a: 2 }, async () => {
        throw new Error("Network request failed");
      }),
    ).rejects.toThrow();
    await expect(
      ob.run("mail.apply", "op3", {}, async () => {
        throw Object.assign(new Error("bad"), { data: { code: "invalid_action", retryable: false } });
      }),
    ).rejects.toThrow();
    expect((await ob.pending()).map((e) => e.clientOpId)).toEqual(["op2"]);
    t += 1;
    const seen: string[] = [];
    const r = await ob.replay(async (e) => {
      seen.push(e.clientOpId);
      return "done";
    }, t);
    expect(seen).toEqual(["op2"]);
    expect(r).toEqual({ replayed: 1, dropped: 0, kept: 0 });
  });

  it("drops stale entries and counts attempts", async () => {
    const storage = memoryOutboxStorage();
    await storage.put({ clientOpId: "old", fn: "mail.apply", args: {}, createdAt: 0, attempts: 1 });
    await storage.put({
      clientOpId: "new",
      fn: "mail.snooze",
      args: {},
      createdAt: OUTBOX_MAX_AGE_MS,
      attempts: 1,
    });
    const ob = createOutbox(storage);
    const r = await ob.replay(async () => "retry", OUTBOX_MAX_AGE_MS + 10);
    expect(r).toEqual({ replayed: 0, dropped: 1, kept: 1 });
    expect((await storage.all())[0]!.attempts).toBe(2);
    expect(isTransient({ retryable: true })).toBe(true);
    expect(isTransient(new Error("boom"))).toBe(false);
  });
});

describe("body cache policy", () => {
  it("evicts least recently used entries over the cap", () => {
    const entries = [
      { key: "a", size: 50, lastAccess: 3 },
      { key: "b", size: 50, lastAccess: 1 },
      { key: "c", size: 50, lastAccess: 2 },
    ];
    expect(planEviction(entries, 200)).toEqual([]);
    expect(planEviction(entries, 60)).toEqual(["b", "c"]);
    expect(isFresh(3, 3)).toBe(true);
    expect(isFresh(2, 3)).toBe(false);
    expect(bodySize({ html: "ab", text: null })).toBe(260);
    const lru = new MemoryLru<string>(10);
    lru.set("x", "1", 6);
    lru.set("y", "2", 6);
    expect(lru.get("x")).toBeUndefined();
    expect(lru.get("y")).toBe("2");
    expect(lru.bytes).toBe(6);
  });
});

describe("theme", () => {
  it("maps tokens to React Native styles", () => {
    const light = makeTheme("light");
    const dark = makeTheme("dark");
    expect(light.colors.background).toBe(color.light.background);
    expect(dark.colors.accent).toBe(color.dark.accent);
    // First Light (D-431): text on accent fills is white in both themes; neutrals are fg over bg, opaque.
    expect(dark.colors.onAccent).toBe(color.dark.onAccent);
    expect(light.colors.surface).toBe("#F6F6F6");
    expect(light.colors.surfaceStrong).toBe("#EFEFEF");
    expect(dark.colors.surface).toBe("#131313");
    expect(dark.colors.tertiary).toBe("#A9A9A9");
    expect(light.colors.dawn).toBe(dark.colors.dawn);
    // Dark accent text is lifted to about 5.5:1 on the background (D-301): 50/50 with the foreground.
    expect(dark.colors.accentText).toBe(color.dark.accentText);
    expect(color.dark.accentText).toBe("#B77777");
    expect(light.colors.accentText).toBe(color.light.accent);
    expect(light.colors.muted).toBe("#747474");
    // Group headers and the mono crumb are both sentence case with no tracking (spec §2.1).
    expect(light.text.label).not.toHaveProperty("textTransform");
    expect(light.text.crumb).not.toHaveProperty("textTransform");
    expect(light.text.crumb).toMatchObject({ fontSize: 11, letterSpacing: 0 });
    expect(light.text.crumb.fontFamily).toBe("GeistMono-Medium");
    expect(light.text.body.fontSize).toBe(15);
    expect(light.text.reading).toMatchObject({ fontSize: 16, lineHeight: 25 });
    expect(light.text.subject.fontFamily).toBe("InstrumentSerif");
    // The weight comes from the tokens (First Light: 500 for strong body text, D-482).
    expect(textStyle("bodyStrong").fontFamily).toBe(fontFamily("sans", tokens.text.app.bodyStrong.weight));
    expect(fontFamily("serif", 700)).toBe("InstrumentSerif");
    expect(resolveMode("system", "dark")).toBe("dark");
    expect(resolveMode("light", "dark")).toBe("light");
    expect(resolveMode(undefined, null)).toBe("light");
    for (const file of Object.values(FONT_FILES)) expect(file).toMatch(/\.ttf$/);
  });

  it("keeps native chrome colours in app.config.ts equal to the tokens", async () => {
    const { NATIVE_THEME } = await import("../app.config.ts");
    expect(NATIVE_THEME.LIGHT_BACKGROUND).toBe(tokens.color.light.background);
    expect(NATIVE_THEME.DARK_BACKGROUND).toBe(tokens.color.dark.background);
    expect(NATIVE_THEME.ACCENT).toBe(tokens.color.light.accent);
    expect(NATIVE_THEME.IOS_BUNDLE_ID).toBe("vc.akatsuki.app");
    expect(NATIVE_THEME.ANDROID_PACKAGE).toBe("vc.akatsuki.app");
  });
});

describe("label picker", () => {
  const labels = [
    { id: "l:a", label: "Clients", group: "me@x.io", hint: null },
    { id: "l:b", label: "Receipts", group: "me@x.io", hint: null },
  ];
  it("offers to create a label the query does not name exactly", () => {
    expect(withCreateLabel(filterPalette(labels, "Projects"), labels, "Projects").map((i) => i.id)).toEqual([
      CREATE_LABEL_ID,
    ]);
    expect(withCreateLabel([], labels, "Projects")[0]!.label).toBe("Create label “Projects”");
    expect(withCreateLabel(filterPalette(labels, "clients"), labels, "clients").map((i) => i.id)).toEqual([
      "l:a",
    ]);
    expect(withCreateLabel(labels, labels, "  ")).toHaveLength(2);
  });
});
