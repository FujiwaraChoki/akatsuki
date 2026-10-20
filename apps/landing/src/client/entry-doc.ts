// Secondary pages: only the site chrome (theme, header, menu, palette, shortcuts sheet, global
// keys) and the table of contents. The inbox, keys, timing and picker controllers stay on home.
import { bindRouter } from "./router";
import { initSite } from "./site";
import { initToc } from "./toc";
import { $, bindVisibility } from "./util";

bindVisibility();
bindRouter();
initSite();
const toc = $("[data-toc]");
if (toc) initToc(toc);
