// The conversations a picker acts on when they are a multi-select (the label, move and snooze pickers are their
// own routes, and a selection can hold hundreds of ids, too many for route params). The list sets the targets
// before it opens a picker with `bulk=1`; the picker takes them once.
import type { ThreadRef } from "./actions";

let targets: ThreadRef[] = [];
const done = new Set<() => void>();

export function setPickerTargets(refs: readonly ThreadRef[], onDone?: () => void): void {
  targets = [...refs];
  done.clear();
  if (onDone) done.add(onDone);
}

/** The bulk targets (empty when none were set). */
export function pickerTargets(): ThreadRef[] {
  return targets;
}

/** Called by a picker after it acted on the targets (the list then clears its selection). */
export function finishPickerTargets(): void {
  targets = [];
  for (const f of [...done]) f();
  done.clear();
}
