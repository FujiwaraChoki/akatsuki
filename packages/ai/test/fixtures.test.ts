// Shared fixtures that crates/ai runs too: resolve, settings, transcript, triage items, routing table.

import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { type FeatureId, getFeature, renderPrompt } from "@akatsuki/prompts";
import { describe, expect, it } from "vitest";
import {
  composeVars,
  extractActionsVars,
  instantReplyVars,
  summarizeVars,
  transcriptBudget,
} from "../src/features.ts";
import { resolveFeature } from "../src/resolve.ts";
import {
  AI_FEATURES,
  PROVIDER_KINDS,
  type ProviderKind,
  ROUTING,
  tableModel,
  tierFor,
} from "../src/routing.ts";
import { streamFeature, varsChars } from "../src/run.ts";
import {
  baseUrlProblem,
  type EgressFeature,
  egressFor,
  egressText,
  isLocalHost,
  zAiSettings,
} from "../src/settings.ts";
import { settingsJsonSchema } from "../src/settings-schema.ts";
import { isRenderWhitespace, renderTranscript, renderTriageItems } from "../src/transcript.ts";
import { validateVars } from "../src/validate.ts";
import { drain, jsonPieces, mockModel } from "./helpers.ts";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const promptsShared = join(root, "../prompts/shared/fixtures");
const load = (dir: string) =>
  readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => [f.replace(/\.json$/, ""), JSON.parse(readFileSync(join(dir, f), "utf8"))] as const);

describe("routing.json", () => {
  it("routes every vAiFeature to its §12.1 tier", () => {
    expect(Object.keys(ROUTING.features).sort()).toEqual([...AI_FEATURES].sort());
    expect(tierFor("triage")).toBe("fast");
    expect(tierFor("nl_search")).toBe("fast");
    expect(tierFor("summarize")).toBe("default");
    expect(tierFor("dictation")).toBe("transcription");
  });

  it("uses the decided models: Sonnet 5.5 default, Haiku 5.5 fast, Opus 5.5 heavy", () => {
    expect(tableModel("default", "akatsuki")).toBe("anthropic/claude-sonnet-5.5");
    expect(tableModel("default", "openrouter")).toBe("anthropic/claude-sonnet-5.5");
    expect(tableModel("default", "anthropic")).toBe("claude-sonnet-5-5");
    expect(tableModel("fast", "akatsuki")).toBe("anthropic/claude-haiku-5.5");
    expect(tableModel("fast", "anthropic")).toBe("claude-haiku-5-5");
    expect(tableModel("heavy", "openrouter")).toBe("anthropic/claude-opus-5.5");
    expect(tableModel("transcription", "openai")).toBe("gpt-4o-transcribe");
    expect(tableModel("default", "ollama")).toBeUndefined();
    expect(ROUTING.escalation.heavy?.anthropic).toEqual(["claude-opus-5-5", "claude-fable-5-1"]);
  });

  it("has a list price for every managed model", () => {
    for (const tier of ["fast", "default", "heavy", "transcription"] as const) {
      const model = tableModel(tier, "akatsuki")!;
      expect(ROUTING.pricing[model], model).toBeDefined();
    }
  });
});

describe("fixtures/resolve", () => {
  it.each(load(join(root, "fixtures/resolve")))("%s", (_name, fx) => {
    const r = resolveFeature(fx.feature, fx.settings, fx.tier ? { tier: fx.tier } : {});
    const got =
      r.status === "ok"
        ? {
            status: r.status,
            tier: r.tier,
            profileId: r.profile.profileId,
            model: r.model,
            managed: r.managed,
            egress: r.egress,
          }
        : r.status === "error"
          ? { status: r.status, code: r.code, profileId: r.profileId }
          : { status: r.status };
    expect(got).toEqual(fx.expect);
  });
});

