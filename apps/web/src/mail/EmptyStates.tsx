// Empty states. Inbox Zero gets the full treatment (hero orbs, Instrument Serif); other views a quiet line.
import { Eyebrow, Heading, Kbd, Orbs } from "@akatsuki/ui";
import type { ReactNode } from "react";
import type { MailboxView } from "../app/routes.ts";

const LINES = [
  "Nothing left. Enjoy the quiet.",
  "All clear. Go make something.",
  "The inbox is empty. The day is yours.",
  "Done for now. New mail lands here.",
];

export function InboxZero({ now = Date.now() }: { now?: number }) {
  const day = Math.floor(now / 86_400_000);
  const line = LINES[day % LINES.length]!;
  const date = new Intl.DateTimeFormat(undefined, { weekday: "long", month: "long", day: "numeric" }).format(
    now,
  );
  return (
    <div className="ak-zero" role="status">
      <Orbs variant="hero" />
      <div className="ak-zero__text">
        <Eyebrow>{date}</Eyebrow>
        <Heading level={2} variant="empty" className="ak-zero__title">
          Inbox zero.
        </Heading>
        <p className="ak-zero__line">{line}</p>
      </div>
    </div>
  );
}

const QUIET: Record<string, ReactNode> = {
  starred: (
    <>
      Nothing starred. Press <Kbd shortcut="thread.star" /> on a conversation to star it.
    </>
  ),
  snoozed: (
    <>
      Nothing snoozed. Press <Kbd shortcut="thread.snooze" /> to put a conversation away until you need it.
    </>
  ),
  reminders: (
    <>
      No reminders. Press <Kbd shortcut="thread.remind" /> on a conversation you are waiting on.
    </>
  ),
  drafts: "No drafts.",
  sent: "Nothing sent yet.",
  all: "No mail yet. New mail shows up here as it syncs.",
  spam: "No spam.",
  trash: "Trash is empty.",
  label: "Nothing with this label.",
  split: "Nothing in this split.",
};

export function EmptyView({ view, noAccounts }: { view: MailboxView; noAccounts: boolean }) {
  if (noAccounts)
    return (
      <div className="ak-empty">
        <Orbs variant="subtle" />
        <Heading level={2} variant="empty">
          No mailbox yet.
        </Heading>
        <p className="ak-empty__line">Connect Gmail, Outlook or any IMAP account to start.</p>
      </div>
    );
  if (view.kind === "inbox") return <InboxZero />;
  const key = view.kind === "sys" ? view.name : view.kind;
  return (
    <div className="ak-empty">
      <Orbs variant="subtle" />
      <p className="ak-empty__line">{QUIET[key] ?? "Nothing here."}</p>
    </div>
  );
}

export function NoResults({ q }: { q: string }) {
  return (
    <div className="ak-empty">
      <p className="ak-empty__line">
        No conversations match <q>{q}</q>.
      </p>
    </div>
  );
}

/** The idle reading pane: the empty-state treatment (faint orbs, a serif line) with the keys that matter. */
export function ReadingEmpty({ count }: { count: number }) {
  return (
    <div className="ak-empty ak-reading__empty">
      <Orbs variant="subtle" />
      <div className="ak-zero__text">
        <Heading level={2} variant="empty">
          {count > 0 ? "Pick a conversation." : "Nothing open."}
        </Heading>
        {count > 0 ? (
          <p className="ak-empty__line">
            <Kbd shortcut="nav.next" /> <Kbd shortcut="nav.prev" /> to move, <Kbd shortcut="nav.open" /> to
            open, <Kbd shortcut="palette.open" /> for everything else.
          </p>
        ) : null}
      </div>
    </div>
  );
}
