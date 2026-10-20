// Fuzzy matching for the command palette. Every whitespace-separated query token must match the label or a
// keyword, either as a substring (best, with a bonus at word starts) or as an in-order subsequence. Pure.

export interface FuzzyMatch {
  score: number;
  /** Indices into the label of matched characters (for highlighting). */
  indices: number[];
}

const isWordStart = (s: string, i: number) => i === 0 || /[\s\-_/.:]/.test(s[i - 1]!);

/** Scores one token against one string, or `null` when it does not match. */
function tokenScore(token: string, hay: string): { score: number; indices: number[] } | null {
  const at = hay.indexOf(token);
  if (at >= 0) {
    // Prefer a substring that starts a word.
    let best = at;
    for (let i = at; i >= 0; i = hay.indexOf(token, i + 1)) {
      if (isWordStart(hay, i)) {
        best = i;
        break;
      }
    }
    const score = 100 + (isWordStart(hay, best) ? 40 : 0) + (best === 0 ? 20 : 0) - Math.min(best, 30) * 0.5;
    return {
      score: score + token.length * 2,
      indices: Array.from({ length: token.length }, (_, k) => best + k),
    };
  }
  const indices: number[] = [];
  let score = 0;
  let from = 0;
  for (const ch of token) {
    const i = hay.indexOf(ch, from);
    if (i < 0) return null;
    score += 10;
    if (isWordStart(hay, i)) score += 8;
    const prev = indices[indices.length - 1];
    if (prev !== undefined && i === prev + 1) score += 5;
    else if (prev !== undefined) score -= Math.min(i - prev - 1, 10);
    indices.push(i);
    from = i + 1;
  }
  return { score, indices };
}

/** Matches `query` against a label (and optional keywords). Empty queries match everything with score 0. */
export function fuzzyMatch(
  query: string,
  label: string,
  keywords: readonly string[] = [],
): FuzzyMatch | null {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return { score: 0, indices: [] };
  const lowLabel = label.toLowerCase();
  const lowKeywords = keywords.map((k) => k.toLowerCase());
  let total = 0;
  const indices = new Set<number>();
  for (const token of tokens) {
    const onLabel = tokenScore(token, lowLabel);
    let best = onLabel ? onLabel.score + 5 : null;
    for (const k of lowKeywords) {
      const r = tokenScore(token, k);
      if (r && (best === null || r.score > best)) best = r.score;
    }
    if (best === null) return null;
    total += best;
    if (onLabel) for (const i of onLabel.indices) indices.add(i);
  }
  return { score: total, indices: [...indices].sort((a, b) => a - b) };
}

/** Splits a label into plain and highlighted runs for rendering. */
export function highlightRuns(label: string, indices: readonly number[]): { text: string; match: boolean }[] {
  const set = new Set(indices);
  const runs: { text: string; match: boolean }[] = [];
  for (let i = 0; i < label.length; i++) {
    const match = set.has(i);
    const last = runs[runs.length - 1];
    if (last && last.match === match) last.text += label[i];
    else runs.push({ text: label[i]!, match });
  }
  return runs;
}
