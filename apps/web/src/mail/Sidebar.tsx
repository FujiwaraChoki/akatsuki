// The sidebar: mailboxes, splits, labels and accounts, with live unread counts (`mail.unreadCounts`) and
// account sync status. Every entry is also a palette command and most have a `g` shortcut.
import { SPLIT_OTHER } from "@akatsuki/mail-model";
import { Badge, Eyebrow, IconButton, Kbd, Tooltip } from "@akatsuki/ui";
import type { ReactNode } from "react";
import { Link } from "../app/router.tsx";
import { formatRoute, type MailboxView, sameView, VIEW_TITLES } from "../app/routes.ts";
import type { LabelView, MailAccountView, SplitView } from "../backend/types.ts";
import {
  IconAlert,
  IconBell,
  IconClock,
  IconCommand,
  IconDraft,
  IconInbox,
  IconLogout,
  IconMonitor,
  IconMoon,
  IconPlus,
  IconSend,
  IconSettings,
  IconStack,
  IconStar,
  IconSun,
  IconTag,
  IconTrash,
} from "../components/Icons.tsx";
import type { ThemePref } from "../lib/theme.ts";

export interface SidebarProps {
  view: MailboxView | null;
  accountId?: string;
  accounts: readonly MailAccountView[];
  labels: readonly LabelView[];
  splits: readonly SplitView[];
  counts: Record<string, number> | undefined;
  theme: ThemePref;
  onToggleTheme: () => void;
  onOpenPalette: () => void;
  onSignOut: () => void;
  onConnect: () => void;
  onOpenSettings: () => void;
}

const ENTRIES: { view: MailboxView; icon: ReactNode; shortcut: string; countKey?: string }[] = [
  { view: { kind: "inbox" }, icon: <IconInbox />, shortcut: "go.inbox", countKey: "sys:inbox" },
  { view: { kind: "sys", name: "starred" }, icon: <IconStar />, shortcut: "go.starred" },
  { view: { kind: "sys", name: "snoozed" }, icon: <IconClock />, shortcut: "go.snoozed" },
  { view: { kind: "reminders" }, icon: <IconBell />, shortcut: "go.reminders" },
  { view: { kind: "sys", name: "drafts" }, icon: <IconDraft />, shortcut: "go.drafts" },
  { view: { kind: "sys", name: "sent" }, icon: <IconSend />, shortcut: "go.sent" },
  { view: { kind: "sys", name: "all" }, icon: <IconStack />, shortcut: "go.all" },
  { view: { kind: "sys", name: "spam" }, icon: <IconAlert />, shortcut: "go.spam", countKey: "sys:spam" },
  { view: { kind: "sys", name: "trash" }, icon: <IconTrash />, shortcut: "go.trash" },
];

