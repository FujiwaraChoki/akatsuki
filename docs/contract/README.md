# Akatsuki technical contract

The binding technical contract is **[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md)**. This directory exists so that tooling and people looking for "the contract" find it. It holds no second copy, so there is nothing here that can drift.

- **Source of truth order**: [`docs/BRIEF.md`](../BRIEF.md) > [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) > code.
- **Why it looks the way it does**: [`docs/DECISIONS.md`](../DECISIONS.md). Entries D-001 to D-052 record the alternatives from the three reviewed proposals and the reason for each choice. Later entries (D-053 onward, numbered in blocks per area) record refinements, interim arrangements and contract changes made while building; a later entry wins over an earlier one on the same point.
- **Changing the contract**: edit `ARCHITECTURE.md` and add a `DECISIONS.md` entry in the same change. Interfaces that cross a team boundary (the Convex function signatures in §4, worker routes in §7, apps/api routes in §8, Rust traits in §9, and wire and error types in §7.3 and §15) need a reviewer from both sides.

## Where to start, by area

| You are implementing | Read |
|---|---|
| Convex schema or functions (`convex/`) | §1, §3, §4, §5, §10, §13 |
| Sync worker (`apps/sync`, `crates/providers`, `crates/core::sync`) | §1, §4.3, §5.4–5.6, §7, §9, §10, §14 |
| apps/api (`apps/api`) | §6.5–6.6, §8, §12, §13 |
| Web or mobile (`apps/web`, `apps/mobile`) | §1, §4.2, §5.1–5.2, §6.2/§6.4, §7.3, §14.3, §16 |
| Desktop (`apps/desktop`, `crates/*`) | §1, §5.1, §6.3, §11, §12, §14.3, §16.3 |
| AI (`packages/ai`, `packages/prompts`, `crates/ai`) | §12 |
| Shared logic (`packages/mail-model`, `packages/wire`) | §1.4, §1.7, §3.4, §5.1, §11.1, §12.6, §15 |
| Infra (`infra/`) | §6.6, §14.1, §18 (environment variables) |

## Shared fixtures (both implementations must pass)

| Fixture | TypeScript | Rust |
|---|---|---|
| `packages/mail-model/fixtures/label-keys.json` | `packages/mail-model` (interim: `convex/lib/model.ts`, run by `convex/test/fixtures.test.ts`) | `crates/core::model` |
| `packages/mail-model/fixtures/apply-ops.json` | `packages/mail-model` (interim: `convex/lib/model.ts`) | `crates/core::model` |
| `packages/mail-model/fixtures/rules.json` | `packages/mail-model` (interim: `convex/lib/model.ts`) | `crates/core::model` |
| `packages/mail-model/fixtures/query.json` | `packages/mail-model/src/query` (interim: `convex/lib/query.ts`) | `crates/search::query` |
| `packages/wire/fixtures/aasp/*.txt` (+ `*.json` expectations) | `packages/wire` (interim: `packages/ai/src/sse.ts`, run by `packages/ai/test/aasp-fixtures.test.ts`) | `crates/core::wire::aasp` |
| `packages/wire/fixtures/worker/*.json` | `packages/wire` (not yet: no TypeScript worker types exist) | `crates/core::wire::worker` |
| `packages/ai/fixtures/resolve/*.json`, `packages/ai/fixtures/settings/*.json`, `packages/ai/fixtures/openrouter/request.json` | `packages/ai` | `crates/ai` |
| `packages/prompts/shared/fixtures/{transcript,triage-items,at-cap}/*.json`, `packages/prompts/fixtures/render/*.json` | `packages/ai`, `packages/prompts` | `crates/ai` |

`make fixtures` runs all of them locally; `.github/workflows/ci.yml` runs them on every push and pull request. See D-126 for the interim homes.

## Open spike items

See ARCHITECTURE §19 (S1–S9). Do not build on an unverified item without its fallback ready.
