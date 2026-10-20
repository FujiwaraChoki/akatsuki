// search.threads (ARCHITECTURE §11.2 backend 1): headers and snippets of everything synced.
import { paginationOptsValidator } from "convex/server";
import { v } from "convex/values";
import type { Doc, Id } from "./_generated/dataModel";
import type { QueryCtx } from "./_generated/server";
import { entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { SYS } from "./lib/model";
import { parseQuery, type QueryAst } from "./lib/query";
import { toThreadRow, vThreadRow } from "./lib/rows";
import { labelKeyOf } from "./lib/threads";

type Leaf = Exclude<QueryAst, { and: QueryAst[] } | { or: QueryAst[] } | { not: QueryAst }>;
type Clause = { neg: boolean; leaf: Leaf };

const DAY = 86_400_000;
const MAX_TERMS = 16;
const MAX_SCAN = 2000;

const IN_KEYS: Record<string, string> = {
  inbox: SYS.inbox,
  sent: SYS.sent,
  drafts: SYS.drafts,
  trash: SYS.trash,
  spam: SYS.spam,
  snoozed: SYS.snoozed,
  starred: SYS.starred,
  important: SYS.important,
  unread: SYS.unread,
};

/** Flattens a top-level AND into clauses; OR nodes are reported as unsupported in Convex search. */
function clausesOf(ast: QueryAst, unsupported: Set<string>): Clause[] {
  const items = "and" in ast ? ast.and : [ast];
  const out: Clause[] = [];
  for (const item of items) {
    if ("or" in item) {
      unsupported.add("OR");
      continue;
    }
    if ("and" in item) {
      out.push(...clausesOf(item, unsupported));
      continue;
    }
    if ("not" in item) {
      const inner = item.not;
      if ("and" in inner || "or" in inner || "not" in inner) {
        unsupported.add("-( )");
        continue;
      }
      out.push({ neg: true, leaf: inner });
      continue;
    }
    out.push({ neg: false, leaf: item });
  }
  return out;
}

interface Resolved {
  labelKeyByName: Map<string, string>;
  splitKeyByName: Map<string, string>;
  accountIdByEmail: Map<string, Id<"mailAccounts">>;
}

async function resolveNames(ctx: Pick<QueryCtx, "db">, userId: string): Promise<Resolved> {
  const accounts = await ctx.db
    .query("mailAccounts")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .take(100);
  const labelKeyByName = new Map<string, string>();
  for (const a of accounts) {
    const labels = await ctx.db
      .query("labels")
      .withIndex("by_account", (q) => q.eq("accountId", a._id))
      .take(2000);
    for (const l of labels) labelKeyByName.set(l.name.toLowerCase(), labelKeyOf(l));
  }
  const splits = await ctx.db
    .query("splits")
    .withIndex("by_user_order", (q) => q.eq("userId", userId))
    .take(50);
  return {
    labelKeyByName,
    splitKeyByName: new Map(splits.map((s) => [s.name.toLowerCase(), `split:${s._id}`])),
    accountIdByEmail: new Map(accounts.map((a) => [a.email.toLowerCase(), a._id])),
  };
}

const includesCI = (hay: string | undefined, needle: string): boolean =>
  hay?.toLowerCase().includes(needle.toLowerCase()) ?? false;

function matchesClause(
  c: Clause,
  t: Doc<"threads">,
  text: string | undefined,
  r: Resolved,
  nowAt: number,
  unsupported: Set<string>,
): boolean {
  const leaf = c.leaf;
  let hit: boolean;
  if ("text" in leaf) {
    // Search-index hits already match words; phrases and negations need the text itself.
    hit =
      text === undefined ? includesCI(`${t.subject}\n${t.snippet}`, leaf.text) : includesCI(text, leaf.text);
  } else if ("at" in leaf) {
    hit = leaf.field === "before" ? t.lastMessageAt < leaf.at : t.lastMessageAt >= leaf.at;
  } else if ("days" in leaf) {
    const edge = nowAt - leaf.days * DAY;
    hit = leaf.field === "older_than" ? t.lastMessageAt < edge : t.lastMessageAt >= edge;
  } else {
    const value = leaf.value.toLowerCase();
    switch (leaf.field) {
      case "from":
        hit = includesCI(t.lastFrom?.email, value) || includesCI(t.lastFrom?.name, value);
        break;
      case "to":
      case "cc":
        hit = t.participants.some((p) => includesCI(p.email, value) || includesCI(p.name, value));
        break;
      case "subject":
        hit = includesCI(t.subject, value);
        break;
      case "label": {
        const key = r.labelKeyByName.get(value);
        hit = key !== undefined && t.labelKeys.includes(key);
        break;
      }
      case "in": {
        if (value === "anywhere") hit = true;
        else if (value === "archive") {
          hit = ![SYS.inbox, SYS.trash, SYS.spam].some((k) => t.labelKeys.includes(k));
        } else {
          const key = IN_KEYS[value] ?? r.splitKeyByName.get(value);
          hit = key !== undefined && (t.labelKeys.includes(key) || t.split === key);
        }
        break;
      }
      case "is":
        if (value === "read") hit = !t.labelKeys.includes(SYS.unread);
        else if (value === "needs_reply") hit = t.triage?.needsReply === true;
        else hit = IN_KEYS[value] !== undefined && t.labelKeys.includes(IN_KEYS[value]!);
        break;
      case "has":
        hit = value === "attachment" ? t.hasAttachments : false;
        break;
      case "list":
        hit = includesCI(t.listId, value);
        break;
      case "category":
        hit = t.triage?.category === value;
        break;
      case "account":
        hit = r.accountIdByEmail.get(value) === t.accountId || t.accountId === leaf.value;
        break;
      case "filename":
        unsupported.add("filename:");
        hit = true;
        break;
    }
  }
  return c.neg ? !hit : hit;
}

export const threads = entitledQuery({
  args: {
    q: v.string(),
    nowAt: v.number(),
    accountId: v.optional(v.id("mailAccounts")),
    paginationOpts: paginationOptsValidator,
  },
  returns: v.object({
    page: v.array(vThreadRow),
    isDone: v.boolean(),
    continueCursor: v.string(),
    unsupported: v.array(v.string()),
    ast: v.any(), // QueryAst (§11.1); recursive, so not expressible as a Convex validator
  }),
  handler: async (ctx, { q, nowAt, accountId, paginationOpts }) => {
    if (q.length > 500) throw appError("invalid_argument", "Queries are limited to 500 characters.");
    if (accountId) await getOwned(ctx, accountId, ctx.userId);
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (x) => x.eq("userId", ctx.userId))
      .unique();
    const { ast } = parseQuery(q, { nowAt, timezone: profile?.timezone ?? "UTC" });
    const unsupported = new Set<string>();
    const clauses = clausesOf(ast, unsupported);
    const resolved = await resolveNames(ctx, ctx.userId);

    // Positive free text plus from/to/subject values feed the search index.
    const terms: string[] = [];
    for (const c of clauses) {
      if (c.neg) continue;
      if ("text" in c.leaf) terms.push(c.leaf.text);
      else if ("value" in c.leaf && ["from", "to", "subject"].includes(c.leaf.field))
        terms.push(c.leaf.value);
    }
    const eqFilters = {
      inInbox: undefined as boolean | undefined,
      isUnread: undefined as boolean | undefined,
      isStarred: undefined as boolean | undefined,
      hasAttachments: undefined as boolean | undefined,
      isOutbound: undefined as boolean | undefined,
    };
    let scopedAccount = accountId;
    for (const c of clauses) {
      if (c.neg || !("value" in c.leaf)) continue;
      const value = c.leaf.value.toLowerCase();
      if (c.leaf.field === "in" && value === "inbox") eqFilters.inInbox = true;
      if (c.leaf.field === "in" && value === "sent") eqFilters.isOutbound = true;
      if (c.leaf.field === "is" && value === "unread") eqFilters.isUnread = true;
      if (c.leaf.field === "is" && value === "starred") eqFilters.isStarred = true;
      if (c.leaf.field === "has" && value === "attachment") eqFilters.hasAttachments = true;
      if (c.leaf.field === "account") scopedAccount = resolved.accountIdByEmail.get(value) ?? scopedAccount;
    }

    const keep = (t: Doc<"threads"> | null, text?: string): t is Doc<"threads"> =>
      t !== null &&
      !t.deletedPending &&
      (scopedAccount === undefined || t.accountId === scopedAccount) &&
      clauses.every((c) => matchesClause(c, t, text, resolved, nowAt, unsupported));

    if (terms.length > 0) {
      const result = await ctx.db
        .query("threadSearch")
        .withSearchIndex("search_text", (s) => {
          let b = s.search("text", terms.slice(0, MAX_TERMS).join(" ")).eq("userId", ctx.userId);
          if (scopedAccount) b = b.eq("accountId", scopedAccount);
          for (const [k, val] of Object.entries(eqFilters)) {
            if (val !== undefined) b = b.eq(k as "inInbox", val);
          }
          return b;
        })
        .paginate(paginationOpts);
      const page: Doc<"threads">[] = [];
      for (const hit of result.page) {
        const t = await ctx.db.get(hit.threadId);
        if (keep(t, hit.text)) page.push(t);
      }
      return {
        page: page.map(toThreadRow),
        isDone: result.isDone,
        continueCursor: result.continueCursor,
        unsupported: [...unsupported],
        ast,
      };
    }

    // No free text: serve from a list index with post-filters (≤ 2,000 docs scanned per page).
    const baseKey = (() => {
      for (const c of clauses) {
        if (c.neg || !("value" in c.leaf)) continue;
        const value = c.leaf.value.toLowerCase();
        if (c.leaf.field === "in" && (IN_KEYS[value] || resolved.splitKeyByName.has(value))) {
          return IN_KEYS[value] ?? resolved.splitKeyByName.get(value)!;
        }
        if (c.leaf.field === "label" && resolved.labelKeyByName.has(value))
          return resolved.labelKeyByName.get(value)!;
      }
      return null;
    })();
    const numItems = Math.min(paginationOpts.numItems, 100);
    const scan = { ...paginationOpts, numItems: Math.min(MAX_SCAN, numItems * 20) };
    let rows: Doc<"threads">[];
    let isDone: boolean;
    let continueCursor: string;
    if (baseKey) {
      const unreadOnly = eqFilters.isUnread === true;
      const r = await (scopedAccount
        ? unreadOnly
          ? ctx.db
              .query("threadLabels")
              .withIndex("by_account_label_unread_sort", (x) =>
                x.eq("accountId", scopedAccount).eq("labelKey", baseKey).eq("isUnread", true),
              )
          : ctx.db
              .query("threadLabels")
              .withIndex("by_account_label_sort", (x) =>
                x.eq("accountId", scopedAccount).eq("labelKey", baseKey),
              )
        : unreadOnly
          ? ctx.db
              .query("threadLabels")
              .withIndex("by_user_label_unread_sort", (x) =>
                x.eq("userId", ctx.userId).eq("labelKey", baseKey).eq("isUnread", true),
              )
          : ctx.db
              .query("threadLabels")
              .withIndex("by_user_label_sort", (x) => x.eq("userId", ctx.userId).eq("labelKey", baseKey))
      )
        .order("desc")
        .paginate(scan);
      rows = (await Promise.all(r.page.map((row) => ctx.db.get(row.threadId)))).filter(
        (t): t is Doc<"threads"> => t !== null,
      );
      isDone = r.isDone;
      continueCursor = r.continueCursor;
    } else {
      const r = await (scopedAccount
        ? ctx.db.query("threads").withIndex("by_account_time", (x) => x.eq("accountId", scopedAccount))
        : ctx.db.query("threads").withIndex("by_user_time", (x) => x.eq("userId", ctx.userId))
      )
        .order("desc")
        .paginate(scan);
      rows = r.page;
      isDone = r.isDone;
      continueCursor = r.continueCursor;
    }
    return {
      page: rows.filter((t) => keep(t)).map(toThreadRow),
      isDone,
      continueCursor,
      unsupported: [...unsupported],
      ast,
    };
  },
});
