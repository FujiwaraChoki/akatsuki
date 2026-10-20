// Empty states (First Light, SPEC §3.3). A folder that is empty gets a serif title, one muted line and one key.
// Inbox zero is the earned dawn: the horizon rises once (800ms, `cine`), once per day; after that the same words
// with no dawn. Reduced motion draws it in its final state.
import { Heading, Kbd, usePrefersReducedMotion } from "@akatsuki/ui";
import { type ReactNode, useState } from "react";
import type { MailboxView } from "../app/routes.ts";
import { clearedToday } from "./statusEcho.ts";

const DAWN_KEY = "ak:dawn-day";

/** True the first time inbox zero is reached today on this device (the dawn is not shown again until tomorrow). */
function useDawnOnce(now: number): boolean {
  const [show] = useState(() => {
    const day = new Date(now).toDateString();
    try {
      if (localStorage.getItem(DAWN_KEY) === day) return false;
      localStorage.setItem(DAWN_KEY, day);
    } catch {
      /* storage off: show it */
    }
    return true;
  });
  return show;
}

export function InboxZero({ now = Date.now() }: { now?: number }) {
  const dawn = useDawnOnce(now);
  const still = usePrefersReducedMotion();
  const cleared = clearedToday(now);
  return (
    <div className="ak-zero" role="status" data-dawn={dawn || undefined} data-still={still || undefined}>
      {dawn && (
        <div className="ak-zero__dawn" aria-hidden="true">
          <span className="ak-zero__band" />
          <span className="ak-zero__line" />
        </div>
      )}
      <div className="ak-zero__copy">
        <Heading level={2} variant="empty" className="ak-zero__title">
          Inbox zero
        </Heading>
        {cleared > 0 && (
          <p className="ak-zero__tally">
            {cleared} conversation{cleared === 1 ? "" : "s"} cleared today
          </p>
        )}
        <p className="ak-zero__hint">
          <Kbd shortcut="undo" /> to undo the last one · <Kbd shortcut="go.all" /> for All mail
        </p>
      </div>
    </div>
  );
}

const QUIET: Record<string, { title: string; line: ReactNode }> = {
  starred: {
    title: "Nothing starred",
    line: (
      <>
        Press <Kbd shortcut="thread.star" /> on a conversation to star it.
      </>
    ),
  },
  snoozed: {
    title: "Nothing snoozed",
    line: (
      <>
        Press <Kbd shortcut="thread.snooze" /> to put a conversation away until you need it.
      </>
    ),
  },
  reminders: {
    title: "No reminders",
    line: (
      <>
        Press <Kbd shortcut="thread.remind" /> on a conversation you are waiting on.
      </>
    ),
  },
  drafts: {
    title: "No drafts",
    line: (
      <>
        Press <Kbd shortcut="compose.new" /> to write one.
      </>
    ),
  },
  sent: { title: "Nothing sent yet", line: "What you send shows up here." },
  all: { title: "No mail yet", line: "New mail shows up here as it syncs." },
  spam: { title: "No spam", line: "Mail you report as spam waits here." },
  trash: { title: "Trash is empty", line: "Deleted conversations wait here for 30 days." },
  label: { title: "Nothing here", line: "No conversation has this label." },
  split: { title: "Nothing here", line: "No conversation in this split." },
};

export function EmptyView({ view, noAccounts }: { view: MailboxView; noAccounts: boolean }) {
  if (noAccounts)
    return (
      <div className="ak-empty">
        <Heading level={2} variant="empty">
          No mailbox yet
        </Heading>
        <p className="ak-empty__line">Connect Gmail, Outlook or any IMAP account to start.</p>
      </div>
    );
  // Splits count as inbox zero too (the split is the inbox you look at).
  if (view.kind === "inbox") return <InboxZero />;
  const copy = QUIET[view.kind === "sys" ? view.name : view.kind] ?? { title: "Nothing here", line: "" };
  return (
    <div className="ak-empty">
      <Heading level={2} variant="empty">
        {copy.title}
      </Heading>
      {copy.line && <p className="ak-empty__line">{copy.line}</p>}
    </div>
  );
}

export function NoResults({ q }: { q: string }) {
  return (
    <div className="ak-empty">
      <Heading level={2} variant="subject" className="ak-empty__search">
        No results for “{q}”
      </Heading>
      <p className="ak-empty__line">Try fewer words, or search the full mailbox from the bar above.</p>
    </div>
  );
}

/** The idle reading pane: a plain serif line and the keys that matter. */
export function ReadingEmpty({ count }: { count: number }) {
  if (count === 0) return null;
  return (
    <div className="ak-empty ak-reading__empty">
      <div className="ak-zero__copy">
        <Heading level={2} variant="subject" className="ak-reading__title">
          {count > 0 ? "Pick a conversation." : "Nothing open."}
        </Heading>
        {count > 0 ? (
          <p className="ak-empty__line">
            <Kbd shortcut="nav.next" />
            <Kbd shortcut="nav.prev" /> to move · <Kbd shortcut="nav.open" /> to open ·{" "}
            <Kbd shortcut="palette.open" /> for everything else
          </p>
        ) : null}
      </div>
    </div>
  );
}
