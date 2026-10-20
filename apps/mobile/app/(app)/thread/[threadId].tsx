// One conversation (§4.2 `mail.getThread`, §7.2 bodies, §14.3 sandboxed rendering). Opening it marks it read.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { SYS } from "@akatsuki/mail-model";
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { useMutation, useQuery } from "convex/react";
import * as Linking from "expo-linking";
import { type ErrorBoundaryProps, router, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert, Pressable, View } from "react-native";
import { ActionMenu, type MenuItem } from "../../../src/components/ActionMenu";
import { Header, HeaderButton } from "../../../src/components/Header";
import { ThreadBody } from "../../../src/components/ThreadBody";
import { openCompose, openSnooze } from "../../../src/components/ThreadList";
import { Button, Screen, Spinner, Txt } from "../../../src/components/ui";
import { ENV } from "../../../src/env";
import { useInlineImages, useThreadBodies } from "../../../src/lib/bodies";
import { worker } from "../../../src/lib/clients";
import { errorText } from "../../../src/lib/errors";
import { useThreadAi } from "../../../src/lib/threadAi";
import { type ActionName, useThreadActions } from "../../../src/state/actions";
import { useKeyboard, useShortcuts } from "../../../src/state/keyboard";
import { useMail } from "../../../src/state/mail";
import { showToast } from "../../../src/state/toasts";
import { useTheme } from "../../../src/theme/ThemeProvider";
import { type DocMessage, initiallyExpanded, threadDocument } from "../../../src/vm/bodyDocument";
import { displayName, formatAddress, fullDate } from "../../../src/vm/format";
import { type ThreadAiFeature, threadForAi } from "../../../src/vm/threadAi";
import { mailtoConfirmation } from "../../../src/vm/unsubscribe";

export function ErrorBoundary({ error, retry }: ErrorBoundaryProps) {
  const t = useTheme();
  return (
    <Screen>
      <Header title="Conversation" onBack={() => router.back()} />
      <View style={{ padding: t.space[3], gap: t.space[2] }}>
        <Txt variant="reading">{errorText(error)}</Txt>
        <Button title="Try again" onPress={retry} />
      </View>
    </Screen>
  );
}

