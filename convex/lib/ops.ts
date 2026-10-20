// Ops outbox helpers (ARCHITECTURE §5.2–§5.5).
import type { Doc, Id } from "../_generated/dataModel";
import type { MutationCtx, QueryCtx } from "../_generated/server";
import { appError } from "./errors";
import {
  diffKeys,
  isLabelKey,
  isSplitKey,
  parseLabelKey,
  inverseAction as sharedInverseAction,
  type ThreadActionLike,
} from "./model";
import { previewAction, recomputeThread } from "./threads";
import type { OpPayload } from "./validators";

export const MAX_ATTEMPTS = { thread: 10, label: 10, delete_draft: 10, send: 5 } as const;
export const MAX_THREADS_PER_CALL = 500;

export function maxAttemptsFor(payload: OpPayload): number {
  switch (payload.kind) {
    case "thread":
      return MAX_ATTEMPTS.thread;
    case "send":
      return MAX_ATTEMPTS.send;
    case "delete_draft":
      return MAX_ATTEMPTS.delete_draft;
    default:
      return MAX_ATTEMPTS.label;
  }
}

export async function opsForClientOp(ctx: Pick<QueryCtx, "db">, userId: string, clientOpId: string) {
  return await ctx.db
    .query("ops")
    .withIndex("by_user_clientOp", (q) => q.eq("userId", userId).eq("clientOpId", clientOpId))
    .collect();
}

const CLIENT_OP_ID_RE = /^[A-Za-z0-9:_-]{8,128}$/;
export function checkClientOpId(id: string): void {
  if (!CLIENT_OP_ID_RE.test(id)) throw appError("invalid_argument", "Bad clientOpId.");
}

/** Label keys an action touches. */
function actionKeys(a: ThreadActionLike): string[] {
  switch (a.kind) {
    case "modify":
      return [...a.add, ...a.remove];
    case "move":
      return [a.to];
    default:
      return [];
  }
}

/**
 * §5.2 step 1: label keys parse, no split keys, user labels belong to the account, and the action fits
 * `account.caps`. Returns the label id of an added `l:` key whose label has no providerLabelId yet.
 */
export async function validateThreadAction(
  ctx: Pick<QueryCtx, "db">,
  account: Doc<"mailAccounts">,
  action: ThreadActionLike,
): Promise<{ waitForLabel?: Id<"labels"> }> {
  if (action.kind === "move" && account.caps.multiLabel) {
    throw appError("invalid_action", "Move is only for folder-based accounts.");
  }
  if (action.kind === "delete_forever" && !account.caps.deleteForever) {
    throw appError("unsupported", "Permanent delete is not available for this account.");
  }
  let waitForLabel: Id<"labels"> | undefined;
  const added =
    action.kind === "modify"
      ? new Set(action.add)
      : action.kind === "move"
        ? new Set([action.to])
        : new Set();
  for (const key of actionKeys(action)) {
    if (!isLabelKey(key) || isSplitKey(key)) throw appError("invalid_action", "Invalid label key.");
    const parsed = parseLabelKey(key);
    if (parsed?.kind !== "user") continue;
    const label = await ctx.db.get(parsed.labelId as Id<"labels">);
    if (!label || label.accountId !== account._id || label.pendingDelete) {
      throw appError("invalid_action", "Unknown label for this account.");
    }
    if (added.has(key) && label.providerLabelId === undefined) waitForLabel = label._id;
  }
  if (action.kind === "modify" && action.add.length + action.remove.length === 0) {
    throw appError("invalid_action", "Empty modify.");
  }
  return { waitForLabel };
}

