// Search query grammar (ARCHITECTURE §11.1, resolution rules D-094). The implementation lives in
// `@akatsuki/mail-model/query` (the TypeScript twin of crates/search::query, D-250); Convex re-exports it.
export {
  CATEGORY_VALUES,
  formatDay,
  HAS_VALUES,
  IN_VALUES,
  IS_VALUES,
  MAX_DEPTH,
  normalizeQuery,
  parseDay,
  parseQuery,
  type QueryAst,
  relativeDays,
  type StringField,
  startOfDayMs,
  zonedMidnight,
} from "@akatsuki/mail-model/query";
