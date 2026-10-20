// A virtualized thread list (FlashList) with swipe actions, a keyboard cursor, the list-scope shortcuts and
// multi-select: long-press a row (or `x`) to start, tap rows to add or remove them, Shift+J/K to extend,
// Ctrl/⌘+A for every loaded row. While selecting, a bar at the top counts the selection and one at the bottom
// runs bulk actions; the same action keys (e, #, !, u, s, h, l, v) act on the selection.
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { router } from "expo-router";
import { type ReactElement, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BackHandler, Platform, RefreshControl, View } from "react-native";
import * as haptics from "../lib/haptics";
import { type ActionName, type ThreadRef, useThreadActions } from "../state/actions";
import { useKeyboard, useShortcuts } from "../state/keyboard";
import { useMail } from "../state/mail";
import { setPickerTargets } from "../state/targets";
import { showToast } from "../state/toasts";
import { useTheme } from "../theme/ThemeProvider";
import {
  addSelected,
  type BulkAction,
  bulkActions,
  EMPTY_SELECTION,
  extendSelection,
  isSelecting,
  pruneSelection,
  type Selection,
  selectAll,
  selectionTitle,
  toggleSelected,
} from "../vm/selection";
import { describeTime } from "../vm/snooze";
import { type SwipeAction, swipeSides } from "../vm/swipe";
import { moveSelection, type ThreadRowLike, type ThreadRowModel, threadRowModel } from "../vm/threadList";
import { ActionMenu } from "./ActionMenu";
import { HeaderButton } from "./Header";
import type { IconName } from "./Icon";
import { ThreadRow } from "./ThreadRow";
import { Toolbar, ToolbarButton } from "./Toolbar";

const BULK_ICON: Record<BulkAction["id"], IconName> = {
  archive: "archive",
  moveToInbox: "inbox",
  untrash: "undo",
  notSpam: "inbox",
  trash: "trash",
  spam: "octagon",
  markRead: "message",
  markUnread: "message",
  star: "star",
  unstar: "star",
  snooze: "clock",
  label: "tag",
  move: "inbox",
};

import { showKeyHints, Txt } from "./ui";

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
  /** Told when selection mode starts or ends (the inbox hides its Compose button meanwhile). */
  onSelectingChange?: (selecting: boolean) => void;
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

/** Run-once action names of the bulk bar (the pickers are handled separately). */
const RUNS: ReadonlySet<string> = new Set<ActionName>([
  "archive",
  "moveToInbox",
  "untrash",
  "notSpam",
  "trash",
  "spam",
  "markRead",
  "markUnread",
  "star",
  "unstar",
]);