/** Inserts one queued thread op (§5.2 steps 3–4). The caller recomputes the thread afterwards. */
export async function insertThreadOp(
  ctx: MutationCtx,
  args: {
    userId: string;
    account: Doc<"mailAccounts">;
    thread: Doc<"threads">;
    clientOpId: string;
    action: ThreadActionLike;
    inverseOf?: Id<"ops">;
    labelId?: Id<"labels">;
  },
): Promise<Id<"ops">> {
  const { before, after } = await previewAction(ctx, args.thread, args.account, args.action);
  const effect = diffKeys(before, after);
  const now = Date.now();
  const payload: OpPayload = { kind: "thread", action: args.action };
  return await ctx.db.insert("ops", {
    userId: args.userId,
    accountId: args.account._id,
    shard: args.account.shard,
    threadId: args.thread._id,
    labelId: args.labelId,
    clientOpId: args.clientOpId,
    payload,
    effect,
    inverseOf: args.inverseOf,
    state: "queued",
    attempt: 0,
    maxAttempts: maxAttemptsFor(payload),
    availableAt: now,
    failureAcked: false,
    createdAt: now,
  });
}

/** Validates, inserts and recomputes in one go. Returns the op id and the account's new rev. */
export async function enqueueThreadAction(
  ctx: MutationCtx,
  args: {
    userId: string;
    account: Doc<"mailAccounts">;
    thread: Doc<"threads">;
    clientOpId: string;
    action: ThreadActionLike;
    inverseOf?: Id<"ops">;
  },
): Promise<{ opId: Id<"ops">; rev: number }> {
  const { waitForLabel } = await validateThreadAction(ctx, args.account, args.action);
  if (waitForLabel) await ensureLabelCreate(ctx, args.userId, args.account, waitForLabel);
  const opId = await insertThreadOp(ctx, { ...args, labelId: waitForLabel });
  const r = await recomputeThread(ctx, args.thread._id);
  return { opId, rev: r?.rev ?? 0 };
}

/**
 * A label without a provider id gets created by a `label_create` op. User labels get theirs from
 * `labels.create`; the Archive folder placeholder of a folder account (D-392) gets one here, the first
 * time an action needs it, so the folder is created lazily at the provider (§5.1). Inserted before the
 * thread op that waits for it.
 */
async function ensureLabelCreate(
  ctx: MutationCtx,
  userId: string,
  account: Doc<"mailAccounts">,
  labelId: Id<"labels">,
): Promise<void> {
  for (const state of ["queued", "inflight"] as const) {
    const open = await ctx.db
      .query("ops")
      .withIndex("by_label_state", (q) => q.eq("labelId", labelId).eq("state", state))
      .filter((q) => q.eq(q.field("payload.kind"), "label_create"))
      .first();
    if (open) return;
  }
  await insertOp(ctx, {
    userId,
    account,
    clientOpId: `label_create:${labelId}:${Date.now()}`,
    payload: { kind: "label_create", labelId },
    labelId,
  });
}

/** Inserts a non-thread op (labels, drafts, sends). */
export async function insertOp(
  ctx: MutationCtx,
  args: {
    userId: string;
    account: Doc<"mailAccounts">;
    clientOpId: string;
    payload: OpPayload;
    labelId?: Id<"labels">;
    threadId?: Id<"threads">;
  },
): Promise<Id<"ops">> {
  const now = Date.now();
  return await ctx.db.insert("ops", {
    userId: args.userId,
    accountId: args.account._id,
    shard: args.account.shard,
    threadId: args.threadId,
    labelId: args.labelId,
    clientOpId: args.clientOpId,
    payload: args.payload,
    state: "queued",
    attempt: 0,
    maxAttempts: maxAttemptsFor(args.payload),
    availableAt: now,
    failureAcked: false,
    createdAt: now,
  });
}

/**
 * The inverse of an applied op for undo (§5.2): swap the effect; on folder providers location changes move
 * back. The rule is `inverseAction` in @akatsuki/mail-model (Rust twin `apply_ops::inverse_action`, D-250).
 */
export function inverseAction(op: Doc<"ops">, account: Doc<"mailAccounts">): ThreadActionLike | null {
  if (!op.effect || op.payload.kind !== "thread") return null;
  return sharedInverseAction(op.payload.action, op.effect, !account.caps.multiLabel);
}
