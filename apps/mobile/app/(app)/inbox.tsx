// Split inbox and every other mailbox view (§3.4, §4.2 `mail.listThreads`, §16.2). The first page of each view is
// kept in SQLite so a cold start shows the last known list at once, then the live query takes over.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { SYS } from "@akatsuki/mail-model";
import { usePaginatedQuery, useQuery } from "convex/react";
import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, ScrollView, View } from "react-native";
import { EmptyState } from "../../src/components/EmptyState";
import { Header, HeaderButton } from "../../src/components/Header";
import { ThreadList } from "../../src/components/ThreadList";
import { Button, Screen, Spinner, Txt } from "../../src/components/ui";
import { convex, worker } from "../../src/lib/clients";
import { kvGet, kvSet } from "../../src/lib/db";
import { prefetchInbox } from "../../src/lib/prefetch";
import { signOutEverywhereOnDevice } from "../../src/lib/session";
import { useMail } from "../../src/state/mail";
import { useTheme } from "../../src/theme/ThemeProvider";
import { planStatus } from "../../src/vm/plan";
import { countKeys, cycleTab, mailboxById, splitTabs, type ThreadRowLike } from "../../src/vm/threadList";

const PAGE = 30; // mobile page size (§1.5)

function useCachedFirstPage(key: string, live: readonly ThreadRowLike[], liveReady: boolean) {
  const [cached, setCached] = useState<ThreadRowLike[] | null>(null);
  useEffect(() => {
    let alive = true;
    setCached(null);
    kvGet<ThreadRowLike[]>(`list:${key}`).then((rows) => {
      if (alive) setCached(rows);
    });
    return () => {
      alive = false;
    };
  }, [key]);
  useEffect(() => {
    if (!liveReady) return;
    const id = setTimeout(() => kvSet(`list:${key}`, live.slice(0, PAGE)).catch(() => {}), 1500);
    return () => clearTimeout(id);
  }, [key, live, liveReady]);
  return liveReady ? live : (cached ?? []);
}

function NotEntitled() {
  const t = useTheme();
  const mail = useMail();
  const status = planStatus(mail.me?.entitlement, mail.me?.billing, Date.now(), mail.timeZone);
  return (
    <EmptyState title={status.title} body={status.detail ?? undefined}>
      <Txt variant="meta" tone="muted" style={{ textAlign: "center", maxWidth: 320 }}>
        Signed in as {mail.me?.email}.
      </Txt>
      <Button title="Settings" onPress={() => router.push("/settings")} />
      <Button
        title="Sign out"
        variant="ghost"
        onPress={() => signOutEverywhereOnDevice().then(() => router.replace("/sign-in"))}
      />
      <View style={{ height: t.space[1] }} />
    </EmptyState>
  );
}

