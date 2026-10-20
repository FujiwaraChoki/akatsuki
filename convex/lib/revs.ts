// Per-account change counter for the desktop mirror feed (ARCHITECTURE §3.3, §16.3).
import type { Id } from "../_generated/dataModel";
import type { MutationCtx, QueryCtx } from "../_generated/server";

/**
 * Increments the account's change counter and returns the new value. Calling it several times in
 * one mutation is safe: writes to the counter collapse into one document write at commit, and the
 * desktop feed only relies on revs being strictly increasing per account (§16.3).
 */
export async function bumpRev(ctx: MutationCtx, accountId: Id<"mailAccounts">): Promise<number> {
  const row = await ctx.db
    .query("accountRevs")
    .withIndex("by_account", (q) => q.eq("accountId", accountId))
    .unique();
  if (row) {
    await ctx.db.patch(row._id, { rev: row.rev + 1 });
    return row.rev + 1;
  }
  await ctx.db.insert("accountRevs", { accountId, rev: 1 });
  return 1;
}

export async function currentRev(ctx: Pick<QueryCtx, "db">, accountId: Id<"mailAccounts">): Promise<number> {
  return (await revState(ctx, accountId)).head;
}

/**
 * The account's head rev and its GC watermark: the highest rev of a tombstone that `tombstones.gc`
 * deleted. A feed position below the watermark may have missed deletions (§16.3, D-391).
 */
export async function revState(
  ctx: Pick<QueryCtx, "db">,
  accountId: Id<"mailAccounts">,
): Promise<{ head: number; gcRev: number }> {
  const row = await ctx.db
    .query("accountRevs")
    .withIndex("by_account", (q) => q.eq("accountId", accountId))
    .unique();
  return { head: row?.rev ?? 0, gcRev: row?.gcRev ?? 0 };
}

/** Records a deletion of a mirrored document (threads, messages, labels). */
export async function tombstone(
  ctx: MutationCtx,
  accountId: Id<"mailAccounts">,
  table: "threads" | "messages" | "labels",
  docId: string,
): Promise<void> {
  const rev = await bumpRev(ctx, accountId);
  await ctx.db.insert("tombstones", { accountId, rev, table, docId, at: Date.now() });
}
