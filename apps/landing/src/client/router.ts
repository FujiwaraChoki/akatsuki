// One keydown listener for the whole page, in the priority order of §10:
// 1 focused text field, 2 open sheet or palette, 3 inbox focused, 4 inbox live, 5 keys section, 6 global.
import { isEditable } from "./util";

export type KeyHandler = (e: KeyboardEvent) => boolean;
interface Layer {
  prio: number;
  fn: KeyHandler;
}
const layers: Layer[] = [];

export function addLayer(prio: number, fn: KeyHandler) {
  layers.push({ prio, fn });
  layers.sort((a, b) => a.prio - b.prio);
}

let bound = false;
export function bindRouter() {
  if (bound) return;
  bound = true;
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.isComposing) return;
    // 1. Text fields keep every key; their own components listen on them directly.
    if (isEditable(e.target)) return;
    for (const l of layers) {
      if (l.fn(e)) return;
    }
  });
}
