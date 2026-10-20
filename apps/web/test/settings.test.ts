// Settings logic: the rule language editor model (§3.1 vRuleCond), filter actions per provider, sender blocks
// (D-286), the unsubscribe plan, the billing copy (§13) and the data export.
import { describe, expect, it, vi } from "vitest";
import type { LabelView, MailAccountView, RuleView } from "../src/backend/types.ts";
import { blockSender } from "../src/senders/block.ts";
import { unsubscribePlan } from "../src/senders/extension.tsx";
import {
  defaultCond,
  fromThreadAction,
  normaliseRule,
  ruleProblem,
  toThreadAction,
  type UiAction,
} from "../src/settings/ruleModel.ts";
import { describePlan } from "../src/settings/sections/Billing.tsx";
import { buildExport } from "../src/settings/sections/Devices.tsx";

const gmail = { _id: "acc1", provider: "gmail", email: "you@example.com" } as unknown as MailAccountView;
const imap = { _id: "acc2", provider: "imap", email: "you@example.org" } as unknown as MailAccountView;
const labels = [
  {
    _id: "lbl1",
    accountId: "acc1",
    name: "Clients",
    kind: "user",
    exclusive: false,
    labelKey: "l:lbl1",
    hidden: false,
    pendingDelete: false,
  },
  {
    _id: "arch",
    accountId: "acc2",
    name: "Archive",
    kind: "folder",
    role: "archive",
    exclusive: true,
    labelKey: "l:arch",
    hidden: false,
    pendingDelete: false,
  },
  {
    _id: "fold",
    accountId: "acc2",
    name: "Projects",
    kind: "folder",
    exclusive: true,
    labelKey: "l:fold",
    hidden: false,
    pendingDelete: false,
  },
] as unknown as LabelView[];

describe("rule model", () => {
  it("validates names, values and addresses", () => {
    const ctx = { labels, accounts: [gmail] };
    expect(ruleProblem("", { match: "all", conds: [] })).toBe("Give it a name.");
    expect(ruleProblem("VIP", { match: "all", conds: [] })).toBe("Add at least one condition.");
    expect(ruleProblem("VIP", { match: "all", conds: [defaultCond("from_email", ctx)] })).toBe(
      "Fill in “From address”.",
    );
    expect(ruleProblem("VIP", { match: "all", conds: [{ f: "from_email", eq: "nope" }] })).toBe(
      "nope is not an email address.",
    );
    expect(
      ruleProblem("VIP", {
        match: "any",
        conds: [
          { f: "from_domain", eq: "@example.com" },
          { f: "vip", eq: true },
        ],
      }),
    ).toBeNull();
  });
  it("normalises what the server compares lowercased", () => {
    expect(
      normaliseRule({
        match: "all",
        conds: [
          { f: "from_email", eq: " Mara@Example.com " },
          { f: "from_domain", eq: "@Example.COM" },
          { f: "list_id", eq: "<news.example.com>" },
        ],
      }).conds,
    ).toEqual([
      { f: "from_email", eq: "mara@example.com" },
      { f: "from_domain", eq: "example.com" },
      { f: "list_id", eq: "news.example.com" },
    ]);
  });
  it("keeps the text when switching between text fields", () => {
    const ctx = { labels, accounts: [gmail] };
    expect(defaultCond("subject_contains", ctx, { f: "from_domain", eq: "example.com" })).toEqual({
      f: "subject_contains",
      text: "example.com",
    });
  });
  it("builds provider-specific actions and reads them back", () => {
    const cases: [UiAction, MailAccountView][] = [
      [{ type: "archive" }, gmail],
      [{ type: "archive" }, imap],
      [{ type: "read" }, gmail],
      [{ type: "star" }, gmail],
      [{ type: "trash" }, imap],
      [{ type: "spam" }, gmail],
      [{ type: "label", labelId: "lbl1" }, gmail],
      [{ type: "move", folderKey: "l:fold" }, imap],
    ];
    for (const [ui, account] of cases) {
      const t = toThreadAction(ui, account, labels);
      expect(fromThreadAction(t, labels)).toEqual(ui);
    }
    expect(toThreadAction({ type: "archive" }, imap, labels)).toEqual({ kind: "move", to: "l:arch" });
    expect(toThreadAction({ type: "archive" }, gmail, labels)).toEqual({
      kind: "modify",
      add: [],
      remove: ["sys:inbox"],
    });
    expect(fromThreadAction({ kind: "untrash" }, labels)).toEqual({
      type: "custom",
      action: { kind: "untrash" },
    });
  });
});

