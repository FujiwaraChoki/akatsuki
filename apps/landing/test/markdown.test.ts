import { describe, expect, it } from "vitest";
import { period, readContent, readMinutes, renderDoc, rewriteHref, slugify } from "@/lib/markdown";

describe("renderDoc", () => {
  it("lifts the first h1 into the title and numbers the h2 sections", () => {
    const doc = renderDoc(
      "# Privacy at Akatsuki\n\nIntro.\n\n## The short version\n\nText.\n\n### Convex\n\n## Who is responsible\n",
    );
    expect(doc.title).toBe("Privacy at Akatsuki");
    expect(doc.intro).toContain("Intro.");
    expect(doc.sections.map((s) => [s.num, s.id, s.title])).toEqual([
      ["01", "the-short-version", "The short version"],
      ["02", "who-is-responsible", "Who is responsible"],
    ]);
    expect(doc.sections[0]?.html).toContain('<h3 id="convex">Convex</h3>');
    expect(doc.sections[0]?.html).not.toContain("<h1");
  });

  it("de-duplicates heading ids", () => {
    const doc = renderDoc("## Notes\n\n## Notes\n");
    expect(doc.sections.map((s) => s.id)).toEqual(["notes", "notes-1"]);
  });

  it("drops trailing periods from serif headings (only the home hero and finale keep one)", () => {
    const doc = renderDoc("## In brief.\n");
    expect(doc.sections[0]?.html).toMatch(/In brief<\/h2>/);
    expect(period("Changelog.")).toBe("Changelog");
  });

  it("maps repository links to site pages and marks external links", () => {
    const doc = renderDoc(
      "## Links\n\nSee [policy](../SECURITY.md#report) and [example](https://example.com/x).",
    );
    expect(doc.sections[0]?.html).toContain('<a href="/security#report">policy</a>');
    expect(doc.sections[0]?.html).toContain(
      '<a href="https://example.com/x" rel="noopener noreferrer">example</a>',
    );
  });

  it("wraps tables for horizontal scrolling", () => {
    const doc = renderDoc("## T\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
    expect(doc.sections[0]?.html).toMatch(/<div class="table-wrap"><table>[\s\S]*<\/table><\/div>/);
  });
});

describe("helpers", () => {
  it("slugify keeps letters and digits", () => {
    expect(slugify("Who we share data with (sub-processors)")).toBe("who-we-share-data-with-sub-processors");
  });
  it("rewriteHref keeps anchors and mailto", () => {
    expect(rewriteHref("#self-hosted-copies")).toBe("#self-hosted-copies");
    expect(rewriteHref("mailto:a@b.c")).toBe("mailto:a@b.c");
    expect(rewriteHref("LICENSE")).toBe("/license");
  });
  it("estimates reading time", () => {
    expect(readMinutes(10)).toBe(1);
    expect(readMinutes(2300)).toBe(10);
  });
});

describe("repository documents", () => {
  it("reads privacy, security and the licence from the repository at build time", () => {
    expect(renderDoc(readContent("privacy")).sections.length).toBeGreaterThan(5);
    expect(readContent("security")).toMatch(/security/i);
    expect(readContent("licence")).toMatch(/ASAL/);
  });
});