export default function ThreadScreen() {
  const t = useTheme();
  const mail = useMail();
  const keyboard = useKeyboard();
  const actions = useThreadActions();
  const { threadId } = useLocalSearchParams<{ threadId: string }>();
  const data = useQuery(
    api.mail.getThread,
    mail.entitled && threadId ? { threadId: threadId as Id<"threads"> } : "skip",
  );
  const [menu, setMenu] = useState(false);
  const [showImagesNow, setShowImagesNow] = useState(false);
  const [expandAll, setExpandAll] = useState(false);
  const markedRead = useRef(false);

  const messages = useMemo(() => (data?.messages ?? []).filter((m) => !m.isDraft), [data?.messages]);
  const messageIds = useMemo(() => messages.map((m) => m._id as string), [messages]);
  const lastSender = messages[messages.length - 1]?.from?.email;
  const senderPref = useQuery(
    api.senderPrefs.get,
    lastSender && mail.entitled ? { sender: lastSender } : "skip",
  );
  const setSenderPref = useMutation(api.senderPrefs.set);
  const persist = mail.prefs?.cacheBodiesOnDevice ?? true;
  const { bodies, retry } = useThreadBodies(threadId ?? "", messageIds, persist);
  const inlineImages = useInlineImages(bodies);
  // Unread messages and the newest start open. Decided once per message, so marking the thread read does not
  // collapse what was open; messages that arrive later get the same rule.
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const seen = useRef(new Set<string>());
  useEffect(() => {
    const fresh = messages.filter((m) => !seen.current.has(m._id as string));
    if (fresh.length === 0) return;
    for (const m of fresh) seen.current.add(m._id as string);
    const init = initiallyExpanded(messages.map((m) => ({ id: m._id as string, isUnread: m.isUnread })));
    const freshIds = new Set(fresh.map((m) => m._id as string));
    setOpened((prev) => new Set([...prev, ...[...init].filter((id) => freshIds.has(id))]));
  }, [messages]);
  const expanded = useMemo(() => (expandAll ? new Set(messageIds) : opened), [expandAll, messageIds, opened]);

  const thread = data?.thread;
  const ref = thread ? { _id: thread._id as string, accountId: thread.accountId as string } : null;

  // Opening a conversation marks it read (no undo toast; `u` marks it unread again).
  useEffect(() => {
    if (!thread || !ref || markedRead.current) return;
    markedRead.current = true;
    if (thread.labelKeys.includes(SYS.unread)) actions.run("markRead", [ref], { undo: false });
  }, [thread, ref, actions]);

  const remoteImages = Object.values(bodies).reduce((n, b) => n + (b.body?.remoteImageCount ?? 0), 0);
  const showImages =
    showImagesNow || mail.prefs?.remoteImages === "proxy" || senderPref?.allowRemoteImages === true;

  const html = useMemo(() => {
    const now = Date.now();
    const docMessages: DocMessage[] = messages.map((m) => {
      const b = bodies[m._id as string];
      return {
        id: m._id as string,
        from: displayName(m.from),
        fromEmail: m.from?.email ?? "",
        to: `To ${[...m.to, ...m.cc].map(formatAddress).join(", ") || "undisclosed recipients"}`,
        date: fullDate(m.date, now, mail.timeZone),
        snippet: m.snippet,
        expanded: expanded.has(m._id as string),
        state: b?.state ?? "loading",
        body: b?.body ?? null,
      };
    });
    return threadDocument(docMessages, {
      theme: {
        background: t.colors.background,
        foreground: t.colors.text,
        muted: t.colors.muted,
        border: t.colors.borderSubtle,
        accent: t.colors.accentText,
        paper: t.colors.paper,
        paperText: t.colors.paperText,
        dark: t.mode === "dark",
      },
      imageOrigin: ENV.env?.syncUrl ?? "",
      showImages,
      inlineImages,
    });
  }, [messages, bodies, expanded, mail.timeZone, t, showImages, inlineImages]);

  const after = useCallback(
    (name: ActionName) => {
      if (!ref) return;
      actions.run(name, [ref]);
      router.back();
    },
    [ref, actions],
  );

  // Summarize and Suggest replies (§12, D-408): from the bodies this screen already holds.
  const aiInput = useCallback(
    () =>
      messages.length === 0
        ? null
        : threadForAi(
            messages.map((m) => ({
              from: m.from ?? null,
              to: m.to,
              cc: m.cc,
              date: m.date,
              snippet: m.snippet,
            })),
            (i) => bodies[messageIds[i]!]?.body ?? null,
            mail.timeZone,
          ),
    [messages, bodies, messageIds, mail.timeZone],
  );
  const ai = useThreadAi(aiInput);
  const runAi = useCallback(
    (feature: ThreadAiFeature) => {
      void ai.run(feature).then((problem) => {
        if (!problem) return;
        showToast({
          id: "thread-ai",
          text: problem,
          actions: [{ label: "Open AI settings", run: () => router.push("/settings/ai") }],
        });
      });
    },
    [ai],
  );

  const latestBody = messageIds.length > 0 ? bodies[messageIds[messageIds.length - 1]!]?.body : undefined;
  const unsub = latestBody?.headers.listUnsubscribe ?? null;

  const unsubscribe = useCallback(() => {
    if (!ref || !unsub) return;
    if (unsub.oneClick && unsub.http) actions.unsubscribe(ref, "one_click_post");
    else if (unsub.mailto) {
      // An email goes out from the user's mailbox: say where before sending.
      const c = mailtoConfirmation(unsub.mailto, lastSender);
      if (!c) return;
      Alert.alert(c.foreign ? "Check the address" : "Unsubscribe by email", c.message, [
        { text: "Cancel", style: "cancel" },
        {
          text: "Send",
          style: c.foreign ? "destructive" : "default",
          onPress: () => actions.unsubscribe(ref, "mailto"),
        },
      ]);
    } else if (unsub.http) Linking.openURL(unsub.http).catch(() => {});
  }, [ref, unsub, actions, lastSender]);

  const openAttachment = useCallback((messageId: string, partId: string) => {
    if (!worker) return;
    worker
      .createLink(messageId as Id<"messages">, partId)
      .then((l) => Linking.openURL(l.url))
      .catch((err) => showToast({ id: `att:${partId}`, text: errorText(err), tone: "error" }));
  }, []);

  const hint = (id: string) => {
    const k = keysFor(getShortcut(id), keyboard.platform, keyboard.keymap)[0];
    return k ? formatKeys(k, keyboard.platform) : null;
  };
  const starred = thread?.labelKeys.includes(SYS.starred) ?? false;
  const inTrash = thread?.labelKeys.includes(SYS.trash) ?? false;
  const inInbox = thread?.labelKeys.includes(SYS.inbox) ?? false;
  const helpers = thread ? mail.actionsFor(thread.accountId) : null;

  const commands: Record<string, () => void> = {
    "nav.back": () => router.back(),
    "thread.archive": () => after(inInbox ? "archive" : "moveToInbox"),
    "thread.moveToInbox": () => after("moveToInbox"),
    "thread.trash": () => after(inTrash ? "untrash" : "trash"),
    "thread.spam": () => after("spam"),
    "thread.snooze": () => ref && openSnooze(ref, "snooze", "thread"),
    "thread.remind": () => ref && openSnooze(ref, "remind", "thread"),
    "thread.reply": () => threadId && openCompose("reply", threadId),
    "thread.replyAll": () => threadId && openCompose("replyAll", threadId),
    "thread.forward": () => threadId && openCompose("forward", threadId),
    "thread.toggleRead": () => {
      if (!ref) return;
      actions.run("markUnread", [ref], { undo: false });
      router.back();
    },
    "thread.star": () => ref && actions.run(starred ? "unstar" : "star", [ref], { undo: false }),
    "thread.label": () =>
      ref &&
      router.push({
        pathname: "/palette",
        params: { mode: "label", threadId: ref._id, accountId: ref.accountId },
      }),
    "thread.move": () =>
      ref &&
      router.push({
        pathname: "/palette",
        params: { mode: "move", threadId: ref._id, accountId: ref.accountId },
      }),
    "thread.unsubscribe": unsubscribe,
    "thread.expandAll": () => setExpandAll(true),
    "thread.showImages": () => setShowImagesNow(true),
    "ai.summarize": () => runAi("summarize"),
    "ai.instantReply": () => runAi("instant_reply"),
  };
  useShortcuts("thread", commands);

  if (!data || !thread)
    return (
      <Screen>
        <Header title="" onBack={() => router.back()} />
        <View style={{ flex: 1, alignItems: "center", justifyContent: "center" }}>
          <Spinner />
        </View>
      </Screen>
    );

  const menuItems: MenuItem[] = [
    { key: "summarize", label: "Summarize", hint: hint("ai.summarize"), onPress: commands["ai.summarize"]! },
    {
      key: "replies",
      label: "Suggest replies",
      hint: hint("ai.instantReply"),
      onPress: commands["ai.instantReply"]!,
    },
    {
      key: "replyAll",
      label: "Reply all",
      hint: hint("thread.replyAll"),
      onPress: commands["thread.replyAll"]!,
    },
    { key: "forward", label: "Forward", hint: hint("thread.forward"), onPress: commands["thread.forward"]! },
    {
      key: "star",
      label: starred ? "Unstar" : "Star",
      hint: hint("thread.star"),
      onPress: commands["thread.star"]!,
    },
    {
      key: "unread",
      label: "Mark unread",
      hint: hint("thread.toggleRead"),
      onPress: commands["thread.toggleRead"]!,
    },
    {
      key: "remind",
      label: "Remind me if no reply",
      hint: hint("thread.remind"),
      onPress: commands["thread.remind"]!,
    },
    { key: "label", label: "Label", hint: hint("thread.label"), onPress: commands["thread.label"]! },
    ...(helpers?.available("moveTo")
      ? [
          {
            key: "move",
            label: "Move to folder",
            hint: hint("thread.move"),
            onPress: commands["thread.move"]!,
          },
        ]
      : []),
    ...(!inInbox
      ? [
          {
            key: "inbox",
            label: "Move to Inbox",
            hint: hint("thread.moveToInbox"),
            onPress: commands["thread.moveToInbox"]!,
          },
        ]
      : []),
    ...(unsub
      ? [{ key: "unsub", label: "Unsubscribe", hint: hint("thread.unsubscribe"), onPress: unsubscribe }]
      : []),
    ...(remoteImages > 0 && !showImages
      ? [{ key: "images", label: "Show images", onPress: commands["thread.showImages"]! }]
      : []),
    {
      key: "spam",
      label: "Report spam",
      hint: hint("thread.spam"),
      destructive: true,
      onPress: commands["thread.spam"]!,
    },
    {
      key: "trash",
      label: inTrash ? "Restore from Trash" : "Move to Trash",
      hint: hint("thread.trash"),
      destructive: !inTrash,
      onPress: commands["thread.trash"]!,
    },
  ];

  const labels = thread.labelKeys
    .filter((k) => k.startsWith("l:"))
    .map((k) => mail.labels.find((l) => l.labelKey === k)?.name)
    .filter((n): n is string => !!n);

  return (
    <Screen edges={["top", "left", "right", "bottom"]}>
      <Header
        title=""
        onBack={() => router.back()}
        right={
          <HeaderButton
            label={starred ? "Starred" : "Star"}
            accent={starred}
            onPress={commands["thread.star"]!}
          />
        }
      />
      <View style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1], gap: t.space["0_5"] }}>
        <Txt variant="subject" accessibilityRole="header" selectable>
          {thread.subject.trim() || "(no subject)"}
        </Txt>
        <Txt variant="label" tone="muted" numberOfLines={1}>
          {[
            `${messages.length} ${messages.length === 1 ? "message" : "messages"}`,
            ...labels,
            data.account.email,
          ].join(" · ")}
        </Txt>
      </View>
      {remoteImages > 0 && !showImages ? (
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: t.space[1],
            marginHorizontal: t.space[2],
            marginBottom: t.space[1],
            padding: t.space[1],
            borderRadius: t.radius.md,
            backgroundColor: t.colors.accentWash,
          }}
        >
          <Txt variant="meta" tone="secondary" style={{ flex: 1 }}>
            {remoteImages} remote {remoteImages === 1 ? "image" : "images"} blocked
          </Txt>
          <Button compact title="Show" variant="ghost" onPress={() => setShowImagesNow(true)} />
          {lastSender ? (
            <Button
              compact
              title="Always"
              variant="ghost"
              accessibilityLabel={`Always show images from ${lastSender}`}
              onPress={() => {
                setShowImagesNow(true);
                setSenderPref({ sender: lastSender, allowRemoteImages: true }).catch(() => {});
              }}
            />
          ) : null}
        </View>
      ) : null}
      <ThreadAiPanel
        summary={ai.summary}
        replies={ai.replies}
        onReply={(body) =>
          threadId && router.push({ pathname: "/compose", params: { kind: "reply", threadId, body } })
        }
      />
      <View style={{ flex: 1 }}>
        <ThreadBody html={html} onAttachment={openAttachment} />
        {Object.values(bodies).some((b) => b.state === "error") ? (
          <Pressable
            accessibilityRole="button"
            onPress={retry}
            style={{ padding: t.space[1], alignItems: "center" }}
          >
            <Txt variant="meta" tone="accent">
              Some messages did not load. Tap to retry.
            </Txt>
          </Pressable>
        ) : null}
      </View>
      <View
        accessibilityRole="toolbar"
        style={{
          flexDirection: "row",
          justifyContent: "space-around",
          borderTopWidth: 1,
          borderTopColor: t.colors.borderSubtle,
          paddingVertical: t.space["0_5"],
        }}
      >
        <HeaderButton label={inInbox ? "Archive" : "Inbox"} onPress={commands["thread.archive"]!} />
        <HeaderButton label="Snooze" onPress={commands["thread.snooze"]!} />
        <HeaderButton label="Reply" accent onPress={commands["thread.reply"]!} />
        <HeaderButton label="More" onPress={() => setMenu(true)} />
      </View>
      <ActionMenu visible={menu} onClose={() => setMenu(false)} items={menuItems} title="Conversation" />
    </Screen>
  );
}

