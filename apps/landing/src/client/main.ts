// Entry point: attaches every controller whose markup is on the page. Runs once.
import { initInbox } from "./inbox";
import { initKeys } from "./keys";
import { initPicker } from "./picker";
import { bindRouter } from "./router";
import { initSite } from "./site";
import { initTiming } from "./timing";
import { initToc } from "./toc";
import { $, bindVisibility } from "./util";

let started = false;
export function start() {
  if (started) return;
  started = true;
  bindVisibility();
  bindRouter();
  initSite();
  const inbox = $("[data-inbox]");
  if (inbox) initInbox(inbox);
  const keys = $("[data-keys]");
  if (keys) initKeys(keys);
  const timing = $("[data-timeline]");
  if (timing) initTiming(timing);
  const picker = $("[data-picker]");
  if (picker) initPicker(picker);
  const toc = $("[data-toc]");
  if (toc) initToc(toc);
}