describe("sender blocks", () => {
  const m = () => ({
    setSenderPref: vi.fn(async () => null),
    saveRule: vi.fn(async () => ({ ruleId: "r1" as never })),
    removeRule: vi.fn(async () => null),
  });
  it("records the block and adds a trash rule for new mail", async () => {
    const mm = m();
    await blockSender("Spam@Example.com", true, { m: mm, rules: [] });
    expect(mm.setSenderPref).toHaveBeenCalledWith({ sender: "spam@example.com", blocked: true });
    expect(mm.saveRule).toHaveBeenCalledWith({
      name: "Blocked: spam@example.com",
      enabled: true,
      order: -1,
      rule: { match: "all", conds: [{ f: "from_email", eq: "spam@example.com" }] },
      actions: [{ kind: "trash" }],
    });
  });
  it("blocks a domain and unblocks by removing the rule", async () => {
    const mm = m();
    await blockSender("@example.net", true, { m: mm, rules: [] });
    expect((mm.saveRule.mock.calls as unknown as unknown[][])[0]![0]).toMatchObject({
      rule: { conds: [{ f: "from_domain", eq: "example.net" }] },
    });
    const rules = [{ _id: "r9", name: "Blocked: @example.net" }] as unknown as RuleView[];
    await blockSender("@example.net", false, { m: mm, rules });
    expect(mm.removeRule).toHaveBeenCalledWith({ ruleId: "r9" });
  });
});

describe("unsubscribe", () => {
  const body = (lu: unknown) => ({ headers: { listUnsubscribe: lu } }) as never;
  it("prefers RFC 8058 one-click, then mailto, then an https page", () => {
    expect(unsubscribePlan(body({ http: "https://x.example/u", oneClick: true }))?.method).toBe(
      "one_click_post",
    );
    expect(
      unsubscribePlan(body({ mailto: "mailto:u@x.example", http: "https://x.example/u", oneClick: false }))
        ?.method,
    ).toBe("mailto");
    expect(unsubscribePlan(body({ http: "https://x.example/u", oneClick: false }))).toMatchObject({
      method: "link",
      url: "https://x.example/u",
    });
    expect(unsubscribePlan(body({ http: "http://x.example/u", oneClick: false }))).toBeNull();
    expect(unsubscribePlan(body(null))).toBeNull();
  });
  it("shows where a mailto unsubscribe goes and warns when it is not the sender's domain", () => {
    const own = unsubscribePlan(
      body({ mailto: "mailto:leave@lists.shop.example?subject=hi", oneClick: false }),
      {
        email: "news@shop.example",
      },
    );
    expect(own).toMatchObject({ method: "mailto", to: "leave@lists.shop.example", foreign: false });
    expect(own?.label).toContain("leave@lists.shop.example");
    const foreign = unsubscribePlan(
      body({ mailto: "mailto:ceo@victim-corp.example?subject=I%20quit", oneClick: false }),
      {
        email: "promo@spoofed.example",
      },
    );
    expect(foreign).toMatchObject({ method: "mailto", to: "ceo@victim-corp.example", foreign: true });
    expect(foreign?.label).toMatch(/not at the sender's domain/);
    expect(foreign?.label).not.toContain("I quit");
  });
});

describe("billing copy", () => {
  const info = (state: string, billing: object = {}) =>
    ({
      entitlement: { state, hostedSync: true, managedAi: true, until: 0 },
      billing: { status: "none", cancelAtPeriodEnd: false, ...billing },
      aiUsage: { period: "2026-10", usedMicros: 0, limitMicros: 0 },
    }) as never;
  it("names the plan and what happens next", () => {
    expect(describePlan(info("none"), "UTC")).toEqual({
      title: "No plan",
      detail: "Akatsuki Pro is $20 a month after a 14-day free trial.",
    });
    expect(describePlan(info("trialing", { trialEnd: Date.UTC(2026, 10, 3, 12) }), "UTC").detail).toMatch(
      /^Your trial ends .*Nov.*3.*Then \$20 a month\.$/,
    );
    expect(
      describePlan(
        info("active", { currentPeriodEnd: Date.UTC(2026, 10, 3), cancelAtPeriodEnd: true }),
        "UTC",
      ).detail,
    ).toMatch(/It will not renew/);
    expect(describePlan(info("selfhost"), "UTC").title).toBe("Self-hosted");
    expect(describePlan(info("lapsed"), "UTC").detail).toMatch(/30 days/);
  });
});

describe("export", () => {
  it("is JSON with a format marker and no secrets", () => {
    const json = buildExport({
      profile: { email: "you@example.com" },
      aiSettings: { profiles: [{ keyOnDevices: ["d1"] }] },
    });
    const v = JSON.parse(json);
    expect(v.format).toBe("akatsuki-export");
    expect(v.profile.email).toBe("you@example.com");
    expect(json).not.toMatch(/sk-|apiKey|password/i);
  });
});
