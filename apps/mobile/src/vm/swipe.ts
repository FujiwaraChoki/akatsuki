// Swipe actions on list rows. A swipe to the right archives (or restores, outside the inbox); a swipe to the left
// snoozes, and a long swipe to the left moves to Trash. The gesture layer reports the drag distance; this module
// decides which action that distance means, so the thresholds are tested without a device.
import { SYS } from "@akatsuki/mail-model";

export type SwipeActionId = "archive" | "moveToInbox" | "untrash" | "notSpam" | "snooze" | "trash";

export interface SwipeAction {
  id: SwipeActionId;
  label: string;
  /** Leaves the current view, so the row animates out. */
  removes: boolean;
}

export interface SwipeSides {
  /** Revealed when dragging right (from the leading edge). */
  leading: SwipeAction | null;
  /** Revealed when dragging left: [short, long]. */
  trailing: [SwipeAction | null, SwipeAction | null];
}

const A = (id: SwipeActionId, label: string, removes = true): SwipeAction => ({ id, label, removes });

/** The actions a row offers in a view, given the thread's visible label keys. */
export function swipeSides(viewLabelKey: string, threadKeys: readonly string[]): SwipeSides {
  const has = (k: string) => threadKeys.includes(k);
  const inView = (k: string) => viewLabelKey === k;
  let leading: SwipeAction | null;
  if (has(SYS.trash)) leading = A("untrash", "Restore", inView(SYS.trash));
  else if (has(SYS.spam)) leading = A("notSpam", "Not spam", inView(SYS.spam));
  else if (has(SYS.inbox))
    leading = A("archive", "Archive", inView(SYS.inbox) || viewLabelKey.startsWith("split:"));
  else leading = A("moveToInbox", "Move to Inbox", false);

  const snooze = has(SYS.trash) || has(SYS.spam) ? null : A("snooze", "Snooze");
  const trash = has(SYS.trash) ? null : A("trash", "Trash");
  return { leading, trailing: [snooze ?? trash, snooze ? trash : null] };
}

/** Fractions of the row width that arm an action. */
export const SWIPE_SHORT = 0.22;
export const SWIPE_LONG = 0.6;

/** 0 = nothing armed, 1 = the short action, 2 = the long action (trailing side only). */
export type SwipeStage = 0 | 1 | 2;

/** The stage for a drag of `dx` points on a row `width` wide (positive = to the right). */
export function swipeStage(dx: number, width: number, sides: SwipeSides): SwipeStage {
  if (width <= 0) return 0;
  const f = Math.abs(dx) / width;
  if (dx > 0) return sides.leading && f >= SWIPE_SHORT ? 1 : 0;
  if (dx < 0) {
    const [short, long] = sides.trailing;
    if (long && f >= SWIPE_LONG) return 2;
    if (short && f >= SWIPE_SHORT) return 1;
  }
  return 0;
}

/** The action a released drag performs, or null. */
export function swipeResult(dx: number, width: number, sides: SwipeSides): SwipeAction | null {
  const stage = swipeStage(dx, width, sides);
  if (stage === 0) return null;
  if (dx > 0) return sides.leading;
  return stage === 2 ? sides.trailing[1] : sides.trailing[0];
}

/** Haptic feedback fires when the armed stage changes in either direction (Superhuman-style detent). */
export function shouldBuzz(previous: SwipeStage, next: SwipeStage): boolean {
  return previous !== next;
}
