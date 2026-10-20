// contacts.search (ARCHITECTURE §4.2): autocomplete from headers.
import { v } from "convex/values";
import { entitledQuery } from "./lib/auth";
import { appError } from "./lib/errors";

export const search = entitledQuery({
  args: { q: v.string(), limit: v.number() },
  returns: v.array(v.object({ email: v.string(), name: v.optional(v.string()) })),
  handler: async (ctx, { q, limit }) => {
    if (limit < 1 || limit > 10) throw appError("invalid_argument", "limit is 1 to 10.");
    const text = q.trim().slice(0, 200);
    const rows =
      text.length === 0
        ? await ctx.db
            .query("contacts")
            .withIndex("by_user_last", (x) => x.eq("userId", ctx.userId))
            .order("desc")
            .take(limit)
        : await ctx.db
            .query("contacts")
            .withSearchIndex("search_contacts", (x) => x.search("searchText", text).eq("userId", ctx.userId))
            .take(limit);
    return rows.map((c) => (c.name ? { email: c.email, name: c.name } : { email: c.email }));
  },
});
