// The sidebar (First Light, SPEC §3.1.1): the account switcher, then mailboxes with their keys and counts, splits,
// labels and accounts, and two key hints at the foot. No logo, no icons in nav rows: the keys are the affordance.
// Every entry is also a palette command and most have a `g` shortcut; live unread counts come from
// `mail.unreadCounts`, account sync state from the accounts query.
import { SPLIT_OTHER } from "@akatsuki/mail-model";
import { formatKeys, type ShortcutId } from "@akatsuki/shortcuts";
import {
  firstKeys,
  Kbd,
  Menu,
  MenuContent,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  MenuTrigger,
  useUiConfig,
} from "@akatsuki/ui";
import { Link, navigate } from "../app/router.tsx";
import { formatRoute, type MailboxView, sameView, VIEW_TITLES } from "../app/routes.ts";
import type { LabelView, MailAccountView, SplitView } from "../backend/types.ts";
import { IconChevronDown } from "../components/Icons.tsx";
import { countLabel } from "../lib/format.ts";
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
  onOpenHelp: () => void;
  onSignOut: () => void;
  onConnect: () => void;
  onOpenSettings: () => void;
}

const ENTRIES: { view: MailboxView; shortcut: ShortcutId; countKey?: string }[] = [
  { view: { kind: "inbox" }, shortcut: "go.inbox", countKey: "sys:inbox" },
  { view: { kind: "sys", name: "starred" }, shortcut: "go.starred" },
  { view: { kind: "sys", name: "snoozed" }, shortcut: "go.snoozed" },
  { view: { kind: "reminders" }, shortcut: "go.reminders" },
  { view: { kind: "sys", name: "drafts" }, shortcut: "go.drafts" },
  { view: { kind: "sys", name: "sent" }, shortcut: "go.sent" },
  { view: { kind: "sys", name: "all" }, shortcut: "go.all" },
  { view: { kind: "sys", name: "spam" }, shortcut: "go.spam", countKey: "sys:spam" },
  { view: { kind: "sys", name: "trash" }, shortcut: "go.trash" },
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
  degraded: "Delayed",
  reauth_required: "Reconnect",
  paused: "Paused",
  removing: "Removing",
};

const PROVIDER_KIND: Record<string, string> = { gmail: "Gmail", graph: "Outlook", imap: "IMAP" };

/** "G I": the key hint of a nav row, as plain mono text (not keycaps), like the landing page. */
function KeyText({ shortcut }: { shortcut: ShortcutId }) {
  const config = useUiConfig();
  const keys = firstKeys(shortcut, config);
  if (!keys) return <span className="ak-sidebar__keys" />;
  return (
    <span className="ak-sidebar__keys" aria-hidden="true">
      {keys
        .trim()
        .split(/ +/)
        .map((k) => formatKeys(k, config.platform))
        .join(" ")}
    </span>
  );
}

function Count({ n }: { n: number | undefined }) {
  return <span className="ak-sidebar__count">{n ? countLabel(n) : ""}</span>;
}

