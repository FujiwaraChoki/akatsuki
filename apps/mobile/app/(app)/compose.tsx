// Compose, reply, forward (§7.3, §10.1–10.3). Autosave every 2 s to the provider draft through the worker; the local
// copy in SQLite lets Undo reopen the composer. Send closes at once with "Sending… Undo"; Send later schedules.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { WireError } from "@akatsuki/wire";
import { useMutation, useQuery } from "convex/react";
import { router, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ActionMenu } from "../../src/components/ActionMenu";
import { Header, HeaderButton } from "../../src/components/Header";
import { TimePicker } from "../../src/components/TimePicker";
import { Button, Chip, Screen, Spinner, Txt } from "../../src/components/ui";
import {
  type PickedFile,
  pendingAttachment,
  pickDocuments,
  pickPhotos,
  uploadPicked,
} from "../../src/lib/attachments";
import { requireWorker } from "../../src/lib/clients";
import { kvDelete, kvGet } from "../../src/lib/db";
import { getDeviceId } from "../../src/lib/device";
import { newClientId } from "../../src/lib/ids";
import { useKeyboard, useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { composeKey, saveLocalCompose, startSend, toDraftContent } from "../../src/state/sendQueue";
import { showToast } from "../../src/state/toasts";
import { useTheme } from "../../src/theme/ThemeProvider";
import {
  AUTOSAVE_MS,
  addAttachment,
  attachmentsOf,
  type ComposeState,
  commitPending,
  composeFromThread,
  draftSignature,
  emptyCompose,
  isBlank,
  removeAttachment,
  removeRecipient,
  updateAttachment,
  validateCompose,
} from "../../src/vm/compose";
import { displayName, fileSize, formatAddress } from "../../src/vm/format";
import { sendLaterPresets } from "../../src/vm/snooze";

type Field = "to" | "cc" | "bcc";

function RecipientField({
  field,
  label,
  state,
  setState,
  onFocus,
  autoFocus,
}: {
  field: Field;
  label: string;
  state: ComposeState;
  setState: (f: (s: ComposeState) => ComposeState) => void;
  onFocus: () => void;
  autoFocus?: boolean;
}) {
  const t = useTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        flexWrap: "wrap",
        alignItems: "center",
        gap: t.space["0_5"],
        paddingHorizontal: t.space[2],
        paddingVertical: t.space["0_5"],
        borderBottomWidth: 1,
        borderBottomColor: t.colors.borderSubtle,
      }}
    >
      <Txt variant="label" tone="muted" style={{ width: 52 }}>
        {label}
      </Txt>
      {state[field].map((a) => (
        <Chip
          key={a.email}
          label={displayName(a)}
          onRemove={() => setState((s) => removeRecipient(s, field, a.email))}
        />
      ))}
      <TextInput
        accessibilityLabel={`${label} recipients`}
        value={state.pending[field]}
        onChangeText={(v) => {
          setState((s) => {
            const next = { ...s, pending: { ...s.pending, [field]: v } };
            return /[,;\n]\s*$/.test(v) ? commitPending(next, field) : next;
          });
        }}
        onBlur={() => setState((s) => commitPending(s, field))}
        onSubmitEditing={() => setState((s) => commitPending(s, field))}
        onFocus={onFocus}
        autoFocus={autoFocus}
        autoCapitalize="none"
        autoCorrect={false}
        keyboardType="email-address"
        textContentType="emailAddress"
        submitBehavior="submit"
        placeholder={state[field].length === 0 ? "name@example.com" : undefined}
        placeholderTextColor={t.colors.muted}
        selectionColor={t.colors.accent}
        style={[t.text.reading, { flexGrow: 1, minWidth: 120, minHeight: 40, color: t.colors.text }]}
      />
    </View>
  );
}

