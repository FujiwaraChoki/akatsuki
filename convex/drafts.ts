// drafts.* (ARCHITECTURE §4.2, §10.1). Metadata only: draft text lives at the provider and on devices.
import { paginationOptsValidator, paginationResultValidator } from "convex/server";
import { v } from "convex/values";
import { entitledMutation, entitledQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { checkClientOpId, insertOp, opsForClientOp } from "./lib/ops";
import { project, vDraftMeta } from "./lib/rows";

export const list = entitledQuery({
  args: { paginationOpts: paginationOptsValidator },
  returns: paginationResultValidator(vDraftMeta),
  handler: async (ctx, { paginationOpts }) => {
    const result = await ctx.db
      .query("drafts")
      .withIndex("by_user_updated", (q) => q.eq("userId", ctx.userId))
      .order("desc")
      .filter((q) => q.or(q.eq(q.field("state"), "editing"), q.eq(q.field("state"), "scheduled")))
      .paginate(paginationOpts);
    return { ...result, page: result.page.map((d) => project(vDraftMeta, d)) };
  },
});

export const get = entitledQuery({
  args: { clientDraftId: v.string() },
  returns: v.union(vDraftMeta, v.null()),
  handler: async (ctx, { clientDraftId }) => {
    const d = await ctx.db
      .query("drafts")
      .withIndex("by_user_client", (q) => q.eq("userId", ctx.userId).eq("clientDraftId", clientDraftId))
      .unique();
    return d ? project(vDraftMeta, d) : null;
  },
});

export const discard = entitledMutation({
  args: { clientOpId: v.string(), clientDraftId: v.string() },
  returns: v.object({ opId: v.id("ops") }),
  handler: async (ctx, { clientOpId, clientDraftId }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (replay[0]) return { opId: replay[0]._id };
    const draft = await ctx.db
      .query("drafts")
      .withIndex("by_user_client", (q) => q.eq("userId", ctx.userId).eq("clientDraftId", clientDraftId))
      .unique();
    if (!draft) throw appError("not_found");
    if (draft.state === "sending" || draft.state === "sent") {
      throw appError("invalid_argument", "This draft is already being sent.");
    }
    const account = await ctx.db.get(draft.accountId);
    if (!account) throw appError("not_found");
    await ctx.db.patch(draft._id, { state: "discarded", updatedAt: Date.now() });
    const opId = await insertOp(ctx, {
      userId: ctx.userId,
      account,
      clientOpId,
      // No threadId: only `thread` ops queue per thread (§3.2, §5.4).
      payload: { kind: "delete_draft", draftId: draft._id },
    });
    return { opId };
  },
});
