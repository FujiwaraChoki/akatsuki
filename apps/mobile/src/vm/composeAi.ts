// Write with AI and rewrite in the mobile composer (ARCHITECTURE §12.1 `compose`, `rewrite`; web parity with
// apps/web Composer). The model's text streams into the body in place of what it replaces; when it is done the
// composer offers Undo, which puts the body back as it was. Pure: the screen holds one `AiEdit` while a stream
// runs and turns stream state into body text with these functions.

export type RewriteStyle = "shorter" | "friendlier" | "formal" | "fix";

export const REWRITE_STYLES: readonly { style: RewriteStyle; label: string }[] = [
  { style: "shorter", label: "Shorter" },
  { style: "friendlier", label: "Friendlier" },
  { style: "formal", label: "More formal" },
  { style: "fix", label: "Fix spelling and grammar" },
];

/** What a running AI edit replaces: `[start, end)` of the body as it was when the edit started. */
export interface AiEdit {
  kind: "write" | "rewrite";
  start: number;
  end: number;
  before: string;
}

/** The body while (and after) the model streams `out` into the edit's range. */
export function bodyWithAi(edit: AiEdit, out: string): string {
  return edit.before.slice(0, edit.start) + out + edit.before.slice(edit.end);
}

/**
 * The range a rewrite works on: the selection when there is one, else the whole body. Null when that text is
 * blank (nothing to rewrite). A reply quote below the body is not part of `text` on mobile, so it is never sent.
 */
export function rewriteRange(
  text: string,
  selection: { start: number; end: number } | null,
): { start: number; end: number; text: string } | null {
  let start = 0;
  let end = text.length;
  if (selection && selection.end > selection.start) {
    start = Math.max(0, Math.min(selection.start, text.length));
    end = Math.max(start, Math.min(selection.end, text.length));
  }
  const slice = text.slice(start, end);
  return slice.trim() ? { start, end, text: slice } : null;
}

/** A write replaces the whole body (the model gets the current draft as `draft`). */
export function writeEdit(body: string): AiEdit {
  return { kind: "write", start: 0, end: body.length, before: body };
}

export function rewriteEdit(body: string, range: { start: number; end: number }): AiEdit {
  return { kind: "rewrite", start: range.start, end: range.end, before: body };
}

/** The instruction box: required, and capped well under the feature's input budget. */
export const INSTRUCTION_MAX = 2000;

export function instructionProblem(instruction: string): string | null {
  const s = instruction.trim();
  if (!s) return "Say what to write, for example “accept and suggest Tuesday”.";
  if (s.length > INSTRUCTION_MAX) return "Keep the instruction under 2,000 characters.";
  return null;
}

/** Quick prompts under the instruction box (tapping one fills the box). */
export function writeSuggestions(isReply: boolean): string[] {
  return isReply
    ? ["Say yes and thank them", "Politely decline", "Ask for more details", "Suggest a time to talk"]
    : ["Introduce myself", "Ask for a meeting next week", "Follow up on my last email"];
}
