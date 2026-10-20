// Auth wrappers (ARCHITECTURE §4.1), built with convex-helpers customQuery/customMutation.
import { customCtx, customMutation, customQuery } from "convex-helpers/server/customFunctions";
import type { Doc, Id, TableNames } from "../_generated/dataModel";
import { mutation, type QueryCtx, query } from "../_generated/server";
import { billingDisabled, NO_ENTITLEMENT, selfhostEntitlement } from "../ee/entitlement";
import { appError } from "./errors";
import type { Entitlement } from "./validators";

type AuthCtx = Pick<QueryCtx, "auth">;

async function requireIssuer(ctx: AuthCtx, issuer: string | undefined) {
  const identity = await ctx.auth.getUserIdentity();
  if (!identity || !issuer || identity.issuer !== issuer || !identity.subject)
    throw appError("unauthenticated");
  return identity;
}

/** Better Auth user: `iss === CONVEX_SITE_URL`. Returns the Better Auth user id (JWT sub). */
export async function requireUserId(ctx: AuthCtx): Promise<string> {
  return (await requireIssuer(ctx, process.env.CONVEX_SITE_URL)).subject;
}

export async function loadEntitlement(ctx: Pick<QueryCtx, "db">, userId: string): Promise<Entitlement> {
  if (billingDisabled()) return selfhostEntitlement();
  const billing = await ctx.db
    .query("billing")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
  return billing?.entitlement ?? NO_ENTITLEMENT;
}

async function requireEntitled(ctx: Pick<QueryCtx, "auth" | "db">) {
  const userId = await requireUserId(ctx);
  const entitlement = await loadEntitlement(ctx, userId);
  if (!entitlement.hostedSync) throw appError("not_entitled");
  return { userId, entitlement };
}

export const userQuery = customQuery(
  query,
  customCtx(async (ctx) => ({ userId: await requireUserId(ctx) })),
);
export const userMutation = customMutation(
  mutation,
  customCtx(async (ctx) => ({ userId: await requireUserId(ctx) })),
);
export const entitledQuery = customQuery(query, customCtx(requireEntitled));
export const entitledMutation = customMutation(mutation, customCtx(requireEntitled));

export const workerQuery = customQuery(
  query,
  customCtx(async (ctx) => ({ workerSub: (await requireIssuer(ctx, process.env.WORKER_ISSUER)).subject })),
);
export const workerMutation = customMutation(
  mutation,
  customCtx(async (ctx) => ({ workerSub: (await requireIssuer(ctx, process.env.WORKER_ISSUER)).subject })),
);
export const apiQuery = customQuery(
  query,
  customCtx(async (ctx) => {
    await requireIssuer(ctx, process.env.API_ISSUER);
    return {};
  }),
);
export const apiMutation = customMutation(
  mutation,
  customCtx(async (ctx) => {
    await requireIssuer(ctx, process.env.API_ISSUER);
    return {};
  }),
);

type Owned = { userId: string };

/**
 * Loads a document and checks it belongs to `userId`. Missing and foreign ids both throw
 * `not_found`, so ids cannot be probed (§3.3).
 */
export async function getOwned<T extends TableNames>(
  ctx: Pick<QueryCtx, "db">,
  id: Id<T>,
  userId: string,
): Promise<Doc<T> & Owned> {
  const doc = (await ctx.db.get(id)) as (Doc<T> & Partial<Owned>) | null;
  if (!doc || doc.userId !== userId) throw appError("not_found");
  return doc as Doc<T> & Owned;
}
