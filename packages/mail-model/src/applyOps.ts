// Visible thread state as a pure function (ARCHITECTURE §5.1):
//
//   labelKeys = applyOps(syncedLabelKeys, nonTerminalOpsInCreationOrder)
//
// Optimistic updates, provider echoes, failures, cancellations and undo all reduce to "recompute"; no code
// path writes labelKeys directly. Rust twin: crates/core::model::apply_ops. Both run
// fixtures/apply-ops.json (`cases` and `inverses`).
import { SYS } from "./labelKeys.ts";
import type { Provider, ThreadAction } from "./types.ts";

export interface ApplyContext {
  /**
   * Keys that are mutually exclusive locations on folder providers (graph/imap): sys:inbox, sys:trash,
   * sys:spam, sys:drafts, sys:sent, sys:snoozed and every l:<id> whose label.exclusive is true. Empty for
   * gmail.
   */
  folderKeys: ReadonlySet<string>;
}

/** A thread's visible state: keys sorted and de-duplicated; `deleted` while a delete_forever is pending. */
export interface Visible {
  keys: string[];
  deleted: boolean;
}

/** The label-key diff one action causes (`ops.effect`), used for undo. */
export interface Effect {
  added: string[];
  removed: string[];
}

export function sortDedupe(keys: readonly string[]): string[] {
  return [...new Set(keys)].sort();
}

const minus = (keys: readonly string[], remove: Iterable<string>) => {
  const r = new Set(remove);
  return keys.filter((k) => !r.has(k));
};

const norm = (state: Visible, keys: string[]): Visible => ({
  keys: sortDedupe(keys),
  deleted: state.deleted,
});

/** Applies one action to a visible state. */
export function applyAction(state: Visible, a: ThreadAction, ctx: ApplyContext): Visible {
  switch (a.kind) {
    case "modify":
      return norm(state, minus(state.keys, a.remove).concat(a.add));
    case "trash":
      return norm(
        state,
        minus(state.keys, [SYS.inbox, SYS.spam, SYS.snoozed, ...ctx.folderKeys]).concat(SYS.trash),
      );
    case "untrash":
      return norm(state, minus(state.keys, [SYS.trash]).concat(SYS.inbox));
    case "move":
      return norm(state, minus(state.keys, [...ctx.folderKeys]).concat(a.to));
    case "delete_forever":
      return { keys: [], deleted: true };
    case "unsubscribe":
      return state;
  }
}

/** Folds `actions` over the synced keys. */
export function applyOps(
  synced: readonly string[],
  actions: readonly ThreadAction[],
  ctx: ApplyContext,
): Visible {
  return actions.reduce<Visible>((s, a) => applyAction(s, a, ctx), {
    keys: sortDedupe(synced),
    deleted: false,
  });
}

/** The system keys that are folders on Graph and IMAP. */
export const FOLDER_SYS_KEYS: readonly string[] = [
  SYS.inbox,
  SYS.trash,
  SYS.spam,
  SYS.drafts,
  SYS.sent,
  SYS.snoozed,
];

const GMAIL_CONTEXT: ApplyContext = { folderKeys: new Set() };

/** The context for an account: no folder keys on Gmail; system folders plus exclusive labels elsewhere. */
export function applyContextFor(provider: Provider, exclusiveUserLabelIds: Iterable<string>): ApplyContext {
  if (provider === "gmail") return GMAIL_CONTEXT;
  return { folderKeys: new Set([...FOLDER_SYS_KEYS, ...[...exclusiveUserLabelIds].map((id) => `l:${id}`)]) };
}

/** `true` when the provider stores threads in mutually exclusive folders (`caps.multiLabel === false`). */
export const isFolderProvider = (provider: Provider): boolean => provider !== "gmail";

/** Keys added and removed going from `before` to `after` (ops.effect, undo). Both lists sorted. */
export function diffKeys(before: readonly string[], after: readonly string[]): Effect {
  const b = new Set(before);
  const a = new Set(after);
  return {
    added: [...a].filter((k) => !b.has(k)).sort(),
    removed: [...b].filter((k) => !a.has(k)).sort(),
  };
}

/** The effect one action has on a visible state. */
export function effectOf(state: Visible, action: ThreadAction, ctx: ApplyContext): Effect {
  return diffKeys(state.keys, applyAction(state, action, ctx).keys);
}

/** Keys that describe state, not location; never a destination for a folder move. */
const NON_LOCATION_KEYS: ReadonlySet<string> = new Set([SYS.unread, SYS.starred, SYS.important]);

/**
 * The inverse of an applied action for undo (ARCHITECTURE §5.2, D-250), or `null` when there is nothing to
 * undo (`mode: "noop"`).
 *
 * - An empty effect, `unsubscribe` and `delete_forever` have no inverse (a permanent delete can only be
 *   undone while its op is still queued, by cancelling it).
 * - On folder providers (`folderProvider`, Graph and IMAP) a location change (`move`, `trash`, `untrash`)
 *   is undone by moving back: `move{to}` where `to` is the first removed key (sorted) that is a location, or
 *   `sys:inbox` when the action removed no location (a thread that was in no folder).
 * - Everything else is undone by swapping the effect: `modify{add: effect.removed, remove: effect.added}`.
 *
 * Rust twin: crates/core::model::apply_ops::inverse_action. Vectors: fixtures/apply-ops.json `inverses`.
 */
export function inverseAction(
  action: ThreadAction,
  effect: Effect,
  folderProvider: boolean,
): ThreadAction | null {
  if (effect.added.length === 0 && effect.removed.length === 0) return null;
  if (action.kind === "unsubscribe" || action.kind === "delete_forever") return null;
  if (folderProvider && (action.kind === "move" || action.kind === "trash" || action.kind === "untrash")) {
    const back = [...effect.removed].sort().find((k) => !NON_LOCATION_KEYS.has(k)) ?? SYS.inbox;
    return { kind: "move", to: back };
  }
  return { kind: "modify", add: [...effect.removed], remove: [...effect.added] };
}