export default function Compose() {
  const t = useTheme();
  const mail = useMail();
  const keyboard = useKeyboard();
  const insets = useSafeAreaInsets();
  const params = useLocalSearchParams<{
    kind?: string;
    threadId?: string;
    restore?: string;
    body?: string;
  }>();
  const kind =
    params.kind === "reply" || params.kind === "replyAll" || params.kind === "forward" ? params.kind : null;
  const thread = useQuery(
    api.mail.getThread,
    kind && params.threadId ? { threadId: params.threadId as Id<"threads"> } : "skip",
  );
  const discardDraft = useMutation(api.drafts.discard);
  const [state, setStateRaw] = useState<ComposeState | null>(null);
  const [focus, setFocus] = useState<Field | "subject" | "body">("to");
  const [later, setLater] = useState(false);
  const [fromMenu, setFromMenu] = useState(false);
  const [attachMenu, setAttachMenu] = useState(false);
  const [saving, setSaving] = useState(false);
  const deviceId = useRef<string | null>(null);
  const savedSig = useRef<string>("");
  const stateRef = useRef<ComposeState | null>(null);

  const setState = useCallback((f: (s: ComposeState) => ComposeState) => {
    setStateRaw((s) => {
      const next = s ? f(s) : s;
      stateRef.current = next;
      return next;
    });
  }, []);

  useEffect(() => {
    getDeviceId().then((id) => {
      deviceId.current = id;
    });
  }, []);

  // Initial state: a restored local copy, a reply/forward built from the thread, or a new message.
  useEffect(() => {
    if (state) return;
    if (params.restore) {
      kvGet<ComposeState>(composeKey(params.restore)).then((saved) => {
        const s =
          saved ??
          emptyCompose(
            newClientId(),
            mail.defaultIdentity?.accountId ?? null,
            mail.defaultIdentity?.email ?? null,
          );
        stateRef.current = s;
        savedSig.current = draftSignature(s);
        setStateRaw(s);
      });
      return;
    }
    if (kind) {
      if (!thread) return;
      const identity =
        mail.identities.find((i) => i.accountId === thread.account._id && i.isDefault) ??
        mail.identities.find((i) => i.accountId === thread.account._id);
      const s = composeFromThread(kind, {
        clientDraftId: newClientId(),
        threadId: params.threadId!,
        subject: thread.thread.subject,
        messages: thread.messages,
        selfEmails: mail.selfEmails,
        identityEmail: identity?.email ?? thread.account.email,
      });
      // A suggested reply (thread AI, D-408) starts the body; the user edits it before sending.
      if (params.body && kind !== "forward")
        s.text = `${params.body.slice(0, 20_000)}${s.text ? `\n\n${s.text}` : ""}`;
      stateRef.current = s;
      setStateRaw(s);
      setFocus(kind === "forward" ? "to" : "body");
      return;
    }
    const s = emptyCompose(
      newClientId(),
      mail.defaultIdentity?.accountId ?? null,
      mail.defaultIdentity?.email ?? null,
    );
    stateRef.current = s;
    setStateRaw(s);
  }, [
    state,
    params.restore,
    params.threadId,
    params.body,
    kind,
    thread,
    mail.defaultIdentity,
    mail.identities,
    mail.selfEmails,
  ]);

  const signature = state ? draftSignature(state) : "";

  const save = useCallback(async (): Promise<void> => {
    const s = stateRef.current;
    if (!s || isBlank(s) || !s.accountId || !s.identityEmail || !deviceId.current) return;
    const sig = draftSignature(s);
    await saveLocalCompose(s);
    if (sig === savedSig.current) return;
    setSaving(true);
    try {
      const saved = await requireWorker().putDraft(s.clientDraftId, {
        baseVersion: s.baseVersion,
        draft: toDraftContent(s),
        deviceId: deviceId.current,
      });
      savedSig.current = sig;
      setState((cur) => ({ ...cur, baseVersion: saved.version }));
    } catch (err) {
      // Another device saved this draft meanwhile: keep this text and write over it next time (D-336).
      if (
        err instanceof WireError &&
        err.code === "draft_version_conflict" &&
        typeof err.version === "number"
      ) {
        const version = err.version;
        setState((cur) => ({ ...cur, baseVersion: version }));
      }
    } finally {
      setSaving(false);
    }
  }, [setState]);

  // Autosave (§10.1): 2 s after the last change.
  useEffect(() => {
    if (!signature) return;
    const id = setTimeout(() => {
      save().catch(() => {});
    }, AUTOSAVE_MS);
    return () => clearTimeout(id);
  }, [signature, save]);

  const contactQuery =
    state && (focus === "to" || focus === "cc" || focus === "bcc") ? state.pending[focus].trim() : "";
  const contacts = useQuery(
    api.contacts.search,
    contactQuery.length >= 1 && mail.entitled ? { q: contactQuery, limit: 6 } : "skip",
  );

  const problems = useMemo(() => (state ? validateCompose(state) : { errors: [], warnings: [] }), [state]);

  const close = useCallback(() => {
    const s = stateRef.current;
    if (s && !isBlank(s)) {
      save().catch(() => {});
      showToast({ id: `draft:${s.clientDraftId}`, text: "Draft saved" });
    }
    router.back();
  }, [save]);

  const discard = useCallback(() => {
    const s = stateRef.current;
    router.back();
    if (!s) return;
    kvDelete(composeKey(s.clientDraftId)).catch(() => {});
    if (s.baseVersion > 0)
      discardDraft({ clientOpId: newClientId(), clientDraftId: s.clientDraftId }).catch(() => {});
    showToast({ id: `draft:${s.clientDraftId}`, text: "Draft discarded" });
  }, [discardDraft]);

  const send = useCallback(
    (sendAt?: number) => {
      const s = stateRef.current;
      if (!s || !deviceId.current) return;
      const p = validateCompose(s);
      if (p.errors.length > 0) {
        showToast({ id: "compose-error", text: p.errors[0]!, tone: "error" });
        return;
      }
      const go = () => {
        let committed = s;
        for (const f of ["to", "cc", "bcc"] as const) committed = commitPending(committed, f);
        startSend(committed, {
          deviceId: deviceId.current!,
          undoSeconds: mail.prefs?.undoSendSeconds ?? 10,
          timeZone: mail.timeZone,
          ...(sendAt !== undefined ? { sendAt } : {}),
        });
        router.back();
      };
      if (p.warnings.includes("The subject is empty."))
        Alert.alert("Send without a subject?", undefined, [
          { text: "Cancel", style: "cancel" },
          { text: "Send", onPress: go },
        ]);
      else go();
    },
    [mail.prefs?.undoSendSeconds, mail.timeZone],
  );

  // Attachments (§10.4): each picked file shows at once as "Uploading…" and joins the draft once the worker has it.
  const attach = useCallback(
    async (pick: () => Promise<PickedFile[]>) => {
      let picked: PickedFile[];
      try {
        picked = await pick();
      } catch {
        showToast({ id: "attach", text: "Could not open the picker.", tone: "error" });
        return;
      }
      for (const f of picked) {
        const a = pendingAttachment(f);
        setState((s) => addAttachment(s, a));
        if (a.status !== "uploading") continue;
        uploadPicked(f)
          .then((uploadId) => setState((s) => updateAttachment(s, a.key, { status: "ready", uploadId })))
          .catch((err: unknown) =>
            setState((s) =>
              updateAttachment(s, a.key, {
                status: "error",
                error: err instanceof WireError ? err.message : "Upload failed.",
              }),
            ),
          );
      }
    },
    [setState],
  );

  useShortcuts("compose", {
    "compose.attach": () => setAttachMenu(true),
    "compose.send": () => send(),
    "compose.sendLater": () => setLater(true),
    "compose.discard": discard,
    "compose.close": close,
    "compose.cc": () => setState((s) => ({ ...s, showCcBcc: true })),
    "compose.bcc": () => setState((s) => ({ ...s, showCcBcc: true })),
  });

  const hint = (id: string) => {
    const k = keysFor(getShortcut(id), keyboard.platform, keyboard.keymap)[0];
    return k && Platform.OS === "ios" && Platform.isPad ? formatKeys(k, keyboard.platform) : null;
  };

  if (!state)
    return (
      <Screen>
        <View style={{ flex: 1, alignItems: "center", justifyContent: "center" }}>
          <Spinner />
        </View>
      </Screen>
    );

  const title =
    state.kind === "reply" || state.kind === "replyAll"
      ? "Reply"
      : state.kind === "forward"
        ? "Forward"
        : "New message";
  // A reply or forward stays in its thread's account; a new message may go out from any identity.
  const accountIdentities =
    state.kind === "new" ? mail.identities : mail.identities.filter((i) => i.accountId === state.accountId);

  return (
    <Screen edges={["top", "left", "right"]}>
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={{ flex: 1 }}>
        <Header
          title={title}
          subtitle={saving ? "Saving…" : state.baseVersion > 0 ? "Draft saved" : null}
          right={
            <>
              <HeaderButton label="Close" onPress={close} hint="Saves the draft" />
              <Button
                title="Send"
                variant="primary"
                compact
                hint={hint("compose.send")}
                onPress={() => send()}
                disabled={problems.errors.length > 0}
              />
            </>
          }
        />
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ flexGrow: 1 }}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`From ${state.identityEmail ?? "no address"}. Change`}
            onPress={() => setFromMenu(true)}
            disabled={accountIdentities.length < 2}
            style={{
              flexDirection: "row",
              alignItems: "center",
              paddingHorizontal: t.space[2],
              minHeight: 44,
              borderBottomWidth: 1,
              borderBottomColor: t.colors.borderSubtle,
            }}
          >
            <Txt variant="label" tone="muted" style={{ width: 52 }}>
              From
            </Txt>
            <Txt variant="body" tone={state.identityEmail ? "text" : "error"} style={{ flex: 1 }}>
              {state.identityEmail ?? "Connect a mailbox to send"}
            </Txt>
          </Pressable>
          <RecipientField
            field="to"
            label="To"
            state={state}
            setState={setState}
            onFocus={() => setFocus("to")}
            autoFocus={focus === "to"}
          />
          {state.showCcBcc ? (
            <>
              <RecipientField
                field="cc"
                label="Cc"
                state={state}
                setState={setState}
                onFocus={() => setFocus("cc")}
              />
              <RecipientField
                field="bcc"
                label="Bcc"
                state={state}
                setState={setState}
                onFocus={() => setFocus("bcc")}
              />
            </>
          ) : (
            <Pressable
              accessibilityRole="button"
              onPress={() => setState((s) => ({ ...s, showCcBcc: true }))}
              style={{ paddingHorizontal: t.space[2], paddingVertical: t.space[1] }}
            >
              <Txt variant="label" tone="accent">
                Cc / Bcc
              </Txt>
            </Pressable>
          )}
          {contacts && contacts.length > 0 && (focus === "to" || focus === "cc" || focus === "bcc") ? (
            <View style={{ borderBottomWidth: 1, borderBottomColor: t.colors.borderSubtle }}>
              {contacts.map((c) => (
                <Pressable
                  key={c.email}
                  accessibilityRole="button"
                  onPress={() =>
                    setState((s) => ({
                      ...s,
                      [focus]: [...s[focus as Field].filter((a) => a.email !== c.email), c],
                      pending: { ...s.pending, [focus]: "" },
                    }))
                  }
                  style={({ pressed }) => ({
                    paddingHorizontal: t.space[2],
                    paddingVertical: t.space[1],
                    backgroundColor: pressed ? t.colors.selected : "transparent",
                  })}
                >
                  <Txt variant="body">{c.name ?? c.email}</Txt>
                  {c.name ? (
                    <Txt variant="meta" tone="muted">
                      {c.email}
                    </Txt>
                  ) : null}
                </Pressable>
              ))}
            </View>
          ) : null}
          <TextInput
            accessibilityLabel="Subject"
            value={state.subject}
            onChangeText={(v) => setState((s) => ({ ...s, subject: v }))}
            onFocus={() => setFocus("subject")}
            placeholder="Subject"
            placeholderTextColor={t.colors.muted}
            selectionColor={t.colors.accent}
            style={[
              t.text.reading,
              {
                color: t.colors.text,
                paddingHorizontal: t.space[2],
                minHeight: 48,
                borderBottomWidth: 1,
                borderBottomColor: t.colors.borderSubtle,
              },
            ]}
          />
          <TextInput
            accessibilityLabel="Message"
            value={state.text}
            onChangeText={(v) => setState((s) => ({ ...s, text: v }))}
            onFocus={() => setFocus("body")}
            autoFocus={focus === "body"}
            multiline
            textAlignVertical="top"
            placeholder={
              state.forwardOfMessageId
                ? "Add a note (the original message is attached)"
                : "Write your message"
            }
            placeholderTextColor={t.colors.muted}
            selectionColor={t.colors.accent}
            style={[t.text.reading, { flex: 1, minHeight: 240, color: t.colors.text, padding: t.space[2] }]}
          />
          {attachmentsOf(state).length > 0 ? (
            <View
              accessibilityLabel="Attachments"
              style={{ gap: t.space["0_5"], paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}
            >
              {attachmentsOf(state).map((a) => (
                <View
                  key={a.key}
                  style={{
                    flexDirection: "row",
                    alignItems: "center",
                    gap: t.space[1],
                    minHeight: 44,
                    paddingHorizontal: t.space["1_5"],
                    borderRadius: t.radius.card,
                    borderWidth: 1,
                    borderColor: a.status === "error" ? t.colors.accent : t.colors.borderSubtle,
                  }}
                >
                  <View style={{ flex: 1 }}>
                    <Txt variant="body" numberOfLines={1}>
                      {a.filename}
                    </Txt>
                    <Txt variant="meta" tone={a.status === "error" ? "error" : "muted"}>
                      {a.status === "uploading"
                        ? "Uploading…"
                        : a.status === "error"
                          ? (a.error ?? "Upload failed.")
                          : fileSize(a.size)}
                    </Txt>
                  </View>
                  {a.status === "uploading" ? <Spinner /> : null}
                  <HeaderButton
                    label="Remove"
                    hint={`Remove ${a.filename}`}
                    onPress={() => setState((s) => removeAttachment(s, a.key))}
                  />
                </View>
              ))}
            </View>
          ) : null}
          {state.inReplyToMessageId || state.forwardOfMessageId ? (
            <Txt
              variant="meta"
              tone="muted"
              style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}
            >
              {state.forwardOfMessageId ? "Forwarding" : "Replying to"}{" "}
              {[...state.to].map(formatAddress).join(", ") || "the conversation"}. Your signature is added
              when it is sent.
            </Txt>
          ) : null}
        </ScrollView>
        <View
          style={{
            flexDirection: "row",
            gap: t.space[1],
            paddingHorizontal: t.space[2],
            paddingTop: t.space[1],
            paddingBottom: insets.bottom + t.space[1],
            borderTopWidth: 1,
            borderTopColor: t.colors.borderSubtle,
          }}
        >
          <Button title="Attach" compact hint={hint("compose.attach")} onPress={() => setAttachMenu(true)} />
          <Button
            title="Send later"
            compact
            hint={hint("compose.sendLater")}
            onPress={() => setLater(true)}
          />
          <View style={{ flex: 1 }} />
          <Button title="Discard" variant="ghost" compact onPress={discard} />
        </View>
      </KeyboardAvoidingView>

      <Modal visible={later} transparent animationType="slide" onRequestClose={() => setLater(false)}>
        <Pressable
          accessibilityLabel="Close"
          style={{ flex: 1, backgroundColor: t.colors.scrim }}
          onPress={() => setLater(false)}
        />
        <View
          style={{
            backgroundColor: t.colors.background,
            borderTopLeftRadius: t.radius.card,
            borderTopRightRadius: t.radius.card,
            paddingTop: t.space[2],
            paddingBottom: insets.bottom + t.space[2],
          }}
        >
          <Txt variant="heading" style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}>
            Send later
          </Txt>
          <TimePicker
            presets={sendLaterPresets(Date.now(), mail.timeZone)}
            timeZone={mail.timeZone}
            confirmLabel="Schedule"
            onPick={(at) => {
              setLater(false);
              send(at);
            }}
          />
        </View>
      </Modal>

      <ActionMenu
        visible={attachMenu}
        onClose={() => setAttachMenu(false)}
        title="Attach"
        items={[
          // iOS cannot present the picker while the menu is still closing.
          { key: "files", label: "Files", onPress: () => setTimeout(() => void attach(pickDocuments), 350) },
          {
            key: "photos",
            label: "Photos and videos",
            onPress: () => setTimeout(() => void attach(pickPhotos), 350),
          },
        ]}
      />

      <ActionMenu
        visible={fromMenu}
        onClose={() => setFromMenu(false)}
        title="Send from"
        items={accountIdentities.map((i) => ({
          key: i._id as string,
          label: i.name ? `${i.name} <${i.email}>` : i.email,
          onPress: () =>
            setState((s) => ({ ...s, accountId: i.accountId as string, identityEmail: i.email })),
        }))}
      />
    </Screen>
  );
}
