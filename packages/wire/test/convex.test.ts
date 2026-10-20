// `@akatsuki/wire/convex` re-exports the Convex validators, function references and id types (§1.7).
import { describe, expect, it } from "vitest";
import { api, type Id, type LabelView, type ThreadRow, vThreadAction } from "../src/convex.ts";
import type { Id as WireId } from "../src/ids.ts";

describe("@akatsuki/wire/convex", () => {
  it("re-exports validators and function references", () => {
    expect(vThreadAction.kind).toBe("union");
    expect(api.mail.listThreads).toBeDefined();
  });

  it("uses the same Id type as the main entry", () => {
    const a: Id<"threads"> = "abc" as WireId<"threads">;
    const b: WireId<"threads"> = a;
    const row: Pick<ThreadRow, "_id"> = { _id: b };
    const label: Pick<LabelView, "name"> = { name: "Work" };
    expect([row._id, label.name]).toEqual(["abc", "Work"]);
  });
});
