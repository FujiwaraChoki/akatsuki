// URL scheme of the SPA. Pure functions, so they are unit-tested and shared by the router and the palette.
//
//   /                      → inbox
//   /inbox                 the whole inbox          /inbox/split/<id>|other   one split tab
//   /starred /sent /drafts /snoozed /all /spam /trash /reminders
//   /label/<labelId>
//   …/t/<threadId>         a thread open in the reading pane (any mail view)
//   /search?q=…[&t=<threadId>]
//   ?account=<accountId>   scope a view to one mailbox (unified otherwise)
//   /signin  /onboarding[/imap]
//   /settings[/<section>]  /compose        part-2 slots (src/app/extensions.ts)
//   /billing  /billing/done                Stripe Checkout return URLs → /settings/billing
import { SPLIT_OTHER, SYS } from "@akatsuki/mail-model";

export const SYS_VIEWS = ["starred", "sent", "drafts", "snoozed", "all", "spam", "trash"] as const;
export type SysViewName = (typeof SYS_VIEWS)[number];

export type MailboxView =
  | { kind: "inbox"; split?: string /* "split:<id>" | "split:other" */ }
  | { kind: "sys"; name: SysViewName }
  | { kind: "reminders" }
  | { kind: "label"; labelId: string };

export type Route =
  | { name: "signin" }
  | { name: "onboarding"; step?: "imap" }
  | { name: "mail"; view: MailboxView; threadId?: string; accountId?: string }
  | { name: "search"; q: string; threadId?: string; accountId?: string }
  | { name: "settings"; section?: string }
  | { name: "compose" }
  | { name: "not_found"; path: string };

const ID = /^[A-Za-z0-9_-]{1,64}$/;
const isId = (s: string | undefined): s is string => s !== undefined && ID.test(s);

function decode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Parses `location.pathname` + `location.search`. Unknown paths give `not_found`. */
export function parseRoute(pathname: string, search = ""): Route {
  const params = new URLSearchParams(search);
  const parts = pathname.split("/").filter(Boolean).map(decode);
  const accountParam = params.get("account");
  const accountId = isId(accountParam ?? undefined) ? (accountParam as string) : undefined;
  const notFound: Route = { name: "not_found", path: pathname };

  if (parts.length === 0) return { name: "mail", view: { kind: "inbox" }, ...(accountId && { accountId }) };
  const [head, ...rest] = parts as [string, ...string[]];

  switch (head) {
    case "signin":
      return rest.length === 0 ? { name: "signin" } : notFound;
    case "onboarding":
      if (rest.length === 0) return { name: "onboarding" };
      return rest.length === 1 && rest[0] === "imap" ? { name: "onboarding", step: "imap" } : notFound;
    case "settings":
      if (rest.length === 0) return { name: "settings" };
      return rest.length === 1 && isId(rest[0]) ? { name: "settings", section: rest[0] } : notFound;
    case "compose":
      return rest.length === 0 ? { name: "compose" } : notFound;
    // Stripe Checkout returns to /billing/done?session_id=… and cancels to /billing (§13.1).
    case "billing":
      return rest.length === 0 || (rest.length === 1 && rest[0] === "done")
        ? { name: "settings", section: "billing" }
        : notFound;
    case "search": {
      if (rest.length > 0) return notFound;
      const t = params.get("t") ?? undefined;
      return {
        name: "search",
        q: params.get("q") ?? "",
        ...(isId(t) && { threadId: t }),
        ...(accountId && { accountId }),
      };
    }
  }

  let view: MailboxView;
  let tail: string[];
  if (head === "inbox") {
    if (rest[0] === "split") {
      const id = rest[1];
      if (!isId(id)) return notFound;
      view = { kind: "inbox", split: id === "other" ? SPLIT_OTHER : `split:${id}` };
      tail = rest.slice(2);
    } else {
      view = { kind: "inbox" };
      tail = rest;
    }
  } else if (head === "label") {
    if (!isId(rest[0])) return notFound;
    view = { kind: "label", labelId: rest[0] };
    tail = rest.slice(1);
  } else if (head === "reminders") {
    view = { kind: "reminders" };
    tail = rest;
  } else if ((SYS_VIEWS as readonly string[]).includes(head)) {
    view = { kind: "sys", name: head as SysViewName };
    tail = rest;
  } else return notFound;

  if (tail.length === 0) return { name: "mail", view, ...(accountId && { accountId }) };
  if (tail.length === 2 && tail[0] === "t" && isId(tail[1]))
    return { name: "mail", view, threadId: tail[1], ...(accountId && { accountId }) };
  return notFound;
}

function viewPath(view: MailboxView): string {
  switch (view.kind) {
    case "inbox":
      if (!view.split) return "/inbox";
      return view.split === SPLIT_OTHER ? "/inbox/split/other" : `/inbox/split/${view.split.slice(6)}`;
    case "sys":
      return `/${view.name}`;
    case "reminders":
      return "/reminders";
    case "label":
      return `/label/${encodeURIComponent(view.labelId)}`;
  }
}

/** Formats a route back into a path + query. `parseRoute(formatRoute(r))` equals `r`. */
export function formatRoute(route: Route): string {
  const q = new URLSearchParams();
  let path: string;
  switch (route.name) {
    case "signin":
      return "/signin";
    case "onboarding":
      return route.step ? `/onboarding/${route.step}` : "/onboarding";
    case "settings":
      return route.section ? `/settings/${route.section}` : "/settings";
    case "compose":
      return "/compose";
    case "not_found":
      return route.path;
    case "search":
      path = "/search";
      q.set("q", route.q);
      if (route.threadId) q.set("t", route.threadId);
      if (route.accountId) q.set("account", route.accountId);
      break;
    case "mail":
      path = viewPath(route.view);
      if (route.threadId) path += `/t/${encodeURIComponent(route.threadId)}`;
      if (route.accountId) q.set("account", route.accountId);
      break;
  }
  const qs = q.toString();
  return qs ? `${path}?${qs}` : path;
}

/** The `mail.listThreads` label key of a view (`"all"` for All mail), or `null` for timer-backed views. */
export function labelKeyOfView(view: MailboxView): string | null {
  switch (view.kind) {
    case "inbox":
      return view.split ?? SYS.inbox;
    case "sys":
      return view.name === "all" ? "all" : `sys:${view.name}`;
    case "label":
      return `l:${view.labelId}`;
    case "reminders":
      return null;
  }
}

export function sameView(a: MailboxView, b: MailboxView): boolean {
  return viewPath(a) === viewPath(b);
}

/** A stable string key for per-view UI state (cursor position, scroll). */
export function viewKey(view: MailboxView, accountId?: string): string {
  return `${viewPath(view)}${accountId ? `?a=${accountId}` : ""}`;
}

export const VIEW_TITLES: Record<SysViewName | "inbox" | "reminders", string> = {
  inbox: "Inbox",
  starred: "Starred",
  sent: "Sent",
  drafts: "Drafts",
  snoozed: "Snoozed",
  all: "All mail",
  spam: "Spam",
  trash: "Trash",
  reminders: "Reminders",
};