describe("fixtures/base-url/cases.json (same cases as convex/ai.ts and crates/ai, D-129)", () => {
  const fx = JSON.parse(readFileSync(join(root, "fixtures/base-url/cases.json"), "utf8")) as {
    cases: { url: string; ok: boolean; local: boolean }[];
  };
  it.each(fx.cases.map((c) => [c.url, c] as const))("%s", (_url, c) => {
    expect(baseUrlProblem(c.url) === null).toBe(c.ok);
    let host = "";
    try {
      host = new URL(c.url).hostname;
    } catch {}
    expect(isLocalHost(host)).toBe(c.local);
  });
});

describe("fixtures/settings", () => {
  it.each(load(join(root, "fixtures/settings")))("%s", (_name, fx) => {
    const parsed = zAiSettings.safeParse(fx.settings);
    expect(parsed.success, JSON.stringify(parsed.error?.issues)).toBe(fx.valid);
  });

  it("schema/ai-settings.schema.json is up to date (run `pnpm --filter @akatsuki/ai build`)", () => {
    const onDisk = JSON.parse(readFileSync(join(root, "schema/ai-settings.schema.json"), "utf8"));
    expect(onDisk).toEqual(settingsJsonSchema());
  });
});

describe("fixtures/egress/cases.json (same cases as crates/ai, §12.2)", () => {
  const fx = JSON.parse(readFileSync(join(root, "fixtures/egress/cases.json"), "utf8")) as {
    cases: {
      profile: { kind: ProviderKind; baseUrl?: string };
      feature: EgressFeature;
      egress: string;
      text: string;
    }[];
  };
  it("covers every feature, every label and every provider kind", () => {
    expect(new Set(fx.cases.map((c) => c.feature))).toEqual(new Set([...AI_FEATURES, "default"]));
    expect(new Set(fx.cases.map((c) => c.egress))).toEqual(
      new Set(["device", "network", "provider", "akatsuki", "akatsuki_openai"]),
    );
    expect(new Set(fx.cases.map((c) => c.profile.kind))).toEqual(new Set(PROVIDER_KINDS));
  });
  it.each(fx.cases.map((c) => [`${c.profile.kind} ${c.profile.baseUrl ?? ""} ${c.feature}`, c] as const))(
    "%s",
    (_name, c) => {
      expect(egressFor(c.profile, c.feature)).toBe(c.egress);
      expect(egressText(c.profile, c.feature)).toBe(c.text);
    },
  );
  it("never calls managed dictation zero-retention or 'not stored' (D-128)", () => {
    const text = egressText({ kind: "akatsuki" }, "dictation");
    expect(text).toContain("OpenAI");
    expect(text).not.toMatch(/not stored|zero/i);
  });
});

describe("transcript renderer (prompts/shared/fixtures/transcript)", () => {
  it.each(load(join(promptsShared, "transcript")))("%s", (_name, fx) => {
    expect(renderTranscript(fx.input)).toBe(fx.expected);
  });

  it("never exceeds maxInputChars", () => {
    const long = { from: { email: "a@example.com" }, to: [], date: 0, text: "x".repeat(9000) };
    for (const max of [10, 100, 5000, 20000]) {
      const out = renderTranscript({ timezone: "UTC", maxInputChars: max, messages: [long, long, long] });
      expect(Array.from(out).length).toBeLessThanOrEqual(max);
    }
  });
});

