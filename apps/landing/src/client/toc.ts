// Document pages: the sticky table of contents marks the section being read.
import { $$ } from "./util";

export function initToc(nav: HTMLElement) {
  const links = $$<HTMLAnchorElement>("a[href^='#']", nav);
  const targets = links
    .map((a) => document.getElementById(decodeURIComponent(a.hash.slice(1))))
    .filter(Boolean) as HTMLElement[];
  if (!targets.length) return;
  const set = (id: string) => {
    for (const a of links) a.classList.toggle("is-active", a.hash.slice(1) === id);
  };
  const io = new IntersectionObserver(
    () => {
      let best = targets[0]!;
      for (const t of targets) if (t.getBoundingClientRect().top < window.innerHeight * 0.3) best = t;
      set(best.id);
    },
    { rootMargin: "0px 0px -60% 0px", threshold: [0, 1] },
  );
  for (const t of targets) io.observe(t);
}
