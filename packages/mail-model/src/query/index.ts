// Search query grammar (ARCHITECTURE §11.1). One grammar, two implementations: this module and
// crates/search::query. Both pass fixtures/query.json (ASTs, normalized text and exact error messages).
//
// Resolution rules (D-094):
// - `before:` / `after:` take `YYYY-MM-DD` or `YYYY/MM/DD` (1–2 digit month and day) and resolve to the start
//   of that day in `timezone` (`before:` = `date < at`, `after:` = `date >= at`). Relative values are not dates.
// - `older_than:` / `newer_than:` take `N(d|w|m|y)` and resolve to days: d=1, w=7, m=30, y=365.
// - `is:`, `has:`, `category:` and built-in `in:` values are lowercased; other values keep their case.
//   Unknown values are reported in `errors` but stay in the AST.
// - A malformed value (bad date, empty value) is reported and the whole token becomes a text word.
// - A one-element `and`/`or` collapses; the empty query is `{ and: [] }`; groups nest at most 32 deep.
// - A character is a Unicode scalar value and whitespace is Unicode White_Space, as in Rust.
export {
  CATEGORY_VALUES,
  HAS_VALUES,
  IN_VALUES,
  IS_VALUES,
  MAX_DEPTH,
  type QueryAst,
  type StringField,
} from "./ast.ts";
export { formatDay, parseDay, relativeDays, startOfDayMs, zonedMidnight } from "./dates.ts";
export { normalizeQuery } from "./normalize.ts";
export { parseQuery } from "./parse.ts";