describe("transcript budget at the cap (prompts/shared/fixtures/at-cap)", () => {
  type Case = {
    vars: Record<string, string | null>;
    expected: {
      budget: number;
      transcriptChars: number;
      varsChars: number;
      userChars: number;
      userSha256: string;
    };
  };
  const fixtures = load(join(promptsShared, "at-cap"));

  it.each(fixtures)("%s: the unbudgeted transcript would not fit", (_name, fx) => {
    const t = renderTranscript(fx.thread);
    expect(Array.from(t).length).toBe(fx.unbudgetedTranscriptChars);
    for (const [feature, c] of Object.entries<Case>(fx.features))
      expect(varsChars({ ...c.vars, transcript: t })).toBeGreaterThan(
        getFeature(feature as FeatureId).meta.maxInputChars,
      );
  });

  it.each(fixtures)("%s: every transcript feature fits and renders identically", (_name, fx) => {
    for (const [name, c] of Object.entries<Case>(fx.features)) {
      const feature = name as FeatureId;
      const budget = transcriptBudget(feature, c.vars);
      const transcript = renderTranscript({ ...fx.thread, maxInputChars: budget });
      const vars = { ...c.vars, transcript };
      expect(validateVars(feature, vars).ok, feature).toBe(true);
      const user = renderPrompt(feature, vars).user;
      const got = {
        budget,
        transcriptChars: Array.from(transcript).length,
        varsChars: varsChars(vars),
        userChars: Array.from(user).length,
        userSha256: createHash("sha256").update(user, "utf8").digest("hex"),
      };
      expect(got, feature).toEqual(c.expected);
      expect(got.varsChars).toBeLessThanOrEqual(getFeature(feature).meta.maxInputChars);
    }
  });

  it.each(fixtures)("%s: the feature functions use the same budget", (_name, fx) => {
    const now = new Date("2026-10-20T10:00:00Z");
    const f = fx.features as Record<string, Case>;
    const transcriptOf = (feature: FeatureId) =>
      renderTranscript({ ...fx.thread, maxInputChars: f[feature]!.expected.budget });
    const userName = f.summarize!.vars.userName!;
    expect(summarizeVars({ thread: fx.thread, userName, now })).toEqual({
      ...f.summarize!.vars,
      transcript: transcriptOf("summarize"),
    });
    expect(
      instantReplyVars({ thread: fx.thread, userName, voice: f.instant_reply!.vars.voice, now }),
    ).toEqual({
      ...f.instant_reply!.vars,
      transcript: transcriptOf("instant_reply"),
    });
    expect(extractActionsVars({ thread: fx.thread, now })).toEqual({
      ...f.extract_actions!.vars,
      transcript: transcriptOf("extract_actions"),
    });
    const c = f.compose!.vars;
    expect(composeVars({ instruction: c.instruction!, userName: c.userName!, thread: fx.thread })).toEqual({
      ...c,
      transcript: transcriptOf("compose"),
    });
  });

  it.each(fixtures)("%s: streamFeature accepts the budgeted vars", async (_name, fx) => {
    const now = new Date("2026-10-20T10:00:00Z");
    const { model, captured } = mockModel(jsonPieces({ tldr: "t", bullets: [], asks: [], deadlines: [] }));
    const events = await drain(
      streamFeature({
        feature: "summarize",
        vars: summarizeVars({ thread: fx.thread, userName: "Sam Example", now }),
        choice: { model, modelId: "m", kind: "openrouter", tier: "default" },
      }),
    );
    expect(events[0]!.event).toBe("meta");
    expect(events.at(-1)!.event).toBe("done");
    expect(captured.calls).toHaveLength(1);
  });
});

describe("renderer whitespace set (prompts/shared/fixtures/whitespace.json, same check in crates/ai)", () => {
  it("is exactly Unicode White_Space plus U+FEFF, for every code point", () => {
    const fx = JSON.parse(readFileSync(join(promptsShared, "whitespace.json"), "utf8")) as {
      codePoints: string[];
    };
    const want = new Set(fx.codePoints.map((c) => Number.parseInt(c.slice(2), 16)));
    const got: number[] = [];
    for (let cp = 0; cp <= 0x10ffff; cp++) {
      if (cp >= 0xd800 && cp <= 0xdfff) continue;
      if (isRenderWhitespace(String.fromCodePoint(cp))) got.push(cp);
    }
    expect(got).toEqual([...want].sort((a, b) => a - b));
    expect(want.has(0xfeff) && want.has(0x85)).toBe(true);
  });
});

describe("triage items renderer", () => {
  it.each(load(join(promptsShared, "triage-items")))("%s", (_name, fx) => {
    const r = renderTriageItems(fx.input);
    expect({ text: r.text, keys: Object.fromEntries(r.keys) }).toEqual(fx.expected);
  });

  it("refuses more than 25 items", () => {
    const item = { threadId: "t", to: [], subject: "", snippet: "", isBulk: false, hasAttachments: false };
    expect(() => renderTriageItems(Array.from({ length: 26 }, () => item))).toThrow(RangeError);
  });
});
