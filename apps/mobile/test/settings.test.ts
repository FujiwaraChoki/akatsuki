import { EMPTY_AI_SETTINGS } from "@akatsuki/ai";
import { describe, expect, it } from "vitest";
import {
  connectionTest,
  consentText,
  defaultProfile,
  draftProblems,
  egressView,
  keyStatus,
  keyStorageName,
  modelRequired,
  newDraft,
  profileFromDraft,
  removeProfile,
  settingsOrEmpty,
  upsertProfile,
  withKeyOnDevice,
} from "../src/vm/aiSettings.ts";
import {
  FULL_MODE_WARNING,
  needsWarning,
  notificationTarget,
  wantsPushToken,
} from "../src/vm/notifications.ts";
import { planStatus } from "../src/vm/plan.ts";

describe("AI provider settings", () => {
  it("labels Ollama on the LAN as staying on the local network", () => {
    const d = { ...newDraft("p1", "ollama"), baseUrl: "http://192.168.1.20:11434", model: "llama3.3" };
    expect(draftProblems(d, { managedAiAllowed: false })).toEqual([]);
    const p = profileFromDraft(d);
    expect(egressView(p)).toEqual({
      egress: "network",
      text: "Stays on your local network (192.168.1.20:11434)",
    });
    expect(egressView({ kind: "ollama", baseUrl: "http://nas.local:11434" }).egress).toBe("network");
    expect(egressView({ kind: "ollama", baseUrl: "https://ollama.example.com" }).text).toBe(
      "Leaves this device: the mail you run AI on, sent to ollama.example.com",
    );
    expect(connectionTest(p)).toEqual({ kind: "http_get", url: "http://192.168.1.20:11434/api/tags" });
  });

  it("requires a network address for Ollama on a phone", () => {
    const missing = draftProblems({ ...newDraft("p1", "ollama"), model: "m" }, { managedAiAllowed: false });
    expect(missing[0]).toContain("A phone does not run Ollama itself.");
    const loop = draftProblems(
      { ...newDraft("p1", "ollama"), baseUrl: "http://127.0.0.1:11434", model: "m" },
      { managedAiAllowed: false },
    );
    expect(loop).toEqual([
      "localhost is this phone. Use the address of the computer on your network instead.",
    ]);
    const http = draftProblems(
      { ...newDraft("p1", "openai_compatible"), baseUrl: "http://example.com/v1", model: "m" },
      { managedAiAllowed: false },
    );
    expect(http[0]).toMatch(/^The address must use https/);
  });

  it("gates managed AI on the plan and asks for models where routing has none", () => {
    expect(draftProblems(newDraft("p1", "akatsuki"), { managedAiAllowed: false })).toEqual([
      "Akatsuki AI needs an active Akatsuki Pro plan on this account.",
    ]);
    expect(draftProblems(newDraft("p1", "akatsuki"), { managedAiAllowed: true })).toEqual([]);
    expect(modelRequired("openrouter")).toBe(false);
    expect(modelRequired("openai")).toBe(true);
    expect(draftProblems(newDraft("p2", "openai"), { managedAiAllowed: true })).toEqual(["Choose a model."]);
    expect(egressView({ kind: "akatsuki" }).text).toBe("Sent to Akatsuki for processing, not stored");
    expect(egressView({ kind: "anthropic" }).text).toBe(
      "Leaves this device: the mail you run AI on, sent to api.anthropic.com",
    );
  });

  it("never puts a key in the synced profile and tracks devices", () => {
    const p = profileFromDraft({ ...newDraft("p1", "anthropic"), label: "Claude" });
    expect(Object.keys(p).sort()).toEqual(["keyOnDevices", "kind", "label", "models", "profileId"]);
    const withKey = withKeyOnDevice(p, "dev-1", true);
    expect(withKey.keyOnDevices).toEqual(["dev-1"]);
    expect(withKeyOnDevice(withKey, "dev-1", false).keyOnDevices).toEqual([]);
    expect(keyStatus(withKey, "dev-1", true)).toBe("on_this_device");
    expect(keyStatus(withKey, "dev-2", false)).toBe("on_other_devices");
    expect(keyStatus(p, "dev-2", false)).toBe("missing");
    expect(keyStorageName("p1")).toBe("ai.p1");
    expect(() => keyStorageName("../x")).toThrow();
  });

  it("makes a profile the default route and removes it again", () => {
    const p = profileFromDraft({ ...newDraft("p1", "openrouter"), label: "OR" });
    const s1 = { ...EMPTY_AI_SETTINGS, ...upsertProfile(EMPTY_AI_SETTINGS, p, true) };
    expect(s1.routes).toEqual([{ feature: "default", profileId: "p1", enabled: true }]);
    expect(defaultProfile(s1)?.profileId).toBe("p1");
    const s2 = { ...s1, ...removeProfile(s1, "p1") };
    expect(s2).toMatchObject({ profiles: [], routes: [] });
    expect(settingsOrEmpty(null)).toBe(EMPTY_AI_SETTINGS);
  });

  it("names the provider in the consent text", () => {
    expect(consentText({ kind: "ollama", baseUrl: "http://10.0.0.5:11434" })).toContain(
      "Stays on your local network (10.0.0.5:11434)",
    );
  });
});

describe("notifications", () => {
  it("warns before full mode and routes taps", () => {
    expect(needsWarning("private", "full")).toBe(true);
    expect(needsWarning("full", "full")).toBe(false);
    expect(needsWarning("full", "private")).toBe(false);
    expect(FULL_MODE_WARNING.body).toMatch(/Expo.*Apple \(APNs\).*Google/);
    expect(wantsPushToken("off")).toBe(false);
    expect(notificationTarget({ threadId: "jd7abc123def456" })).toEqual({
      pathname: "/thread/[threadId]",
      params: { threadId: "jd7abc123def456" },
    });
    expect(notificationTarget({ threadId: "../../x" })).toEqual({ pathname: "/inbox" });
    expect(notificationTarget(null)).toEqual({ pathname: "/inbox" });
  });
});

describe("plan status", () => {
  const now = Date.UTC(2026, 9, 20, 12);
  it("shows the plan without selling it", () => {
    const trial = planStatus(
      { state: "trialing", hostedSync: true, managedAi: true, until: now + 86_400_000 * 3 },
      { status: "trialing", trialEnd: Date.UTC(2026, 9, 23, 12), cancelAtPeriodEnd: false },
      now,
      "UTC",
    );
    expect(trial).toEqual({
      title: "Akatsuki Pro trial",
      detail: "Trial ends Fri 12:00.",
      mailAvailable: true,
      managedAi: true,
    });
    const none = planStatus(undefined, undefined, now, "UTC");
    expect(none.title).toBe("No active plan");
    expect(JSON.stringify(none)).not.toMatch(/\$|price|subscribe|upgrade|buy/i);
    const cancelling = planStatus(
      { state: "active", hostedSync: true, managedAi: true, until: 0 },
      { status: "active", currentPeriodEnd: Date.UTC(2026, 11, 1), cancelAtPeriodEnd: true },
      now,
      "UTC",
    );
    expect(cancelling.detail).toBe("Ends Dec 1, 00:00; it will not renew.");
  });
});
