// Account shards (ARCHITECTURE §9.2, D-044). `SHARD_COUNT` must be the same in the Convex deployment env and the
// worker's env: Convex fixes each account's shard at insert and the watchdog walks shards 0..SHARD_COUNT-1, the
// worker claims only its shard (§18, D-404). v1 runs one shard.

/** `SHARD_COUNT` from the deployment env, at least 1. */
export function shardCount(env: Record<string, string | undefined> = process.env): number {
  const n = Math.floor(Number(env.SHARD_COUNT ?? 1));
  return Number.isFinite(n) && n >= 1 ? n : 1;
}