export default function Inbox() {
  const t = useTheme();
  const mail = useMail();
  const params = useLocalSearchParams<{ mailbox?: string; labelKey?: string; title?: string }>();
  const mailbox = mailboxById(params.mailbox);
  const isInbox = !params.labelKey && mailbox.id === "inbox";
  const splits = useQuery(api.splits.list, mail.entitled && isInbox ? {} : "skip");
  const baseTabs = useMemo(() => (isInbox ? splitTabs(splits, undefined) : []), [isInbox, splits]);
  const counts = useQuery(
    api.mail.unreadCounts,
    mail.entitled && baseTabs.length > 0 ? { labelKeys: countKeys(baseTabs) } : "skip",
  );
  const tabs = useMemo(() => (isInbox ? splitTabs(splits, counts) : []), [isInbox, splits, counts]);
  const [tab, setTab] = useState<string | null>(null);
  const activeTab = tabs.find((x) => x.key === tab)?.key ?? tabs[0]?.key ?? SYS.inbox;
  const labelKey = params.labelKey ?? (isInbox ? activeTab : mailbox.labelKey);
  const title = params.title ?? mailbox.title;

  const list = usePaginatedQuery(api.mail.listThreads, mail.entitled ? { view: { labelKey } } : "skip", {
    initialNumItems: PAGE,
  });
  const liveReady = list.status !== "LoadingFirstPage";
  const rows = useCachedFirstPage(labelKey, list.results as ThreadRowLike[], liveReady);
  const [refreshing, setRefreshing] = useState(false);

  // Wi-Fi prefetch of the newest inbox bodies (§16.2).
  useEffect(() => {
    if (!isInbox || !liveReady || !convex || list.results.length === 0) return;
    const ids = list.results.slice(0, 25).map((r) => r._id as string);
    const id = setTimeout(() => prefetchInbox(convex!, ids, mail.prefs?.cacheBodiesOnDevice ?? true), 2500);
    return () => clearTimeout(id);
  }, [isInbox, liveReady, list.results, mail.prefs?.cacheBodiesOnDevice]);

  const refresh = useCallback(async () => {
    if (!worker) return;
    setRefreshing(true);
    await Promise.allSettled(mail.accounts.map((a) => worker!.syncAccount(a._id as Id<"mailAccounts">)));
    setTimeout(() => setRefreshing(false), 600);
  }, [mail.accounts]);

  const loadMore = useCallback(() => {
    if (list.status === "CanLoadMore") list.loadMore(PAGE);
  }, [list]);

  const headerRight = (
    <>
      <HeaderButton label="Search" onPress={() => router.push("/search")} />
      <HeaderButton
        label="Commands"
        hint="Open the command palette"
        onPress={() => router.push({ pathname: "/palette", params: { scopes: "list" } })}
      />
    </>
  );

  if (mail.me === undefined)
    return (
      <Screen>
        <View style={{ flex: 1, alignItems: "center", justifyContent: "center" }}>
          <Spinner />
        </View>
      </Screen>
    );

  if (!mail.entitled)
    return (
      <Screen>
        <Header
          title="Akatsuki"
          right={<HeaderButton label="Settings" onPress={() => router.push("/settings")} />}
        />
        <NotEntitled />
      </Screen>
    );

  const splitBar =
    tabs.length > 1 ? (
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={{ paddingHorizontal: t.space[2], gap: t.space[2] }}
        style={{ flexGrow: 0, borderBottomWidth: 1, borderBottomColor: t.colors.borderSubtle }}
        accessibilityRole="tablist"
      >
        {tabs.map((x) => {
          const on = x.key === activeTab;
          return (
            <Pressable
              key={x.key}
              accessibilityRole="tab"
              accessibilityState={{ selected: on }}
              accessibilityLabel={x.count ? `${x.title}, ${x.count} unread` : x.title}
              onPress={() => setTab(x.key)}
              style={{
                paddingVertical: t.space[1],
                minHeight: 44,
                justifyContent: "center",
                borderBottomWidth: 2,
                borderBottomColor: on ? t.colors.accent : "transparent",
                flexDirection: "row",
                alignItems: "center",
                gap: t.space["0_5"],
              }}
            >
              <Txt variant="label" tone={on ? "text" : "muted"}>
                {x.title}
              </Txt>
              {x.count ? (
                <Txt variant="caption" tone="accent">
                  {x.count}
                </Txt>
              ) : null}
            </Pressable>
          );
        })}
      </ScrollView>
    ) : null;

  const empty =
    !liveReady && rows.length === 0 ? (
      <View style={{ padding: t.space[4], alignItems: "center" }}>
        <Spinner />
      </View>
    ) : mail.accounts.length === 0 ? (
      <EmptyState title="Connect a mailbox" body="Add Gmail, Outlook or any IMAP account to start.">
        <Button title="Connect a mailbox" variant="primary" onPress={() => router.push("/settings")} />
      </EmptyState>
    ) : (
      <EmptyState title={mailbox.emptyTitle} body={mailbox.emptyBody} />
    );

  return (
    <Screen>
      <Header title={title} onTitlePress={() => router.push("/mailboxes")} right={headerRight} />
      {splitBar}
      <View style={{ flex: 1 }}>
        <ThreadList
          rows={rows}
          viewLabelKey={labelKey}
          onEndReached={loadMore}
          empty={empty}
          refreshing={refreshing}
          onRefresh={refresh}
          extraShortcuts={
            tabs.length > 1
              ? {
                  "split.next": () => setTab(cycleTab(tabs, activeTab, 1)),
                  "split.prev": () => setTab(cycleTab(tabs, activeTab, -1)),
                }
              : {}
          }
        />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Compose"
          onPress={() => router.push("/compose")}
          style={({ pressed }) => ({
            position: "absolute",
            right: t.space[2],
            bottom: t.space[3],
            paddingHorizontal: t.space[3],
            minHeight: 52,
            borderRadius: t.radius.full,
            backgroundColor: t.colors.accent,
            justifyContent: "center",
            boxShadow: `0 0 ${t.effect.glowBlur}px ${t.colors.accentGlow}`,
            opacity: pressed ? 0.85 : 1,
          })}
        >
          <Txt variant="label" tone="onAccent">
            Compose
          </Txt>
        </Pressable>
      </View>
    </Screen>
  );
}
