// A small history-API router: the app has a dozen flat routes (src/app/routes.ts), so a dependency would
// buy nothing. `navigate()` is synchronous and every subscriber re-renders from `location`.
import { type AnchorHTMLAttributes, type MouseEvent, useSyncExternalStore } from "react";
import { parseRoute, type Route } from "./routes.ts";

const listeners = new Set<() => void>();
const notify = () => {
  for (const l of listeners) l();
};

function subscribe(cb: () => void) {
  listeners.add(cb);
  window.addEventListener("popstate", cb);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("popstate", cb);
  };
}

const snapshot = () => `${window.location.pathname}${window.location.search}`;

/** Pushes (or replaces) a history entry. No-op when already there. */
export function navigate(to: string, opts: { replace?: boolean } = {}): void {
  if (to === snapshot()) return;
  if (opts.replace) window.history.replaceState(null, "", to);
  else window.history.pushState(null, "", to);
  notify();
}

/** The current path + search, live. */
export function useLocation(): string {
  return useSyncExternalStore(subscribe, snapshot, () => "/");
}

/** The parsed current route. */
export function useRoute(): Route {
  const loc = useLocation();
  const q = loc.indexOf("?");
  return q === -1 ? parseRoute(loc) : parseRoute(loc.slice(0, q), loc.slice(q));
}

/** An `<a>` that navigates in-app on a plain left click. */
export function Link({ href, onClick, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  const handle = (e: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(e);
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    navigate(href);
  };
  return <a href={href} onClick={handle} {...rest} />;
}