/** Labels worth listing: user labels, categories, keywords and non-system folders that are not hidden. */
export function visibleLabels(labels: readonly LabelView[]): LabelView[] {
  return labels
    .filter(
      (l) => !l.hidden && !l.pendingDelete && l.kind !== "system" && l.role !== "archive" && l.role !== "all",
    )
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** The unread-count keys the sidebar and split tabs ask for (≤ 30, the server cap). */
export function countKeys(splits: readonly SplitView[], labels: readonly LabelView[]): string[] {
  const keys = [
    "sys:inbox",
    "sys:spam",
    ...splits.filter((s) => s.showCount).map((s) => s.labelKey),
    SPLIT_OTHER,
  ];
  for (const l of visibleLabels(labels)) if (!keys.includes(l.labelKey)) keys.push(l.labelKey);
  return keys.slice(0, 30);
}

const STATUS_COPY: Record<MailAccountView["status"], string | null> = {
  connecting: "Connecting",
  initial_sync: "Syncing",
  live: null,
  degraded: "Sync delayed",
  reauth_required: "Needs reconnect",
  paused: "Paused",
  removing: "Removing",
};

export function Sidebar(props: SidebarProps) {
  const { view, accountId, accounts, labels, splits, counts } = props;
  const href = (v: MailboxView) => formatRoute({ name: "mail", view: v, ...(accountId && { accountId }) });
  const shownLabels = visibleLabels(labels);
  const multi = accounts.length > 1;
  const themeIcon =
    props.theme === "light" ? <IconSun /> : props.theme === "dark" ? <IconMoon /> : <IconMonitor />;

  return (
    <nav className="ak-sidebar" aria-label="Mailboxes">
      <div className="ak-sidebar__brand">
        <span className="ak-sidebar__mark" aria-hidden="true" />
        <span className="ak-sidebar__name">Akatsuki</span>
      </div>
      <button type="button" className="ak-sidebar__palette" onClick={props.onOpenPalette}>
        <IconCommand />
        <span>Commands</span>
        <Kbd shortcut="palette.open" />
      </button>
      <ul className="ak-sidebar__list">
        {ENTRIES.map((e) => {
          const active = view !== null && sameView(view, e.view) && !(view.kind === "inbox" && view.split);
          const label =
            e.view.kind === "sys"
              ? VIEW_TITLES[e.view.name]
              : VIEW_TITLES[e.view.kind as "inbox" | "reminders"];
          const n = e.countKey ? counts?.[e.countKey] : undefined;
          return (
            <li key={label}>
              <Link
                href={href(e.view)}
                className="ak-sidebar__item"
                aria-current={active ? "page" : undefined}
              >
                <span className="ak-sidebar__icon">{e.icon}</span>
                <span className="ak-sidebar__label">{label}</span>
                {n ? (
                  <Badge count={n} tone={e.countKey === "sys:inbox" ? "accent" : "neutral"} />
                ) : (
                  <Kbd shortcut={e.shortcut} className="ak-sidebar__kbd" />
                )}
              </Link>
            </li>
          );
        })}
      </ul>
      {splits.length > 0 && (
        <section className="ak-sidebar__section" aria-label="Splits">
          <Eyebrow>Splits</Eyebrow>
          <ul className="ak-sidebar__list">
            {[
              ...splits.map((s) => ({ key: s.labelKey, name: s.name })),
              { key: SPLIT_OTHER, name: "Other" },
            ].map((s) => {
              const v: MailboxView = { kind: "inbox", split: s.key };
              const active = view !== null && sameView(view, v);
              const n = counts?.[s.key];
              return (
                <li key={s.key}>
                  <Link
                    href={href(v)}
                    className="ak-sidebar__item"
                    aria-current={active ? "page" : undefined}
                  >
                    <span className="ak-sidebar__dot" aria-hidden="true" />
                    <span className="ak-sidebar__label">{s.name}</span>
                    {n ? <Badge count={n} tone="neutral" /> : null}
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {shownLabels.length > 0 && (
        <section className="ak-sidebar__section" aria-label="Labels">
          <Eyebrow>Labels</Eyebrow>
          <ul className="ak-sidebar__list">
            {shownLabels.map((l) => {
              const v: MailboxView = { kind: "label", labelId: l._id };
              const active = view !== null && sameView(view, v);
              const n = counts?.[l.labelKey];
              const owner = multi ? accounts.find((a) => a._id === l.accountId)?.email : undefined;
              return (
                <li key={l._id}>
                  <Link
                    href={href(v)}
                    className="ak-sidebar__item"
                    aria-current={active ? "page" : undefined}
                    title={owner}
                  >
                    <span className="ak-sidebar__icon">
                      <IconTag />
                    </span>
                    <span className="ak-sidebar__label">{l.name}</span>
                    {n ? <Badge count={n} tone="neutral" /> : null}
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      <section className="ak-sidebar__section ak-sidebar__accounts" aria-label="Accounts">
        <Eyebrow>Accounts</Eyebrow>
        <ul className="ak-sidebar__list">
          {multi && (
            <li>
              <Link
                href={formatRoute({ name: "mail", view: view ?? { kind: "inbox" } })}
                className="ak-sidebar__item"
                aria-current={!accountId ? "true" : undefined}
              >
                <span className="ak-sidebar__label">All accounts</span>
              </Link>
            </li>
          )}
          {accounts.map((a) => {
            const status = STATUS_COPY[a.status];
            return (
              <li key={a._id}>
                <Link
                  href={formatRoute({ name: "mail", view: view ?? { kind: "inbox" }, accountId: a._id })}
                  className="ak-sidebar__item ak-sidebar__account"
                  aria-current={accountId === a._id ? "true" : undefined}
                  data-status={a.status}
                >
                  <span className="ak-sidebar__status" aria-hidden="true" />
                  <span className="ak-sidebar__label">{a.displayName || a.email}</span>
                  {status && <span className="ak-sidebar__status-text">{status}</span>}
                </Link>
              </li>
            );
          })}
        </ul>
      </section>
      <div className="ak-sidebar__footer">
        <IconButton label="Connect a mailbox" icon={<IconPlus />} onClick={props.onConnect} />
        <IconButton
          label="Settings"
          icon={<IconSettings />}
          shortcut="settings.open"
          onClick={props.onOpenSettings}
        />
        <Tooltip content={`Theme: ${props.theme}`}>
          <button
            type="button"
            className="ak-icon-button ak-button--ghost ak-icon-button--md"
            aria-label={`Theme: ${props.theme}. Change theme`}
            onClick={props.onToggleTheme}
          >
            <span className="ak-icon-button__icon" aria-hidden="true">
              {themeIcon}
            </span>
          </button>
        </Tooltip>
        <IconButton label="Sign out" icon={<IconLogout />} onClick={props.onSignOut} />
      </div>
    </nav>
  );
}
