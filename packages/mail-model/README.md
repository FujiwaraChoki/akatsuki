# @akatsuki/mail-model

The pure mail logic shared by Convex, web and mobile (ARCHITECTURE §1.4, §3.4, §5.1, §11.1, §16.1). No Convex
server code, no DOM, no I/O. Rust twins live in `crates/core::model` and `crates/search::query`; both sides run
the shared fixtures in `fixtures/`.

```ts
import { actionsFor, applyContextFor, applyOps, parseQuery, SYS } from "@akatsuki/mail-model";
import { applyOptimisticUpdate } from "@akatsuki/mail-model/optimistic"; // needs the `convex` client
```

## Entry points

| Import | What |
|---|---|
| `@akatsuki/mail-model` | Everything below except the optimistic layer. |
| `@akatsuki/mail-model/query` | Only the search grammar (Convex `search.ts` uses this). |
| `@akatsuki/mail-model/optimistic` | `optimisticApply` and the `withOptimisticUpdate` helpers (imports `convex/browser` types and `convex/server` `makeFunctionReference`). |
| `@akatsuki/mail-model/fixtures/*` | The shared JSON fixtures. |

The sources are TypeScript with explicit `.ts` import extensions (like every package here). Bundlers (Vite,
Metro, esbuild, the Convex CLI) and Node's type stripping load them directly; a consuming `tsconfig.json`
needs `"allowImportingTsExtensions": true` (with `noEmit`), as `convex/tsconfig.json` has.

## API

### Label keys (`labelKeys.ts`, §1.4)

- `parseLabelKey(key)` → `{kind:"sys",role} | {kind:"user",labelId} | {kind:"split",splitId} | null`;
  `isLabelKey`, `isSplitKey`, `isSysKey`.
- `SYS` (`SYS.inbox === "sys:inbox"`, …), `SYS_ROLES`, `SPLIT_OTHER`, `userKey(id)`, `splitKey(id)`,
  `sysKey(role)`, `labelKeyOf(labelRow)` (`sys:<role>` for the nine system roles, else `l:<_id>`).

### Visible state (`applyOps.ts`, §5.1)

- `applyOps(synced, actions, ctx)` → `{ keys, deleted }` (keys sorted and deduped). `applyAction` for one step.
- `applyContextFor(provider, exclusiveUserLabelIds)`: Gmail has no folder keys; Graph/IMAP have the six system
  folders plus every exclusive `l:` label. `isFolderProvider(provider)`.
- `diffKeys(before, after)` → `{ added, removed }` (`ops.effect`); `effectOf(state, action, ctx)`.
- `inverseAction(action, effect, folderProvider)` → the undo action or `null` (D-250):
  empty effect, `unsubscribe` and `delete_forever` have none; on folder providers `move`/`trash`/`untrash` are
  undone with `move{to: <first removed location key>}` (fallback `sys:inbox`); everything else swaps the
  effect. Convex `mail.undo` and Rust `apply_ops::inverse_action` use the same rule.

### Action helpers (`actions.ts`, §5.1 table) — the only way UIs build actions

```ts
const a = actionsFor(account /* {provider, caps?} */, labelsOfThatAccount /* [{_id, exclusive, role?}] */);
a.archive();          // gmail: modify{remove:[sys:inbox]}; graph/imap: move{to: l:<role "archive" folder>}
a.toggleRead(isUnread); a.toggleStar(isStarred); a.spam(); a.notSpam(); a.trash(); a.untrash();
a.moveToInbox(); a.snooze(); a.addLabel(id); a.removeLabel(id); a.moveTo(folderKey); a.unsubscribe("mailto");
a.deleteForever();    // throws ActionUnavailableError("unsupported") on Gmail
a.available("archive"); // false on a folder account without an archive folder: hide or disable the command
```

`ActionUnavailableError.code` is a §15 code (`unsupported` | `invalid_action`), so `packages/ui` error copy
applies directly.

### Rules and splits (`rules.ts`, §3.4)

`matchRule(rule, facts)`, `condMatches`, `firstMatchingSplit(splits, facts)`, `domainOf(email)`. `RuleFacts` is
exactly what a rule may see (sender, participants, label keys, triage, bulk, list id, subject, account, VIP).

### Aggregates (`aggregates.ts`, §3.3, §4.4)

`threadKeysFromMessages`, `threadAggregates` (newest 200 messages), `threadSearchText`, `CAPS` (§1.6), `truncate`.

### Search grammar (`query/`, §11.1, D-094)

`parseQuery(q, { nowAt, timezone })` → `{ ast, errors }` and `normalizeQuery(ast, timezone)`; `QueryAst`,
`IN_VALUES`, `IS_VALUES`, `HAS_VALUES`, `CATEGORY_VALUES`, `MAX_DEPTH`, plus the date helpers (`parseDay`,
`startOfDayMs`, `zonedMidnight`, `formatDay`, `relativeDays`). Characters are Unicode scalar values and
whitespace is Unicode White_Space, so error positions equal Rust's. Round `nowAt` down to the minute before
calling `search.threads` so the Convex query cache stays effective (§11.1).

### Optimistic updates (`optimistic.ts`, §16.1)

```ts
const apply = useMutation(api.mail.apply).withOptimisticUpdate(
  applyOptimisticUpdate({ contextFor: (accountId) => applyContextFor(provider, exclusiveIds) }),
);
const snooze = useMutation(api.mail.snooze).withOptimisticUpdate(
  snoozeOptimisticUpdate({ contextFor, snoozeActionFor: (accountId) => actionsFor(account).snooze() }),
);
```

`optimisticApply(localStore, threadIds, action, { contextFor })` patches every cached `mail.listThreads` page
(rows leave views that no longer contain them; rows are never inserted), `mail.getThread`, and moves
`mail.unreadCounts` (counts at the server cap of 100 are left alone). There is no rollback code: Convex drops
the optimistic layer when the server result lands. `viewContains(view, visible, row)` mirrors `listThreads`.

## Shared fixtures

| Fixture | TypeScript | Rust |
|---|---|---|
| `fixtures/label-keys.json` | `test/fixtures.test.ts` (`parseLabelKey`) | `crates/core::model::shared_fixtures` |
| `fixtures/apply-ops.json` `cases` | `applyOps`, `diffKeys` | `crates/core::model::shared_fixtures` |
| `fixtures/apply-ops.json` `inverses` | `inverseAction` (effect, inverse, restored state) | `crates/core::model::apply_ops` tests |
| `fixtures/rules.json` | `matchRule` | `crates/core::model::shared_fixtures` |
| `fixtures/query.json` | `parseQuery`, `normalizeQuery` | `crates/search::query` |

`convex/test/fixtures.test.ts` runs the same files through `convex/lib/model.ts` / `query.ts`, which are now
re-exports of this package. When a behaviour changes, change both implementations and the fixture together.
`make fixtures` runs every side.

## Scripts

`pnpm --filter @akatsuki/mail-model typecheck` and `test` (Vitest, no network).
