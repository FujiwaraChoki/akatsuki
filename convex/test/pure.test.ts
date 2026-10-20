// Pure logic: label keys, applyOps, rules, aggregates, query grammar, sanitizer, entitlement, email.
import { readdirSync, readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { checkBaseUrl, isLocalHost } from "../ai";
import { aiBudgetMicros, computeEntitlement, periodOf } from "../ee/entitlement";
import {
  ConsoleEmailSender,
  consoleTransportMode,
  emailSenderFromEnv,
  isLoopbackHost,
  redactForLog,
  renderEmail,
  SesEmailSender,
} from "../lib/email";
import {
  applyContextFor,
  applyOps,
  diffKeys,
  isLabelKey,
  matchRule,
  parseLabelKey,
  threadAggregates,
  threadKeysFromMessages,
} from "../lib/model";
import { parseQuery, zonedMidnight } from "../lib/query";
import { htmlToText, sanitizeUserHtml } from "../lib/sanitizeUserHtml";

const gmail = applyContextFor("gmail", []);
const graph = applyContextFor("graph", ["k57abc1234567890"]);

describe("label keys", () => {
  it("parses the grammar of §1.4", () => {
    expect(parseLabelKey("sys:inbox")).toEqual({ kind: "sys", role: "inbox" });
    expect(parseLabelKey("l:j57d0a8ahbx9kq3v0f1t0k2p1n7ca2e8")).toEqual({
      kind: "user",
      labelId: "j57d0a8ahbx9kq3v0f1t0k2p1n7ca2e8",
    });
    expect(parseLabelKey("split:other")).toEqual({ kind: "split", splitId: "other" });
    for (const bad of ["sys:archive", "inbox", "l:", "l:UPPER", "split:", "sys:", ""])
      expect(isLabelKey(bad)).toBe(false);
  });
});

describe("applyOps (§5.1)", () => {
  it("modify, trash, untrash on gmail", () => {
    expect(
      applyOps(["sys:inbox", "sys:unread"], [{ kind: "modify", add: [], remove: ["sys:inbox"] }], gmail),
    ).toEqual({
      keys: ["sys:unread"],
      deleted: false,
    });
    expect(applyOps(["sys:inbox", "sys:snoozed"], [{ kind: "trash" }], gmail).keys).toEqual(["sys:trash"]);
    expect(applyOps(["sys:trash"], [{ kind: "untrash" }], gmail).keys).toEqual(["sys:inbox"]);
  });
  it("move removes every folder key on folder providers", () => {
    const r = applyOps(["sys:inbox", "sys:unread"], [{ kind: "move", to: "l:k57abc1234567890" }], graph);
    expect(r.keys).toEqual(["l:k57abc1234567890", "sys:unread"]);
    expect(applyOps(["l:k57abc1234567890"], [{ kind: "trash" }], graph).keys).toEqual(["sys:trash"]);
  });
  it("delete_forever hides and unsubscribe does nothing", () => {
    expect(applyOps(["sys:inbox"], [{ kind: "delete_forever" }], graph)).toEqual({ keys: [], deleted: true });
    expect(applyOps(["sys:inbox"], [{ kind: "unsubscribe", method: "mailto" }], gmail).keys).toEqual([
      "sys:inbox",
    ]);
  });
  it("is order-sensitive and dedupes", () => {
    const actions = [
      { kind: "modify" as const, add: ["sys:starred"], remove: [] },
      { kind: "modify" as const, add: ["sys:starred"], remove: [] },
      { kind: "modify" as const, add: [], remove: ["sys:starred"] },
    ];
    expect(applyOps(["sys:inbox"], actions, gmail).keys).toEqual(["sys:inbox"]);
  });
  it("diffs keys for undo", () => {
    expect(diffKeys(["a", "b"], ["b", "c"])).toEqual({ added: ["c"], removed: ["a"] });
  });
});

describe("rules (§3.4)", () => {
  const facts = {
    fromEmail: "billing@mail.stripe.com",
    participants: ["me@x.com", "team@x.com"],
    labelKeys: ["sys:inbox"],
    triage: { category: "receipts", needsReply: false },
    isBulk: true,
    listId: "news.stripe.com",
    subject: "Your receipt from Acme",
    accountId: "acc1",
    vip: false,
  };
  it("matches all/any", () => {
    expect(
      matchRule(
        {
          match: "all",
          conds: [
            { f: "from_domain", eq: "stripe.com" },
            { f: "is_bulk", eq: true },
          ],
        },
        facts,
      ),
    ).toBe(true);
    expect(
      matchRule(
        {
          match: "all",
          conds: [
            { f: "from_domain", eq: "stripe.com" },
            { f: "vip", eq: true },
          ],
        },
        facts,
      ),
    ).toBe(false);
    expect(
      matchRule(
        {
          match: "any",
          conds: [
            { f: "subject_contains", text: "RECEIPT" },
            { f: "vip", eq: true },
          ],
        },
        facts,
      ),
    ).toBe(true);
    expect(matchRule({ match: "any", conds: [] }, facts)).toBe(false);
    expect(matchRule({ match: "all", conds: [{ f: "from_domain", eq: "tripe.com" }] }, facts)).toBe(false);
    expect(matchRule({ match: "all", conds: [{ f: "to_email", eq: "TEAM@x.com" }] }, facts)).toBe(true);
    expect(matchRule({ match: "all", conds: [{ f: "triage", category: "receipts" }] }, facts)).toBe(true);
    expect(
      matchRule({ match: "all", conds: [{ f: "needs_reply", eq: false }] }, { ...facts, triage: undefined }),
    ).toBe(false);
  });
});

describe("thread aggregates", () => {
  const m = (o: Record<string, unknown>) => ({
    to: [{ email: "me@x.com" }],
    cc: [],
    subject: "Hello",
    snippet: "s",
    date: 1,
    labelKeys: ["sys:inbox"],
    isUnread: false,
    isDraft: false,
    isOutbound: false,
    hasAttachments: false,
    ...o,
  });
  it("aggregates newest-first participants, counts and timestamps", () => {
    const a = threadAggregates([
      m({ from: { email: "A@x.com", name: "A" }, date: 1, snippet: "first" }),
      m({
        from: { email: "me@x.com" },
        date: 2,
        isOutbound: true,
        to: [{ email: "a@x.com" }],
        snippet: "reply",
      }),
      m({ from: { email: "me@x.com" }, date: 3, isDraft: true, snippet: "draft" }),
      m({
        from: { email: "b@x.com" },
        date: 4,
        isUnread: true,
        hasAttachments: true,
        listId: "l.x.com",
        snippet: "latest",
      }),
    ]);
    expect(a.snippet).toBe("latest");
    expect(a.lastFrom?.email).toBe("b@x.com");
    expect(a.participants.map((p) => p.email)).toEqual(["b@x.com", "me@x.com", "a@x.com"]);
    expect(a).toMatchObject({
      messageCount: 3,
      unreadCount: 1,
      draftCount: 1,
      hasAttachments: true,
      isBulk: true,
    });
    expect(a.lastInboundAt).toBe(4);
    expect(a.lastOutboundAt).toBe(2);
    expect(
      threadKeysFromMessages([
        m({ isUnread: true, labelKeys: ["sys:inbox", "sys:unread"] }),
        m({ labelKeys: ["sys:starred"] }),
      ]),
    ).toEqual(["sys:inbox", "sys:starred", "sys:unread"]);
  });
});

describe("query grammar (§11.1)", () => {
  const opts = { nowAt: Date.UTC(2026, 9, 20, 12), timezone: "Europe/Zurich" };
  it("parses words, phrases, fields, negation, OR and groups", () => {
    expect(parseQuery("", opts).ast).toEqual({ and: [] });
    expect(parseQuery("hello", opts).ast).toEqual({ text: "hello", phrase: false });
    expect(parseQuery('from:ada "q3 numbers" -is:read', opts).ast).toEqual({
      and: [
        { field: "from", value: "ada" },
        { text: "q3 numbers", phrase: true },
        { not: { field: "is", value: "read" } },
      ],
    });
    expect(parseQuery("a OR (b c)", opts).ast).toEqual({
      or: [
        { text: "a", phrase: false },
        {
          and: [
            { text: "b", phrase: false },
            { text: "c", phrase: false },
          ],
        },
      ],
    });
    expect(parseQuery("x:y FROM:z", opts).ast).toEqual({
      and: [
        { text: "x:y", phrase: false },
        { field: "from", value: "z" },
      ],
    });
    expect(parseQuery('subject:"weekly sync"', opts).ast).toEqual({ field: "subject", value: "weekly sync" });
  });
  it("resolves dates in the user's time zone and relative durations", () => {
    expect(parseQuery("before:2026-10-01", opts).ast).toEqual({
      field: "before",
      at: Date.UTC(2026, 8, 30, 22),
    });
    expect(parseQuery("after:2026/01/15", opts).ast).toEqual({
      field: "after",
      at: Date.UTC(2026, 0, 14, 23),
    });
    expect(parseQuery("older_than:2w", opts).ast).toEqual({ field: "older_than", days: 14 });
    expect(parseQuery("newer_than:1y", opts).ast).toEqual({ field: "newer_than", days: 365 });
    expect(parseQuery("before:soon", opts).errors).toEqual(["invalid date: soon"]);
    // D-094: before:/after: take dates only; relative values belong to older_than:/newer_than:.
    expect(parseQuery("before:7d", opts)).toEqual({
      ast: { text: "before:7d", phrase: false },
      errors: ["invalid date: 7d"],
    });
    // D-094: is:/has:/category:/built-in in: values are lowercased, other values keep their case.
    expect(parseQuery("is:Unread in:INBOX in:Newsletters", opts).ast).toEqual({
      and: [
        { field: "is", value: "unread" },
        { field: "in", value: "inbox" },
        { field: "in", value: "Newsletters" },
      ],
    });
    // D-094: groups nest at most 32 deep.
    expect(parseQuery(`${"(".repeat(40)}x${")".repeat(40)}`, opts).errors[0]).toBe(
      "groups nested too deeply at 32",
    );
    expect(zonedMidnight(2026, 7, 1, "America/New_York")).toBe(Date.UTC(2026, 6, 1, 4));
  });
});

describe("sanitizeUserHtml", () => {
  it("keeps formatting and safe links", () => {
    expect(sanitizeUserHtml('<p>Hi <b>there</b> <a href="https://x.com" onclick="evil()">x</a></p>')).toBe(
      '<p>Hi <b>there</b> <a href="https://x.com" target="_blank" rel="noopener noreferrer">x</a></p>',
    );
  });
  it("removes scripts, handlers, styles and dangerous URLs", () => {
    const out = sanitizeUserHtml(
      '<script>alert(1)</script><img src=x onerror=alert(1)><a href="java&#x09;script:alert(1)">y</a><style>p{}</style><iframe src="//e"></iframe><div style="position:fixed">z</div>',
    );
    expect(out).toBe("<a>y</a><div>z</div>");
    expect(sanitizeUserHtml("<scr<script>ipt>alert(1)</script>")).toBe("&lt;scr");
    expect(sanitizeUserHtml("a < b & c")).toBe("a &lt; b &amp; c");
    expect(sanitizeUserHtml("<b>unclosed")).toBe("<b>unclosed</b>");
    expect(sanitizeUserHtml('<a href="mailto:a@b.c">m</a>')).toContain('href="mailto:a@b.c"');
  });
  it("extracts text", () => {
    expect(htmlToText("<p>Hello</p><p>World &amp; you</p>")).toBe("Hello\nWorld & you");
  });
});

describe("entitlement (§13.3)", () => {
  afterEach(() => {
    delete process.env.AKATSUKI_BILLING;
    delete process.env.AKATSUKI_MANAGED_AI;
  });
  const now = Date.UTC(2026, 9, 20);
  const day = 86_400_000;
  it("maps Stripe status to entitlement", () => {
    expect(computeEntitlement({ status: "trialing", trialEnd: now + day }, now)).toEqual({
      state: "trialing",
      hostedSync: true,
      managedAi: true,
      until: now + 4 * day,
    });
    // At and after trialEnd the trial stays entitled until a Stripe snapshot moves it (D-411)…
    expect(computeEntitlement({ status: "trialing", trialEnd: now }, now)).toMatchObject({
      state: "trialing",
      hostedSync: true,
      managedAi: true,
      until: now + 3 * day,
    });
    expect(computeEntitlement({ status: "trialing", trialEnd: now - 2 * day }, now).hostedSync).toBe(true);
    // …for at most three days without one.
    expect(computeEntitlement({ status: "trialing", trialEnd: now - 3 * day }, now).state).toBe("lapsed");
    expect(computeEntitlement({ status: "trialing" }, now).state).toBe("lapsed");
    expect(computeEntitlement({ status: "active", currentPeriodEnd: now }, now)).toMatchObject({
      state: "active",
      until: now + 3 * day,
    });
    expect(computeEntitlement({ status: "past_due", pastDueSince: now - day }, now)).toMatchObject({
      state: "grace",
      until: now + 6 * day,
    });
    expect(computeEntitlement({ status: "past_due", pastDueSince: now - 8 * day }, now).state).toBe("lapsed");
    expect(computeEntitlement({ status: "canceled" }, now)).toMatchObject({
      state: "lapsed",
      hostedSync: false,
      until: 0,
    });
    expect(computeEntitlement({ status: "incomplete" }, now).state).toBe("none");
    expect(computeEntitlement(null, now).state).toBe("none");
  });
  it("self-host and budgets", () => {
    process.env.AKATSUKI_BILLING = "off";
    expect(computeEntitlement(null, now)).toMatchObject({
      state: "selfhost",
      hostedSync: true,
      managedAi: false,
    });
    process.env.AKATSUKI_MANAGED_AI = "on";
    expect(computeEntitlement(null, now).managedAi).toBe(true);
    expect(aiBudgetMicros({ state: "trialing", hostedSync: true, managedAi: true, until: 1 })).toBe(
      1_000_000,
    );
    expect(aiBudgetMicros({ state: "active", hostedSync: true, managedAi: true, until: 1 })).toBe(8_000_000);
    expect(periodOf(Date.UTC(2026, 0, 31, 23))).toBe("2026-01");
  });
});

describe("AI base URL rules", () => {
  it("allows https and local http only, never userinfo or query", () => {
    expect(() => checkBaseUrl("https://openrouter.ai/api/v1")).not.toThrow();
    expect(() => checkBaseUrl("http://localhost:11434")).not.toThrow();
    expect(() => checkBaseUrl("http://192.168.1.20:8080/v1")).not.toThrow();
    expect(() => checkBaseUrl("http://example.com/v1")).toThrow();
    expect(() => checkBaseUrl("https://user:pw@example.com")).toThrow();
    expect(() => checkBaseUrl("https://example.com/v1?key=1")).toThrow();
    expect(() => checkBaseUrl("https://example.com/v1#frag")).toThrow();
    expect(() => checkBaseUrl("http://10.attacker.example/v1")).toThrow();
    expect(() => checkBaseUrl("http://127.evil.net/v1")).toThrow();
  });

  // The same cases run against packages/ai `baseUrlProblem` and crates/ai `base_url_problem` (D-129).
  const shared = (path: string) =>
    JSON.parse(readFileSync(new URL(`../../packages/ai/fixtures/${path}`, import.meta.url), "utf8"));
  const fx = shared("base-url/cases.json") as { cases: { url: string; ok: boolean; local: boolean }[] };
  it.each(fx.cases.map((c) => [c.url, c] as const))("base-url/cases.json %s", (_url, c) => {
    let ok = true;
    try {
      checkBaseUrl(c.url);
    } catch {
      ok = false;
    }
    expect(ok).toBe(c.ok);
    let host = "";
    try {
      host = new URL(c.url).hostname;
    } catch {}
    expect(isLocalHost(host)).toBe(c.local);
  });

  // Settings fixtures that are invalid because of a baseUrl must be refused here too; valid ones must pass.
  const settingsDir = new URL("../../packages/ai/fixtures/settings/", import.meta.url);
  const settingsCases = readdirSync(settingsDir)
    .filter((f) => f.endsWith(".json"))
    .map((f) => [f, shared(`settings/${f}`)] as const)
    .filter(([f, c]) => c.valid || /^invalid_(base_url|http)_/.test(f));
  it.each(settingsCases)("settings/%s", (_f, c) => {
    const urls: string[] = c.settings.profiles.flatMap((p: { baseUrl?: string }) =>
      p.baseUrl ? [p.baseUrl] : [],
    );
    const refused = urls.some((u) => {
      try {
        checkBaseUrl(u);
        return false;
      } catch {
        return true;
      }
    });
    expect(refused).toBe(!c.valid);
  });
});

describe("email console transport (D-350)", () => {
  afterEach(() => vi.restoreAllMocks());
  const otp = renderEmail("otp", "person@example.com", { otp: "482913" });
  const link = renderEmail("magic_link", "person@example.com", {
    url: "https://x.convex.site/api/auth/magic-link/verify?token=abc123",
  });

  it("recognises loopback hosts only", () => {
    for (const h of ["localhost", "LOCALHOST", "127.0.0.1", "127.8.9.10", "[::1]", "::1"]) {
      expect(isLoopbackHost(h)).toBe(true);
    }
    for (const h of [
      "example.com",
      "app.localhost",
      "localhost.example.com",
      "127.evil.net",
      "128.0.0.1",
      "10.0.0.1",
      "127.0.0.256",
      "[::2]",
    ]) {
      expect(isLoopbackHost(h)).toBe(false);
    }
  });

  it("reveals on the local harness, redacts with the dev flag, refuses elsewhere", () => {
    expect(consoleTransportMode({ CONVEX_SITE_URL: "http://127.0.0.1:3411" })).toBe("reveal");
    expect(consoleTransportMode({ CONVEX_SITE_URL: "http://localhost:3211" })).toBe("reveal");
    expect(consoleTransportMode({ CONVEX_SITE_URL: "https://happy-otter-123.convex.site" })).toBe("refuse");
    expect(consoleTransportMode({})).toBe("refuse");
    expect(consoleTransportMode({ CONVEX_SITE_URL: "not a url" })).toBe("refuse");
    expect(
      consoleTransportMode({
        CONVEX_SITE_URL: "https://happy-otter-123.convex.site",
        AKATSUKI_DEV_HARNESS: "1",
      }),
    ).toBe("redact");
    expect(
      consoleTransportMode({
        CONVEX_SITE_URL: "https://happy-otter-123.convex.site",
        AKATSUKI_DEV_HARNESS: "true",
      }),
    ).toBe("refuse");
  });

  it("refuses EMAIL_TRANSPORT=console on a remote deployment", () => {
    expect(() =>
      emailSenderFromEnv({
        EMAIL_TRANSPORT: "console",
        CONVEX_SITE_URL: "https://happy-otter-123.convex.site",
      }),
    ).toThrow(/only allowed on the local harness/);
    expect(() => emailSenderFromEnv({ EMAIL_TRANSPORT: "console" })).toThrow(
      /only allowed on the local harness/,
    );
  });

  it("logs address, link and code on the local harness", async () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    const sender = emailSenderFromEnv({
      EMAIL_TRANSPORT: "console",
      CONVEX_SITE_URL: "http://127.0.0.1:3411",
    });
    expect((sender as ConsoleEmailSender).reveals).toBe(true);
    await sender.send(link);
    await sender.send(otp);
    const out = info.mock.calls.map((c) => String(c[0])).join("\n");
    expect(out).toContain("person@example.com");
    expect(out).toContain("token=abc123");
    expect(out).toContain("482913");
  });

  it("redacts address, link and code with the dev flag on a remote deployment", async () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    const sender = emailSenderFromEnv({
      EMAIL_TRANSPORT: "console",
      CONVEX_SITE_URL: "https://happy-otter-123.convex.site",
      AKATSUKI_DEV_HARNESS: "1",
    });
    expect((sender as ConsoleEmailSender).reveals).toBe(false);
    await sender.send(link);
    await sender.send(otp);
    const out = info.mock.calls.map((c) => String(c[0])).join("\n");
    expect(out).not.toContain("person@example.com");
    expect(out).not.toContain("example.com");
    expect(out).not.toContain("abc123");
    expect(out).not.toContain("convex.site");
    expect(out).not.toContain("482913");
    expect(out).toContain("to=<redacted>");
    expect(out).toContain("<code redacted>");
  });

  it("redacts by default when constructed directly", async () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    await new ConsoleEmailSender().send(otp);
    expect(String(info.mock.calls[0]?.[0])).not.toContain("482913");
    expect(redactForLog("see https://a.b/c?d=1 and 123456")).toBe("see <link redacted> and <code redacted>");
  });
});