export function Sidebar(props: SidebarProps) {
  const { view, accountId, accounts, labels, splits, counts } = props;
  const href = (v: MailboxView) => formatRoute({ name: "mail", view: v, ...(accountId && { accountId }) });
  const shownLabels = visibleLabels(labels);
  const multi = accounts.length > 1;
  const current = accountId ? accounts.find((a) => a._id === accountId) : undefined;
  const switchLabel = current ? current.email : "All inboxes";
  const accountHref = (id?: string) =>
    formatRoute({ name: "mail", view: view ?? { kind: "inbox" }, ...(id && { accountId: id }) });
  const nextTheme = props.theme === "system" ? "light" : props.theme === "light" ? "dark" : "system";

  return (
    <nav className="ak-sidebar" aria-label="Mailboxes">
      <Menu>
        <MenuTrigger>
          <button type="button" className="ak-sidebar__switch" aria-label={`${switchLabel}, account menu`}>
            <span className="ak-sidebar__switch-name">{switchLabel}</span>
            <IconChevronDown />
          </button>
        </MenuTrigger>
        <MenuContent align="start" className="ak-sidebar__menu">
          {(multi || accounts.length > 0) && <MenuLabel>Accounts</MenuLabel>}
          {multi && <MenuItem onSelect={() => navigate(accountHref())}>All inboxes</MenuItem>}
          {accounts.map((a) => (
            <MenuItem key={a._id} onSelect={() => navigate(accountHref(a._id))}>
              {a.email}
            </MenuItem>
          ))}
          <MenuItem onSelect={props.onConnect}>Add account</MenuItem>
          <MenuSeparator />
          <MenuItem shortcut="settings.open" onSelect={props.onOpenSettings}>
            Settings
          </MenuItem>
          <MenuItem onSelect={props.onToggleTheme}>
            Theme: {props.theme} (switch to {nextTheme})
          </MenuItem>
          <MenuSeparator />
          <MenuItem onSelect={props.onSignOut}>Sign out</MenuItem>
        </MenuContent>
      </Menu>
      <ul className="ak-sidebar__list">
        {ENTRIES.map((e) => {
          const active = view !== null && sameView(view, e.view) && !(view.kind === "inbox" && view.split);
          const label =
            e.view.kind === "sys"
              ? VIEW_TITLES[e.view.name]
              : VIEW_TITLES[e.view.kind as "inbox" | "reminders"];
          return (
            <li key={label}>
              <Link
                href={href(e.view)}
                className="ak-sidebar__item"
                aria-current={active ? "page" : undefined}
              >
                <span className="ak-sidebar__label">{label}</span>
                <KeyText shortcut={e.shortcut} />
                <Count n={e.countKey ? counts?.[e.countKey] : undefined} />
              </Link>
            </li>
          );
        })}
      </ul>
      {splits.length > 0 && (
        <section className="ak-sidebar__section" aria-label="Splits">
          <h2 className="ak-sidebar__heading">Splits</h2>
          <ul className="ak-sidebar__list">
            {[
              ...splits.map((s) => ({ key: s.labelKey, name: s.name })),
              { key: SPLIT_OTHER, name: "Other" },
            ].map((s) => {
              const v: MailboxView = { kind: "inbox", split: s.key };
              const active = view !== null && sameView(view, v);
              return (
                <li key={s.key}>
                  <Link
                    href={href(v)}
                    className="ak-sidebar__item"
                    aria-current={active ? "page" : undefined}
                  >
                    <span className="ak-sidebar__label">{s.name}</span>
                    <span className="ak-sidebar__keys" />
                    <Count n={counts?.[s.key]} />
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {shownLabels.length > 0 && (
        <section className="ak-sidebar__section" aria-label="Labels">
          <h2 className="ak-sidebar__heading">Labels</h2>
          <ul className="ak-sidebar__list">
            {shownLabels.map((l) => {
              const v: MailboxView = { kind: "label", labelId: l._id };
              const active = view !== null && sameView(view, v);
              const owner = multi ? accounts.find((a) => a._id === l.accountId)?.email : undefined;
              return (
                <li key={l._id}>
                  <Link
                    href={href(v)}
                    className="ak-sidebar__item ak-sidebar__item--label"
                    aria-current={active ? "page" : undefined}
                    title={owner}
                  >
                    <span className="ak-sidebar__mark" aria-hidden="true" />
                    <span className="ak-sidebar__label">{l.name}</span>
                    <Count n={counts?.[l.labelKey]} />
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {accounts.length > 0 && (
        <section className="ak-sidebar__section ak-sidebar__accounts" aria-label="Accounts">
          <h2 className="ak-sidebar__heading">Accounts</h2>
          <ul className="ak-sidebar__list">
            {accounts.map((a) => {
              const status = STATUS_COPY[a.status];
              return (
                <li key={a._id}>
                  <Link
                    href={accountHref(accountId === a._id ? undefined : a._id)}
                    className="ak-sidebar__item ak-sidebar__account"
                    aria-current={accountId === a._id ? "true" : undefined}
                    data-status={a.status}
                    title={accountId === a._id ? "Show all accounts" : `Show only ${a.email}`}
                  >
                    <span className="ak-sidebar__tile" aria-hidden="true">
                      {(a.email[0] ?? "?").toUpperCase()}
                    </span>
                    <span className="ak-sidebar__label">{a.displayName || a.email}</span>
                    <span className="ak-sidebar__kind">{status ?? PROVIDER_KIND[a.provider] ?? ""}</span>
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      <div className="ak-sidebar__footer">
        <button type="button" className="ak-sidebar__hint" onClick={props.onOpenPalette}>
          <Kbd shortcut="palette.open" />
          <span>Commands</span>
        </button>
        <button type="button" className="ak-sidebar__hint" onClick={props.onOpenHelp}>
          <Kbd shortcut="help.shortcuts" />
          <span>Shortcuts</span>
        </button>
        <button type="button" className="ak-sidebar__hint" onClick={props.onOpenSettings}>
          <Kbd shortcut="settings.open" />
          <span>Settings</span>
        </button>
      </div>
    </nav>
  );
}
