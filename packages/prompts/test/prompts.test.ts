import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Ajv } from "ajv";
import { describe, expect, it } from "vitest";
import { checkOutputSchema, loadEvalFixtures, loadPromptPackFromDir, PACK_ROOT } from "../src/fs.ts";
import { renderGoldens } from "../src/goldens.ts";
import {
  escapeValue,
  FEATURE_IDS,
  getFeature,
  PROMPT_PACK,
  promptVersion,
  renderPrompt,
  renderTemplate,
  templateVariables,
} from "../src/index.ts";

const json = (p: string) => JSON.parse(readFileSync(p, "utf8"));

describe("prompt pack files", () => {
  const fromDisk = loadPromptPackFromDir();

  it("pack.json is up to date with the plain files (run `pnpm --filter @akatsuki/prompts build`)", () => {
    expect(PROMPT_PACK).toEqual(fromDisk);
  });

  it("covers exactly the contract features with versions from manifest.json", () => {
    const manifest = json(join(PACK_ROOT, "manifest.json"));
    expect(Object.keys(fromDisk.features).sort()).toEqual([...FEATURE_IDS].sort());
    for (const id of FEATURE_IDS) expect(promptVersion(id)).toBe(manifest.features[id].version);
  });

  it("matches the §12.1 tiers and modes", () => {
    const tiers = Object.fromEntries(FEATURE_IDS.map((id) => [id, getFeature(id).meta.tier]));
    expect(tiers).toEqual({
      summarize: "default",
      instant_reply: "default",
      compose: "default",
      rewrite: "default",
      extract_actions: "default",
      triage: "fast",
      nl_search: "fast",
      voice_profile: "default",
    });
    expect(getFeature("compose").meta.mode).toBe("text");
    expect(getFeature("rewrite").meta.mode).toBe("text");
    expect(getFeature("summarize").meta.stream).toBe(true);
    for (const id of FEATURE_IDS) expect(getFeature(id).meta).not.toHaveProperty("temperature");
  });

  it("every system prompt ends with the shared untrusted-content rules", () => {
    const untrusted = readFileSync(join(PACK_ROOT, "shared/untrusted.md"), "utf8").trimEnd();
    for (const id of FEATURE_IDS) expect(getFeature(id).system.endsWith(`${untrusted}\n`)).toBe(true);
  });

  it("email-derived variables are wrapped in tags", () => {
    for (const id of FEATURE_IDS) {
      const user = getFeature(id).user;
      for (const [v, tag] of [
        ["transcript", "email_transcript"],
        ["items", "triage_items"],
        ["samples", "writing_samples"],
        ["draft", "draft"],
        ["text", "text"],
        ["instruction", "instruction"],
        ["query", "search_request"],
      ] as const) {
        if (!getFeature(id).variables.includes(v)) continue;
        expect(user).toMatch(new RegExp(`<${tag}>\\n?\\{\\{${v}\\}\\}\\n?</${tag}>`));
      }
    }
  });

  it("output schemas use the strict subset", () => {
    for (const id of FEATURE_IDS) {
      const schema = getFeature(id).outputSchema;
      if (getFeature(id).meta.mode === "object") expect(checkOutputSchema(schema)).toEqual([]);
      else expect(schema).toBeNull();
    }
  });

  describe("loader checks (same as crates/ai PromptPack::load)", () => {
    const copy = () => {
      const dir = mkdtempSync(join(tmpdir(), "akatsuki-prompts-"));
      cpSync(join(PACK_ROOT, "manifest.json"), join(dir, "manifest.json"));
      cpSync(join(PACK_ROOT, "shared"), join(dir, "shared"), { recursive: true });
      cpSync(join(PACK_ROOT, "features"), join(dir, "features"), { recursive: true });
      return dir;
    };
    const edit = (dir: string, rel: string, f: (t: string) => string) =>
      writeFileSync(join(dir, rel), f(readFileSync(join(dir, rel), "utf8")));

    it("normalises CRLF files to LF", () => {
      const dir = copy();
      for (const rel of ["shared/untrusted.md", "features/summarize/system.md", "features/summarize/user.md"])
        edit(dir, rel, (t) => t.replaceAll("\n", "\r\n"));
      expect(loadPromptPackFromDir(dir)).toEqual(fromDisk);
      rmSync(dir, { recursive: true });
    });

    it.each([
      [
        /sampling parameters/,
        "features/summarize/meta.json",
        (t: string) => t.replace("{", '{"temperature":0.2,'),
      ],
      [/sampling parameters/, "features/summarize/meta.json", (t: string) => t.replace("{", '{"top_p":0.9,')],
      [/unknown field "seed"/, "features/rewrite/meta.json", (t: string) => t.replace("{", '{"seed":1,')],
      [/unknown field "extra"/, "manifest.json", (t: string) => t.replace("{", '{"extra":true,')],
      [
        /meta.id/,
        "features/rewrite/meta.json",
        (t: string) => t.replace('"id": "rewrite"', '"id": "compose"'),
      ],
      [/placeholders/, "features/rewrite/user.md", (t: string) => t.replace("{{style}}", "style")],
      [/outside a valid placeholder/, "features/rewrite/user.md", (t: string) => `${t}\n{{ text }}`],
      [/must be static/, "features/rewrite/system.md", (t: string) => `${t}\n{{text}}`],
    ] as const)("refuses %s", (message, rel, f) => {
      const dir = copy();
      edit(dir, rel, f);
      expect(() => loadPromptPackFromDir(dir)).toThrow(message);
      rmSync(dir, { recursive: true });
    });
  });

  it("rejects schemas outside the subset", () => {
    expect(checkOutputSchema({ type: "object", properties: { a: { type: "string" } } })).toHaveLength(2);
    expect(checkOutputSchema({ type: "string", format: "date" })[0]).toMatch(/format/);
    expect(checkOutputSchema({ oneOf: [] }).length).toBeGreaterThan(0);
    expect(checkOutputSchema({ type: ["string", "number"] })[0]).toMatch(/union/);
  });
});

