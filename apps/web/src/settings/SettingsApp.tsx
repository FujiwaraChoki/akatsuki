// Settings (`/settings/<section>`): a sidebar of sections and the section's page. Keyboard-first like the mail
// shell: ↑/↓ or j/k move between sections, Escape or g i goes back to the inbox, Ctrl/⌘ K opens the palette
// with every section and the main navigation, Ctrl/⌘ , comes here from anywhere.
// First Light (SPEC §3.10, the Linear pattern): the nav has the sidebar's metrics with "← Back to mail Esc" on top;
// the page is a 640 column with one serif title, then hairline-bordered groups of rows.
import {
  CommandPalette,
  Heading,
  Kbd,
  type PaletteItem,
  paletteItemsFromShortcuts,
  useUiConfig,
} from "@akatsuki/ui";
import {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { Link, navigate } from "../app/router.tsx";
import { useBackend } from "../backend/context.tsx";
import type { MeView } from "../backend/types.ts";
import { ErrorBoundary, ErrorState } from "../components/ErrorState.tsx";
import { IconBack } from "../components/Icons.tsx";
import { isTextInput, useShortcuts } from "../keyboard/useShortcuts.ts";
import { type AppearancePatch, useSyncServerTheme } from "../lib/theme.ts";
import { SETTINGS_PAGES, type SettingsPageId } from "./pages.ts";
import { AccountsSection, IdentitiesSection } from "./sections/Accounts.tsx";
import { AiSection } from "./sections/Ai.tsx";
import { BillingSection, CheckoutStep } from "./sections/Billing.tsx";
import { DataSection, DevicesSection } from "./sections/Devices.tsx";
import {
  AppearanceSection,
  MailSection,
  NotificationsSection,
  ShortcutsSection,
} from "./sections/General.tsx";
import { LabelsSection } from "./sections/Labels.tsx";
import { RulesSection, SplitsSection } from "./sections/Rules.tsx";
import { SendersSection, SnippetsSection } from "./sections/Snippets.tsx";
import { SettingsPageTitle } from "./ui.tsx";

const RENDER: Record<SettingsPageId, (me: MeView) => ReactNode> = {
  accounts: () => <AccountsSection />,
  identities: () => <IdentitiesSection />,
  mail: (me) => <MailSection me={me} />,
  appearance: (me) => <AppearanceSection me={me} />,
  shortcuts: (me) => <ShortcutsSection me={me} />,
  notifications: (me) => <NotificationsSection me={me} />,
  splits: () => <SplitsSection />,
  rules: () => <RulesSection />,
  labels: () => <LabelsSection />,
  snippets: () => <SnippetsSection />,
  senders: () => <SendersSection />,
  ai: (me) => <AiSection me={me} />,
  billing: (me) => <BillingSection me={me} />,
  devices: (me) => <DevicesSection me={me} />,
  data: (me) => <DataSection me={me} />,
};

/** Below this width settings are list-then-detail: `/settings` is the list, each page has a Back button. */
export const SETTINGS_NARROW_QUERY = "(max-width: 719px)";

function useNarrow(): boolean {
  return useSyncExternalStore(
    (cb) => {
      const mq = typeof matchMedia === "function" ? matchMedia(SETTINGS_NARROW_QUERY) : null;
      mq?.addEventListener?.("change", cb);
      return () => mq?.removeEventListener?.("change", cb);
    },
    () => typeof matchMedia === "function" && matchMedia(SETTINGS_NARROW_QUERY).matches,
    () => false,
  );
}

export default function SettingsApp({ section }: { section?: string }) {
  const backend = useBackend();
  const me = backend.useMe();
  const ui = useUiConfig();
  const [palette, setPalette] = useState(false);
  const page = SETTINGS_PAGES.find((p) => p.id === section) ?? SETTINGS_PAGES[0]!;
  const narrow = useNarrow();
  // On a phone `/settings` is the list of pages on its own screen; a page replaces it (D-409).
  const view = narrow && !SETTINGS_PAGES.some((p) => p.id === section) ? "list" : "page";
  const nav = useRef<HTMLElement>(null);
  const mutations = backend.useMutations();
  const savePrefs = useCallback((patch: AppearancePatch) => mutations.updatePrefs({ patch }), [mutations]);
  useSyncServerTheme(me?.profile.prefs.theme, me?.profile.prefs.density, savePrefs);
  useEffect(() => {
    document.title = `${page.label} · Settings · Akatsuki`;
    return () => {
      document.title = "Akatsuki";
    };
  }, [page.label]);

  const go = (d: 1 | -1) => {
    const i = SETTINGS_PAGES.indexOf(page);
    const next = SETTINGS_PAGES[(i + d + SETTINGS_PAGES.length) % SETTINGS_PAGES.length]!;
    navigate(`/settings/${next.id}`);
    queueMicrotask(() => nav.current?.querySelector<HTMLElement>(`[data-section="${next.id}"]`)?.focus());
  };
  const handlers: Record<string, () => void> = {
    "palette.open": () => setPalette(true),
    "go.inbox": () => navigate("/inbox"),
    "go.starred": () => navigate("/starred"),
    "go.sent": () => navigate("/sent"),
    "go.drafts": () => navigate("/drafts"),
    // On a phone a page goes back to the list first.
    "nav.back": () => navigate(narrow && view === "page" ? "/settings" : "/inbox"),
    "nav.next": () => go(1),
    "nav.prev": () => go(-1),
  };
  // Inside a text field single keys type text (the "search" text scope); elsewhere j/k/Escape navigate.
  const [typing, setTyping] = useState(false);
  useEffect(() => {
    const update = () => setTyping(isTextInput(document.activeElement));
    const later = () => queueMicrotask(update);
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", later);
    return () => {
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", later);
    };
  }, []);
  useShortcuts({
    platform: ui.platform,
    keymap: ui.keymap,
    scopes: typing ? ["search"] : ["mail"],
    handlers,
    enabled: !palette,
  });

  const items: PaletteItem[] = [
    ...paletteItemsFromShortcuts(
      {
        "go.inbox": handlers["go.inbox"],
        "go.starred": handlers["go.starred"],
        "go.sent": handlers["go.sent"],
        "go.drafts": handlers["go.drafts"],
      },
      { platform: ui.platform, keymap: ui.keymap },
    ),
    ...SETTINGS_PAGES.map((p) => ({
      id: `settings:${p.id}`,
      label: `Settings: ${p.label}`,
      section: "Settings",
      keywords: [...p.keywords],
      run: () => navigate(`/settings/${p.id}`),
    })),
    { id: "session:signout", label: "Sign out", section: "Account", run: () => void backend.auth.signOut() },
  ];

  const onNavKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      go(e.key === "ArrowDown" ? 1 : -1);
    }
  };

  if (!me) return null;
  return (
    <div className="ak-settings" data-view={view}>
      <nav ref={nav} className="ak-settings__nav" aria-label="Settings" onKeyDown={onNavKey}>
        <Link href="/inbox" className="ak-settings__back">
          <IconBack />
          <span>Back to mail</span>
          <Kbd shortcut="nav.back" />
        </Link>
        <ul>
          {SETTINGS_PAGES.map((p) => (
            <li key={p.id}>
              <Link
                href={`/settings/${p.id}`}
                data-section={p.id}
                className="ak-settings__link"
                aria-current={view === "page" && p.id === page.id ? "page" : undefined}
              >
                {p.label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <main className="ak-settings__main" aria-label={page.label}>
        {narrow && (
          <div className="ak-settings__top">
            <Link href="/settings" className="ak-settings__up">
              <IconBack />
              <span>All settings</span>
            </Link>
          </div>
        )}
        <div className="ak-settings__page">
          <Heading level={1} variant="heading" className="ak-settings__title">
            {page.label}
          </Heading>
          <SettingsPageTitle.Provider value={page.label}>
            <ErrorBoundary
              resetKey={page.id}
              fallback={(e, reset) => <ErrorState error={e} onRetry={reset} />}
            >
              {RENDER[page.id](me)}
            </ErrorBoundary>
          </SettingsPageTitle.Provider>
        </div>
      </main>
      <CommandPalette open={palette} onOpenChange={setPalette} items={items} />
    </div>
  );
}

export { CheckoutStep };