describe("email", () => {
  afterEach(() => vi.unstubAllGlobals());
  it("renders templates without leaking HTML", () => {
    const e = renderEmail("magic_link", "a@b.c", { url: 'https://x.convex.site/verify?t="1"' });
    expect(e.subject).toBe("Your Akatsuki sign-in link");
    expect(e.html).toContain("&quot;1&quot;");
    expect(renderEmail("otp", "a@b.c", { otp: "123456" }).text).toContain("123456");
  });
  it("selects a transport from env and signs SES v2 requests", async () => {
    expect(
      emailSenderFromEnv({ EMAIL_TRANSPORT: "console", CONVEX_SITE_URL: "http://127.0.0.1:3411" }).name,
    ).toBe("console");
    expect(emailSenderFromEnv({ EMAIL_TRANSPORT: "none" }).name).toBe("none");
    expect(() => emailSenderFromEnv({})).toThrow(/SES is not configured/);
    expect(() => emailSenderFromEnv({ EMAIL_TRANSPORT: "" })).toThrow(/SES is not configured/);
    expect(() => emailSenderFromEnv({ EMAIL_TRANSPORT: "smtp" })).toThrow(/ses, console or none/);
    const calls: Request[] = [];
    vi.stubGlobal("fetch", async (req: Request) => {
      calls.push(req);
      return new Response("{}", { status: 200 });
    });
    const ses = new SesEmailSender({
      region: "eu-north-1",
      accessKeyId: "AKIDEXAMPLE",
      secretAccessKey: "secret",
      from: "Akatsuki <no-reply@akatsuki.vc>",
    });
    await ses.send(renderEmail("otp", "a@b.c", { otp: "1" }));
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe("https://email.eu-north-1.amazonaws.com/v2/email/outbound-emails");
    expect(calls[0]!.headers.get("authorization")).toMatch(
      /^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE\/\d{8}\/eu-north-1\/ses\/aws4_request/,
    );
    const body = await calls[0]!.json();
    expect(body.Destination.ToAddresses).toEqual(["a@b.c"]);
  });
});