export function ThreadList({
  rows,
  viewLabelKey,
  onEndReached,
  header,
  empty,
  refreshing,
  onRefresh,
  extraShortcuts,
  onSelectingChange,
}: ThreadListProps) {
  const t = useTheme();
  const mail = useMail();
  const keyboard = useKeyboard();
  const actions = useThreadActions();
  const listRef = useRef<FlashListRef<ThreadRowModel>>(null);
  const [cursor, setCursor] = useState(-1);
  const [now, setNow] = useState(() => Date.now());
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [more, setMore] = useState(false);
  const selecting = isSelecting(selection);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  const byId = useMemo(() => new Map(rows.map((r) => [r._id, r])), [rows]);
  const order = useMemo(() => rows.map((r) => r._id), [rows]);
  const describe = useCallback((at: number) => describeTime(at, now, mail.timeZone), [now, mail.timeZone]);

  // Keep the cursor on a valid row as rows leave the view, and the selection on rows still in it.
  useEffect(() => {
    if (cursor >= rows.length) setCursor(rows.length - 1);
  }, [rows.length, cursor]);
  useEffect(() => setSelection((s) => pruneSelection(s, order)), [order]);
  useEffect(() => onSelectingChange?.(selecting), [selecting, onSelectingChange]);

  const clear = useCallback(() => setSelection(EMPTY_SELECTION), []);

  // Android back leaves selection mode first.
  useEffect(() => {
    if (!selecting || Platform.OS !== "android") return;
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      clear();
      return true;
    });
    return () => sub.remove();
  }, [selecting, clear]);

  const selected = cursor >= 0 ? rows[cursor] : undefined;
  const selectedRefs = useMemo(
    () => rows.filter((r) => selection.ids.has(r._id)).map(ref),
    [rows, selection],
  );
  const available = useMemo(
    () =>
      bulkActions(
        rows
          .filter((r) => selection.ids.has(r._id))
          .map((r) => ({
            labelKeys: r.labelKeys,
            canMove: mail.actionsFor(r.accountId)?.available("moveTo") ?? false,
          })),
      ),
    [rows, selection, mail],
  );

  const onSwipe = useCallback(
    (id: string, action: SwipeAction) => {
      const row = byId.get(id);
      if (!row) return;
      if (action.id === "snooze") openSnooze(ref(row));
      else actions.run(action.id, [ref(row)]);
    },
    [byId, actions],
  );

  const onLongPress = useCallback((id: string) => {
    haptics.tick();
    setSelection((s) => addSelected(s, id));
  }, []);
  const onToggle = useCallback((id: string) => {
    haptics.tick();
    setSelection((s) => toggleSelected(s, id));
  }, []);

  /** Runs a bulk action on the selection; pickers get the selection as their targets. */
  const runBulk = useCallback(
    (a: BulkAction) => {
      const targets = selectedRefs;
      if (targets.length === 0) return;
      if (a.id === "snooze" || a.id === "label" || a.id === "move") {
        setPickerTargets(targets, clear);
        if (a.id === "snooze")
          router.push({ pathname: "/snooze", params: { bulk: "1", mode: "snooze", from: "list" } });
        else router.push({ pathname: "/palette", params: { mode: a.id, bulk: "1" } });
        return;
      }
      if (!RUNS.has(a.id)) return;
      const undo = a.id !== "markRead" && a.id !== "markUnread" && a.id !== "star" && a.id !== "unstar";
      actions.run(a.id as ActionName, targets, { undo });
      if (!undo) showToast({ id: "bulk", text: `${a.label}: ${targets.length}` });
      clear();
    },
    [selectedRefs, actions, clear],
  );

  /** A list shortcut acts on the selection while there is one, else on the cursor row. */
  const bulkOr = (shortcut: string, single: () => void) => () => {
    if (!selecting) return single();
    const a =
      shortcut === "thread.archive"
        ? available[0]
        : shortcut === "thread.trash"
          ? available.find((x) => x.id === "trash" || x.id === "untrash")
          : available.find((x) => x.shortcut === shortcut);
    if (a) runBulk(a);
  };

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
    "nav.open": onSelected((r) => (selecting ? onToggle(r._id) : openThread(r._id))),
    "nav.back": () => {
      if (selecting) clear();
    },
    "select.toggle": onSelected((r) => onToggle(r._id)),
    "select.extendDown": () => {
      const r = extendSelection(selection, order, cursor, 1);
      setSelection(r.selection);
      setCursor(r.cursor);
    },
    "select.extendUp": () => {
      const r = extendSelection(selection, order, cursor, -1);
      setSelection(r.selection);
      setCursor(r.cursor);
    },
    "select.all": () => setSelection((s) => selectAll(s, order)),
    "thread.archive": bulkOr(
      "thread.archive",
      onSelected((r) => actions.run("archive", [ref(r)])),
    ),
    "thread.moveToInbox": bulkOr(
      "thread.moveToInbox",
      onSelected((r) => actions.run("moveToInbox", [ref(r)])),
    ),
    "thread.trash": bulkOr(
      "thread.trash",
      onSelected((r) => actions.run("trash", [ref(r)])),
    ),
    "thread.spam": bulkOr(
      "thread.spam",
      onSelected((r) => actions.run("spam", [ref(r)])),
    ),
    "thread.toggleRead": bulkOr(
      "thread.toggleRead",
      onSelected((r) =>
        actions.run(r.labelKeys.includes("sys:unread") ? "markRead" : "markUnread", [ref(r)], {
          undo: false,
        }),
      ),
    ),
    "thread.star": bulkOr(
      "thread.star",
      onSelected((r) =>
        actions.run(r.labelKeys.includes("sys:starred") ? "unstar" : "star", [ref(r)], { undo: false }),
      ),
    ),
    "thread.snooze": bulkOr(
      "thread.snooze",
      onSelected((r) => openSnooze(ref(r))),
    ),
    "thread.remind": onSelected((r) => !selecting && openSnooze(ref(r), "remind")),
    "thread.reply": onSelected((r) => !selecting && openCompose("reply", r._id)),
    "thread.replyAll": onSelected((r) => !selecting && openCompose("replyAll", r._id)),
    "thread.forward": onSelected((r) => !selecting && openCompose("forward", r._id)),
    "thread.label": bulkOr(
      "thread.label",
      onSelected((r) =>
        router.push({
          pathname: "/palette",
          params: { mode: "label", threadId: r._id, accountId: r.accountId },
        }),
      ),
    ),
    "thread.move": bulkOr(
      "thread.move",
      onSelected((r) =>
        router.push({
          pathname: "/palette",
          params: { mode: "move", threadId: r._id, accountId: r.accountId },
        }),
      ),
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
  const hint = (id: string) => {
    if (!showKeyHints()) return null;
    const k = keysFor(getShortcut(id), keyboard.platform, keyboard.keymap)[0];
    return k ? formatKeys(k, keyboard.platform) : null;
  };
  const bar = available.slice(0, 4);
  const overflow = available.slice(4);

  return (
    <View style={{ flex: 1 }}>
      {selecting ? (
        <View
          accessibilityRole="toolbar"
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: t.space[1],
            paddingHorizontal: t.space[1],
            borderBottomWidth: 1,
            borderBottomColor: t.colors.borderSubtle,
            backgroundColor: t.colors.background,
          }}
        >
          <HeaderButton label="Cancel" onPress={clear} hint="Leaves selection mode" />
          <Txt
            variant="bodyStrong"
            style={{ flex: 1, textAlign: "center" }}
            accessibilityLiveRegion="polite"
            accessibilityRole="header"
          >
            {selectionTitle(selection, rows.length)}
          </Txt>
          <HeaderButton
            label={selection.ids.size === rows.length ? "None" : "All"}
            hint={
              selection.ids.size === rows.length
                ? "Clears the selection"
                : "Selects every loaded conversation"
            }
            onPress={() => setSelection((s) => selectAll(s, order))}
          />
        </View>
      ) : null}
      <FlashList
        ref={listRef}
        data={models}
        keyExtractor={(m) => m.id}
        extraData={[cursor, selection]}
        renderItem={({ item, index }) => (
          <ThreadRow
            model={item}
            sides={sides[index]!}
            selected={index === cursor}
            onOpen={openThread}
            onSwipe={onSwipe}
            onLongPress={onLongPress}
            selecting={selecting}
            checked={selection.ids.has(item.id)}
            onToggle={onToggle}
          />
        )}
        onEndReached={onEndReached}
        onEndReachedThreshold={0.6}
        ListHeaderComponent={selecting ? null : header}
        ListEmptyComponent={empty}
        keyboardDismissMode="on-drag"
        contentContainerStyle={{ paddingBottom: t.space[12] }}
        refreshControl={
          onRefresh && !selecting ? (
            <RefreshControl refreshing={!!refreshing} onRefresh={onRefresh} tintColor={t.colors.muted} />
          ) : undefined
        }
      />
      {selecting ? (
        <Toolbar label="Actions for the selected conversations">
          {bar.map((a) => (
            <ToolbarButton
              key={a.id}
              icon={BULK_ICON[a.id]}
              label={a.short}
              hint={`${a.label}: ${selection.ids.size} conversations`}
              onPress={() => runBulk(a)}
            />
          ))}
          {overflow.length > 0 ? (
            <ToolbarButton icon="more" label="More" onPress={() => setMore(true)} />
          ) : null}
        </Toolbar>
      ) : null}
      <ActionMenu
        visible={more}
        onClose={() => setMore(false)}
        title={selectionTitle(selection, rows.length)}
        items={overflow.map((a) => ({
          key: a.id,
          label: a.label,
          hint: hint(a.shortcut),
          destructive: a.destructive,
          onPress: () => runBulk(a),
        }))}
      />
    </View>
  );
}