describe("eval fixtures", () => {
  const ajv = new Ajv({ strict: true, allowUnionTypes: true });
  const fixtures = loadEvalFixtures();

  it("exist for every feature", () => {
    for (const id of FEATURE_IDS) expect(fixtures.some((f) => f.feature === id)).toBe(true);
  });

  it.each(fixtures.map((f) => [`${f.feature}/${f.name}`, f] as const))(
    "%s vars match input.schema.json",
    (_n, f) => {
      const { $schema: _s, ...schema } = getFeature(f.feature).inputSchema;
      const validate = ajv.compile(schema);
      expect(validate(f.vars), JSON.stringify(validate.errors)).toBe(true);
      expect(f.expect.schemaValid).toBe(true);
    },
  );

  it("contain only synthetic addresses (no real mail)", () => {
    const text = JSON.stringify(fixtures);
    const emails = text.match(/[\w.+-]+@[\w-]+(\.[\w-]+)+/g) ?? [];
    expect(emails.length).toBeGreaterThan(0);
    for (const e of emails) expect(e).toMatch(/@([\w-]+\.)*(example\.(com|org|net)|[\w-]+\.example)$/);
  });

  it("render goldens are up to date (crates/ai compares against them byte for byte)", () => {
    const dir = join(PACK_ROOT, "fixtures/render");
    const onDisk = readdirSync(dir)
      .sort()
      .map((f) => json(join(dir, f)));
    const fresh = renderGoldens(PROMPT_PACK, fixtures).sort((a, b) =>
      `${a.feature}.${a.name}`.localeCompare(`${b.feature}.${b.name}`),
    );
    expect(onDisk).toEqual(fresh);
  });
});

describe("template renderer", () => {
  it("substitutes, escapes angle brackets and renders null as empty", () => {
    expect(renderTemplate("<a>{{x}}</a>|{{y}}|{{x}}", { x: "</a><b>", y: null })).toBe(
      "<a>&lt;/a&gt;&lt;b&gt;</a>||&lt;/a&gt;&lt;b&gt;",
    );
  });

  it("does not re-scan substituted values", () => {
    expect(renderTemplate("{{a}}", { a: "{{b}}" })).toBe("{{b}}");
  });

  it("throws on missing or non-string values and on stray braces", () => {
    expect(() => renderTemplate("{{a}}", {})).toThrow(/missing/);
    expect(() => renderTemplate("{{a}}", { a: 3 as unknown as string })).toThrow(/string/);
    expect(() => templateVariables("{{ a }}")).toThrow(/outside a valid placeholder/);
    expect(() => templateVariables("x }} y")).toThrow();
  });

  const templateCases = json(join(PACK_ROOT, "fixtures/template/cases.json")).cases as {
    template: string;
    vars: Record<string, string | null>;
    expected?: string;
    error?: "stray" | "missing" | "type";
  }[];
  it.each(templateCases.map((c) => [JSON.stringify(c.template), c] as const))(
    "fixtures/template (same cases as crates/ai): %s",
    (_n, c) => {
      if (c.expected !== undefined) {
        expect(renderTemplate(c.template, c.vars)).toBe(c.expected);
        return;
      }
      const message = {
        stray: /outside a valid placeholder/,
        missing: /missing/,
        type: /must be a string or null/,
      }[c.error!];
      expect(() => renderTemplate(c.template, c.vars)).toThrow(message);
    },
  );

  it("lists variables in order of first appearance", () => {
    expect(templateVariables("{{b}} {{a}} {{b}}")).toEqual(["b", "a"]);
  });

  it("escapeValue only touches < and >", () => {
    expect(escapeValue('a & <b> "c"')).toBe('a & &lt;b&gt; "c"');
  });

  it("renderPrompt uses only the feature's variables", () => {
    const p = renderPrompt("rewrite", { text: "Hi <there>", style: "fix", voice: null, extra: "ignored" });
    expect(p.user).toContain("Style: fix");
    expect(p.user).toContain("Hi &lt;there&gt;");
    expect(p.user).not.toContain("ignored");
    expect(p.system).toBe(getFeature("rewrite").system);
  });
});
