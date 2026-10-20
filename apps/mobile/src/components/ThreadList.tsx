// A virtualized thread list (FlashList) with swipe actions, a keyboard cursor and the list-scope shortcuts.
import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { router } from "expo-router";
import { type ReactElement, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RefreshControl } from "react-native";
import { type ThreadRef, useThreadActions } from "../state/actions";
import { useShortcuts } from "../state/keyboard";
import { useMail } from "../state/mail";
import { showToast } from "../state/toasts";
import { useTheme } from "../theme/ThemeProvider";
import { describeTime } from "../vm/snooze";
import { type SwipeAction, swipeSides } from "../vm/swipe";
import { moveSelection, type ThreadRowLike, type ThreadRowModel, threadRowModel } from "../vm/threadList";
import { ThreadRow } from "./ThreadRow";

export interface ThreadListProps {
  rows: readonly ThreadRowLike[];
  viewLabelKey: string;
  onEndReached?: () => void;
  header?: ReactElement | null;
  empty?: ReactElement | null;
  refreshing?: boolean;
  onRefresh?: () => void;
  /** Extra list-scope shortcuts (split.next / split.prev on the inbox). */
  extraShortcuts?: Partial<Record<string, () => void>>;
}

const ref = (r: ThreadRowLike): ThreadRef => ({ _id: r._id, accountId: r.accountId });

export function openThread(id: string) {
  router.push({ pathname: "/thread/[threadId]", params: { threadId: id } });
}

export function openSnooze(
  thread: ThreadRef,
  mode: "snooze" | "remind" = "snooze",
  from: "list" | "thread" = "list",
) {
  router.push({
    pathname: "/snooze",
    params: { threadId: thread._id, accountId: thread.accountId, mode, from },
  });
}

export function openCompose(kind: "reply" | "replyAll" | "forward", threadId: string) {
  router.push({ pathname: "/compose", params: { kind, threadId } });
}

export function ThreadList({
  rows,
  viewLabelKey,
  onEndReached,
  header,
  empty,
  refreshing,
  onRefresh,
  extraShortcuts,
}: ThreadListProps) {
  const t = useTheme();
  const mail = useMail();
  const actions = useThreadActions();
  const listRef = useRef<FlashListRef<ThreadRowModel>>(null);
  const [cursor, setCursor] = useState(-1);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  const byId = useMemo(() => new Map(rows.map((r) => [r._id, r])), [rows]);
  const describe = useCallback((at: number) => describeTime(at, now, mail.timeZone), [now, mail.timeZone]);

  // Keep the cursor on a valid row as rows leave the view.
  useEffect(() => {
    if (cursor >= rows.length) setCursor(rows.length - 1);
  }, [rows.length, cursor]);

  const selected = cursor >= 0 ? rows[cursor] : undefined;

  const onSwipe = useCallback(
    (id: string, action: SwipeAction) => {
      const row = byId.get(id);
      if (!row) return;
      if (action.id === "snooze") openSnooze(ref(row));
      else actions.run(action.id, [ref(row)]);
    },
    [byId, actions],
  );

  const move = (delta: number) => {
    const next = moveSelection(rows.length, cursor, delta);
    setCursor(next);
    if (next >= 0) listRef.current?.scrollToIndex({ index: next, viewPosition: 0.5, animated: true });
  };
  const onSelected = (fn: (r: ThreadRowLike) => void) => () => {
    if (selected) fn(selected);
  };

  useShortcuts("list", {
    "nav.next": () => move(1),
    "nav.prev": () => move(-1),
    "nav.open": onSelected((r) => openThread(r._id)),
    "thread.archive": onSelected((r) => actions.run("archive", [ref(r)])),
    "thread.moveToInbox": onSelected((r) => actions.run("moveToInbox", [ref(r)])),
    "thread.trash": onSelected((r) => actions.run("trash", [ref(r)])),
    "thread.spam": onSelected((r) => actions.run("spam", [ref(r)])),
    "thread.toggleRead": onSelected((r) =>
      actions.run(r.labelKeys.includes("sys:unread") ? "markRead" : "markUnread", [ref(r)], { undo: false }),
    ),
    "thread.star": onSelected((r) =>
      actions.run(r.labelKeys.includes("sys:starred") ? "unstar" : "star", [ref(r)], { undo: false }),
    ),
    "thread.snooze": onSelected((r) => openSnooze(ref(r))),
    "thread.remind": onSelected((r) => openSnooze(ref(r), "remind")),
    "thread.reply": onSelected((r) => openCompose("reply", r._id)),
    "thread.replyAll": onSelected((r) => openCompose("replyAll", r._id)),
    "thread.forward": onSelected((r) => openCompose("forward", r._id)),
    "thread.label": onSelected((r) =>
      router.push({
        pathname: "/palette",
        params: { mode: "label", threadId: r._id, accountId: r.accountId },
      }),
    ),
    "thread.move": onSelected((r) =>
      router.push({
        pathname: "/palette",
        params: { mode: "move", threadId: r._id, accountId: r.accountId },
      }),
    ),
    "thread.unsubscribe": () =>
      showToast({ id: "unsub-hint", text: "Open the conversation to unsubscribe." }),
    ...extraShortcuts,
  });

  const models = useMemo(
    () =>
      rows.map((r) =>
        threadRowModel(r, { now, timeZone: mail.timeZone, selfEmails: mail.selfEmails, describe }),
      ),
    [rows, now, mail.timeZone, mail.selfEmails, describe],
  );
  const sides = useMemo(() => rows.map((r) => swipeSides(viewLabelKey, r.labelKeys)), [rows, viewLabelKey]);

  return (
    <FlashList
      ref={listRef}
      data={models}
      keyExtractor={(m) => m.id}
      extraData={cursor}
      renderItem={({ item, index }) => (
        <ThreadRow
          model={item}
          sides={sides[index]!}
          selected={index === cursor}
          onOpen={openThread}
          onSwipe={onSwipe}
        />
      )}
      onEndReached={onEndReached}
      onEndReachedThreshold={0.6}
      ListHeaderComponent={header}
      ListEmptyComponent={empty}
      keyboardDismissMode="on-drag"
      contentContainerStyle={{ paddingBottom: t.space[12] }}
      refreshControl={
        onRefresh ? (
          <RefreshControl refreshing={!!refreshing} onRefresh={onRefresh} tintColor={t.colors.accent} />
        ) : undefined
      }
    />
  );
}
