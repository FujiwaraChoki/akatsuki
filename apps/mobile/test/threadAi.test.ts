// Thread AI on mobile (D-408): the states and copy match web, the transcript comes from visible text, and the
// palette offers the AI commands in a thread plus the app-level rows everywhere.
import { describe, expect, it } from "vitest";
import { mobileCommands } from "../src/vm/keyboard.ts";
import { appPaletteItems } from "../src/vm/palette.ts";
import { htmlToText, THREAD_AI_COPY, threadAiStatus, threadForAi, visibleText } from "../src/vm/threadAi.ts";

const consent = { autoTriage: false, syncVoiceProfile: false, grantedAt: 1, version: 1 };
const byo = {
  profileId: "p1",
  kind: "openrouter" as const,
  label: "OpenRouter",
  models: { default: "anthropic/claude-sonnet-5.5" },
};
const settings = (over: Record<string, unknown> = {}) =>
  ({
    profiles: [byo],
    routes: [{ feature: "default", profileId: "p1", enabled: true }],
    consent,
    ...over,
  }) as never;
const env = (over: Record<string, unknown> = {}) => ({
  settings: settings(),
  managedAi: false,
  apiConfigured: true,
  hasKey: () => true,
  ...over,
});

describe("thread AI status", () => {
  it("is ready with consent, a route and a key on this device", () => {
    expect(threadAiStatus("summarize", env()).status).toBe("ready");
  });
  it("names what is missing, with the web copy", () => {
    expect(
      threadAiStatus(
        "summarize",
        env({ settings: settings({ consent: { ...consent, grantedAt: undefined } }) }),
      ).status,
    ).toBe("consent");
    expect(threadAiStatus("summarize", env({ hasKey: () => false })).status).toBe("no_key");
    expect(THREAD_AI_COPY.consent).toBe("AI is off. Turn it on in Settings, AI.");
    expect(THREAD_AI_COPY.no_key).toBe("Key not set on this device. Add it in Settings, AI.");
  });
});

describe("transcript input", () => {
  it("uses the visible part of each body and falls back to the snippet", () => {
    expect(
      htmlToText(
        '<div class="akatsuki-body"><style>p{}</style><p>Hi&nbsp;Ana</p><p>Thanks &amp; bye</p></div>',
      ),
    ).toBe("Hi Ana\nThanks & bye");
    const body = { html: null, text: "New text\n> old", quoted: { htmlStart: null, textStart: 9 } };
    expect(visibleText(body, "snip")).toBe("New text");
    expect(visibleText(null, "snip")).toBe("snip");
    const t = threadForAi(
      [{ from: { email: "a@x.org" }, to: [], cc: [], date: 1, snippet: "s" }],
      () => body,
      "Europe/Zurich",
    );
    expect(t).toEqual({
      timezone: "Europe/Zurich",
      messages: [{ from: { email: "a@x.org" }, to: [], cc: [], date: 1, text: "New text" }],
    });
  });
});

describe("palette", () => {
  it("offers Summarize and Suggest replies in a thread", () => {
    const ids = mobileCommands("mac", "superhuman", new Set(["thread"])).map((c) => c.id);
    expect(ids).toContain("ai.summarize");
    expect(ids).toContain("ai.instantReply");
    expect(mobileCommands("mac", "superhuman", new Set(["list"])).map((c) => c.id)).not.toContain(
      "ai.summarize",
    );
  });
  it("has app-level rows: other themes, density, settings pages and sign out", () => {
    const rows = appPaletteItems({ theme: "dark", density: "comfortable" }).map((r) => r.id);
    expect(rows).toEqual([
      "app.theme.light",
      "app.theme.system",
      "app.density.compact",
      "app.settings",
      "app.settings.ai",
      "app.settings.shortcuts",
      "app.mailboxes",
      "app.signOut",
    ]);
  });
});
