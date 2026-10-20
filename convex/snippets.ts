// snippets.* (ARCHITECTURE §4.2): user-authored templates, sanitized on write.
import { v } from "convex/values";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { project, vSnippetView } from "./lib/rows";
import { htmlToText, sanitizeUserHtml } from "./lib/sanitizeUserHtml";

const MAX_BODY = 20 * 1024;
const SHORTCUT_RE = /^[a-z0-9][a-z0-9_-]{0,31}$/;
const VAR_RE = /^[a-z][a-z0-9_]{0,31}$/;

export const list = entitledQuery({
  args: {},
  returns: v.array(vSnippetView),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("snippets")
      .withIndex("by_user_name", (q) => q.eq("userId", ctx.userId))
      .take(500);
    return rows.map((s) => project(vSnippetView, s));
  },
});

export const search = entitledQuery({
  args: { q: v.string() },
  returns: v.array(vSnippetView),
  handler: async (ctx, { q }) => {
    const text = q.trim().slice(0, 200);
    if (text.length === 0) return [];
    // An exact shortcut match comes first (typing ";thx" in the composer).
    const byShortcut = await ctx.db
      .query("snippets")
      .withIndex("by_user_shortcut", (x) => x.eq("userId", ctx.userId).eq("shortcut", text.toLowerCase()))
      .first();
    const hits = await ctx.db
      .query("snippets")
      .withSearchIndex("search_snippets", (x) => x.search("searchText", text).eq("userId", ctx.userId))
      .take(20);
    const out = byShortcut ? [byShortcut, ...hits.filter((h) => h._id !== byShortcut._id)] : hits;
    return out.slice(0, 20).map((s) => project(vSnippetView, s));
  },
});

export const save = entitledMutation({
  args: {
    snippetId: v.optional(v.id("snippets")),
    name: v.string(),
    shortcut: v.optional(v.string()),
    subject: v.optional(v.string()),
    bodyHtml: v.string(),
    variables: v.array(v.string()),
  },
  returns: v.object({ snippetId: v.id("snippets") }),
  handler: async (ctx, { snippetId, name, shortcut, subject, bodyHtml, variables }) => {
    const n = name.trim();
    if (n.length === 0 || n.length > 100)
      throw appError("invalid_argument", "Snippet names are 1 to 100 characters.");
    const sc = shortcut?.trim().toLowerCase() || undefined;
    if (sc !== undefined && !SHORTCUT_RE.test(sc)) throw appError("invalid_argument", "Bad shortcut.");
    if (subject !== undefined && subject.length > 998)
      throw appError("invalid_argument", "Subject is too long.");
    if (variables.length > 20 || !variables.every((x) => VAR_RE.test(x))) {
      throw appError("invalid_argument", "Variables are up to 20 lowercase names.");
    }
    const clean = sanitizeUserHtml(bodyHtml);
    if (clean.length > MAX_BODY) throw appError("payload_too_large", "Snippets are limited to 20 KB.");
    if (sc !== undefined) {
      const clash = await ctx.db
        .query("snippets")
        .withIndex("by_user_shortcut", (q) => q.eq("userId", ctx.userId).eq("shortcut", sc))
        .first();
      if (clash && clash._id !== snippetId)
        throw appError("invalid_argument", "That shortcut is already used.");
    }
    const searchText = [n, sc ?? "", subject ?? "", htmlToText(clean)].join("\n").slice(0, 4000);
    const now = Date.now();
    if (snippetId) {
      await getOwned(ctx, snippetId, ctx.userId);
      await ctx.db.patch(snippetId, {
        name: n,
        shortcut: sc,
        subject,
        bodyHtml: clean,
        variables,
        searchText,
        updatedAt: now,
      });
      return { snippetId };
    }
    const id = await ctx.db.insert("snippets", {
      userId: ctx.userId,
      name: n,
      shortcut: sc,
      subject,
      bodyHtml: clean,
      variables,
      searchText,
      useCount: 0,
      updatedAt: now,
    });
    return { snippetId: id };
  },
});

export const remove = entitledMutation({
  args: { snippetId: v.id("snippets") },
  returns: v.null(),
  handler: async (ctx, { snippetId }) => {
    await getOwned(ctx, snippetId, ctx.userId);
    await ctx.db.delete(snippetId);
    return null;
  },
});

export const markUsed = entitledMutation({
  args: { snippetId: v.id("snippets") },
  returns: v.null(),
  handler: async (ctx, { snippetId }) => {
    const s = await getOwned(ctx, snippetId, ctx.userId);
    await ctx.db.patch(snippetId, { useCount: s.useCount + 1, lastUsedAt: Date.now() });
    return null;
  },
});
