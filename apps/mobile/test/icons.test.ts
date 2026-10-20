// The generated icon data matches the shared SVG set (design spec §2.11).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { generate, OUT, parseSvg } from "../scripts/icons.mjs";

describe("icons", () => {
  it("are generated from the SVG files", () => {
    expect(readFileSync(OUT, "utf8")).toBe(generate());
  });
  it("parse stroke geometry", () => {
    expect(
      parseSvg(
        '<svg viewBox="0 0 24 24" stroke="black"><path d="M3 6h18"/><circle cx="12" cy="12" r="8.5"/></svg>',
      ),
    ).toEqual([
      ["path", { d: "M3 6h18" }],
      ["circle", { cx: "12", cy: "12", r: "8.5" }],
    ]);
  });
});
