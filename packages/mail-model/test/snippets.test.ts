// Snippet helpers (D-295): variables, the `;shortcut` trigger, ranking, and the text ↔ HTML round trip.
import { describe, expect, it } from "vitest";
import {
  applySnippet,
  builtinVariables,
  expandVariables,
  firstName,
  rankSnippets,
  snippetForTrigger,
  snippetHtmlToText,
  snippetProblems,
  snippetSaveArgs,
  snippetTextToHtml,
  snippetTrigger,
  spliceText,
  variablesIn,
} from "../src/snippets.ts";

const NOW = Date.UTC(2026, 9, 20, 12); // Tuesday 20 October 2026

describe("variables", () => {
  it("lists variables in order of first use, lowercased", () => {
    expect(variablesIn("Hi {{First_Name}}, see you {{ day }}. {{first_name}}")).toEqual([
      "first_name",
      "day",
    ]);
    expect(variablesIn("no {{1bad}} {{}} vars")).toEqual([]);
  });
  it("fills known variables and reports the rest", () => {
    const vars = builtinVariables([{ name: "Mara Okafor", email: "mara@example.com" }], NOW, "UTC");
    expect(vars).toEqual({
      first_name: "Mara",
      name: "Mara Okafor",
      email: "mara@example.com",
      day: "Tuesday",
      date: "October 20",
    });
    expect(expandVariables("Hi {{first_name}}, about {{project}} {{Project}}", vars)).toEqual({
      text: "Hi Mara, about {{project}} {{Project}}",
      missing: ["project"],
    });
  });
  it("reads the day in the user's zone", () => {
    expect(builtinVariables([], Date.UTC(2026, 9, 20, 23, 30), "Asia/Tokyo").day).toBe("Wednesday");
    expect(builtinVariables([], NOW, "UTC").first_name).toBe("");
  });
  it("finds a first name", () => {
    expect(firstName({ name: "Okafor, Mara", email: "x@y.z" })).toBe("Mara");
    expect(firstName({ email: "jun.park@example.com" })).toBe("Jun");
    expect(firstName(undefined)).toBe("");
  });
});

describe("the ;shortcut trigger", () => {
  const snippets = [
    { _id: "a", name: "Thanks", shortcut: "thx", bodyHtml: "<p>Thanks, {{first_name}}!</p>", useCount: 2 },
    { _id: "b", name: "Intro call", shortcut: "intro", bodyHtml: "<p>Let us talk.</p>", useCount: 5 },
    { _id: "c", name: "Address", bodyHtml: "<p>1 Main St</p>" },
  ];
  it("matches a shortcut at a word start before the cursor", () => {
    expect(snippetTrigger("Hello ;THX")).toEqual({ start: 6, end: 10, shortcut: "thx" });
    expect(snippetTrigger(";intro")).toEqual({ start: 0, end: 6, shortcut: "intro" });
    expect(snippetTrigger("a;thx")).toBeNull();
    expect(snippetTrigger("see https://x.y/;thx")).toBeNull();
    expect(snippetTrigger("Hello ;thx and more", 10)).toEqual({ start: 6, end: 10, shortcut: "thx" });
    expect(snippetForTrigger(snippets, snippetTrigger("ok ;thx"))?._id).toBe("a");
    expect(snippetForTrigger(snippets, snippetTrigger("ok ;nope"))).toBeUndefined();
  });
  it("ranks exact shortcuts, then use, then name", () => {
    expect(rankSnippets(snippets).map((s) => s._id)).toEqual(["b", "a", "c"]);
    expect(rankSnippets(snippets, ";thx").map((s) => s._id)).toEqual(["a"]);
    expect(rankSnippets(snippets, "main").map((s) => s._id)).toEqual(["c"]);
  });
  it("applies a snippet over the trigger and fills the subject once", () => {
    const draft = { text: "Hi ;thx", subject: "", to: [{ email: "mara@example.com" }] };
    const r = applySnippet(
      { bodyHtml: "<p>Thanks, {{first_name}}!<br>See {{when}}</p>", subject: "Thank you" },
      draft,
      { start: 3, end: 7 },
      { now: NOW, timeZone: "UTC" },
    );
    expect(r).toEqual({
      text: "Hi Thanks, Mara!\nSee {{when}}",
      cursor: 29,
      subject: "Thank you",
      missing: ["when"],
    });
    expect(
      applySnippet(
        { bodyHtml: "x", subject: "S" },
        { ...draft, subject: "Kept" },
        { start: 0, end: 0 },
        {
          now: NOW,
          timeZone: "UTC",
        },
      ).subject,
    ).toBeNull();
    expect(spliceText("abc", 5, 1, "Z")).toEqual({ text: "abcZ", cursor: 4 });
  });
});

describe("editing snippets", () => {
  it("round-trips plain text through simple HTML", () => {
    const text = "Hi <team> & co,\nline two\n\nSecond paragraph";
    const html = snippetTextToHtml(text);
    expect(html).toBe("<p>Hi &lt;team&gt; &amp; co,<br>line two</p>\n<p>Second paragraph</p>");
    expect(snippetHtmlToText(html)).toBe(text);
    expect(snippetHtmlToText("<div>A</div><ul><li>one</li><li>two</li></ul><script>x</script>")).toBe(
      "A\n\n- one\n- two",
    );
    expect(snippetHtmlToText("&amp;lt;")).toBe("&lt;");
  });
  it("checks what snippets.save checks and builds its arguments", () => {
    expect(snippetProblems({ name: "", shortcut: "bad shortcut", text: " " })).toEqual([
      "Give it a name.",
      "Write the snippet text.",
      "Shortcuts use letters, digits, - and _ only (up to 32).",
    ]);
    expect(snippetProblems({ name: "Thanks", shortcut: ";THX", text: "Thanks {{first_name}}" })).toEqual([]);
    expect(
      snippetSaveArgs({ name: " Thanks ", shortcut: ";THX", subject: " ", text: "Thanks {{first_name}}" }),
    ).toEqual({
      name: "Thanks",
      shortcut: "thx",
      bodyHtml: "<p>Thanks {{first_name}}</p>",
      variables: ["first_name"],
    });
  });
});