/** Summary and suggested replies above the conversation (results stay on this screen). */
function ThreadAiPanel({
  summary,
  replies,
  onReply,
}: {
  summary: ReturnType<typeof useThreadAi>["summary"];
  replies: ReturnType<typeof useThreadAi>["replies"];
  onReply: (body: string) => void;
}) {
  const t = useTheme();
  if (summary.state === "idle" && replies.state === "idle") return null;
  return (
    <View
      accessibilityLabel="AI"
      style={{
        marginHorizontal: t.space[2],
        marginBottom: t.space[1],
        padding: t.space["1_5"],
        gap: t.space[1],
        borderRadius: t.radius.lg,
        borderWidth: 1,
        borderColor: t.colors.borderSubtle,
      }}
    >
      {summary.state === "running" ? <Spinner /> : null}
      {summary.state === "error" ? (
        <Txt variant="meta" tone="accent">
          {summary.message}
        </Txt>
      ) : null}
      {summary.state === "done" ? (
        <View accessibilityLiveRegion="polite" style={{ gap: t.space["0_5"] }}>
          <Txt variant="label" tone="muted">
            SUMMARY
          </Txt>
          {summary.value.tldr ? <Txt variant="reading">{summary.value.tldr}</Txt> : null}
          {summary.value.bullets.slice(0, 5).map((b) => (
            <Txt key={b} variant="body" tone="secondary">
              {`• ${b}`}
            </Txt>
          ))}
        </View>
      ) : null}
      {replies.state === "running" ? <Spinner /> : null}
      {replies.state === "error" ? (
        <Txt variant="meta" tone="accent">
          {replies.message}
        </Txt>
      ) : null}
      {replies.state === "done" && replies.value.length > 0 ? (
        <View style={{ gap: t.space["0_5"] }}>
          <Txt variant="label" tone="muted">
            SUGGESTED REPLIES
          </Txt>
          {replies.value.slice(0, 3).map((r) => (
            <Button
              key={r.label}
              compact
              variant="ghost"
              title={r.label}
              accessibilityLabel={`Reply: ${r.label}`}
              onPress={() => onReply(r.body)}
            />
          ))}
        </View>
      ) : null}
    </View>
  );
}
