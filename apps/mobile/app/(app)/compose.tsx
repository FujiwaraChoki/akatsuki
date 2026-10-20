// Compose, reply, forward (§7.3, §10.1–10.3). Autosave every 2 s to the provider draft through the worker; the local
// copy in SQLite lets Undo reopen the composer. Send closes at once with "Sending… Undo"; Send later schedules.
// Writing help (§12): Write with AI and Rewrite stream into the body with Undo; Dictate records and transcribes
// with the user's dictation route; snippets insert from a picker or by typing `;shortcut` (D-295). Every AI
// surface shows where its data goes.

import { AiFeatureError, composeVars } from "@akatsuki/ai";
import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { applySnippet, snippetForTrigger, snippetTrigger } from "@akatsuki/mail-model/snippets";
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { WireError } from "@akatsuki/wire";
import { useMutation, useQuery } from "convex/react";
import { router, useLocalSearchParams, usePreventRemove } from "expo-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ActionMenu } from "../../src/components/ActionMenu";
import { ComposeAiSheet } from "../../src/components/ComposeAiSheet";
import { DictationSheet } from "../../src/components/DictationSheet";
import { HeaderButton } from "../../src/components/Header";
import { SheetModal } from "../../src/components/SheetModal";
import { type SnippetRow, SnippetSheet } from "../../src/components/SnippetSheet";
import { TimePicker } from "../../src/components/TimePicker";
import { Button, Chip, NO_OUTLINE, Screen, Spinner, Txt } from "../../src/components/ui";
import { aiStatusText, streamTextFeature, useAiEnv } from "../../src/lib/ai";
import {
  type PickedFile,
  pendingAttachment,
  pickDocuments,
  pickPhotos,
  uploadPicked,
} from "../../src/lib/attachments";
import { cachedBody } from "../../src/lib/bodies";
import { convex, requireWorker } from "../../src/lib/clients";
import { kvDelete, kvGet } from "../../src/lib/db";
import { getDeviceId } from "../../src/lib/device";
import { errorText } from "../../src/lib/errors";
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
import {
  type AiEdit,
  bodyWithAi,
  type RewriteStyle,
  rewriteEdit,
  rewriteRange,
  writeEdit,
} from "../../src/vm/composeAi";
import { insertTranscript } from "../../src/vm/dictation";
import { displayName, fileSize, formatAddress } from "../../src/vm/format";
import { sendLaterPresets } from "../../src/vm/snooze";
import { threadForAi } from "../../src/vm/threadAi";

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
        paddingVertical: 6,
        minHeight: 48,
        borderBottomWidth: StyleSheet.hairlineWidth,
        borderBottomColor: t.colors.borderSubtle,
      }}
    >
      <Txt variant="body" tone="muted" style={{ width: 52 }}>
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
        selectionColor={t.colors.dawn}
        cursorColor={t.colors.dawn}
        style={[
          t.text.reading,
          { flexGrow: 1, minWidth: 120, minHeight: 36, color: t.colors.text },
          NO_OUTLINE,
        ]}
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
  const [sheet, setSheet] = useState<null | "ai" | "snippets" | "dictate">(null);
  const [aiRun, setAiRun] = useState<{ edit: AiEdit; controller: AbortController } | null>(null);
  const [aiUndo, setAiUndo] = useState<string | null>(null);
  const [forcedSelection, setForcedSelection] = useState<{ start: number; end: number } | null>(null);
  const selection = useRef<{ start: number; end: number }>({ start: 0, end: 0 });
  const [cursorAt, setCursorAt] = useState(0);
  const ai = useAiEnv();
  const snippets = useQuery(api.snippets.list, mail.entitled ? {} : "skip") as SnippetRow[] | undefined;
  const markSnippetUsed = useMutation(api.snippets.markUsed);
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

  // A new message opened before the identities query answered (a cold start or a deep link) gets its From once
  // they arrive.
  useEffect(() => {
    const id = mail.defaultIdentity;
    if (!id || !state || state.kind !== "new" || state.accountId) return;
    setState((s) => (s.accountId ? s : { ...s, accountId: id.accountId as string, identityEmail: id.email }));
  }, [mail.defaultIdentity, state, setState]);

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

  /** Set before the composer closes itself, so the swipe-down prompt below stays out of the way. */
  const leaving = useRef(false);

  const keepDraft = useCallback(() => {
    const s = stateRef.current;
    if (s && !isBlank(s)) {
      save().catch(() => {});
      showToast({ id: `draft:${s.clientDraftId}`, text: "Draft saved" });
    }
  }, [save]);

  const dropDraft = useCallback(() => {
    const s = stateRef.current;
    if (!s) return;
    kvDelete(composeKey(s.clientDraftId)).catch(() => {});
    if (s.baseVersion > 0)
      discardDraft({ clientOpId: newClientId(), clientDraftId: s.clientDraftId }).catch(() => {});
    showToast({ id: `draft:${s.clientDraftId}`, text: "Draft discarded" });
  }, [discardDraft]);

  /** Closes the composer without the prompt (set once the prevention hook below exists). */
  const leaveRef = useRef<() => void>(() => router.back());

  const close = useCallback(() => {
    keepDraft();
    leaveRef.current();
  }, [keepDraft]);

  const discard = useCallback(() => {
    leaveRef.current();
    dropDraft();
  }, [dropDraft]);

  // Swiping the sheet down (iOS) or Back (Android) with something written asks what to do with the draft.
  const dirty = Platform.OS !== "web" && !!state && !isBlank(state);
  const disablePrevention = usePreventRemove(dirty, ({ repeat }) => {
    if (leaving.current) return repeat();
    Alert.alert("Keep this draft?", "Kept drafts stay in Drafts on every device.", [
      {
        text: "Discard",
        style: "destructive",
        onPress: () => {
          leaving.current = true;
          dropDraft();
          repeat();
        },
      },
      {
        text: "Keep draft",
        onPress: () => {
          leaving.current = true;
          keepDraft();
          repeat();
        },
      },
    ]);
  });
  const leave = useCallback(() => {
    leaving.current = true;
    disablePrevention();
    router.back();
  }, [disablePrevention]);
  leaveRef.current = leave;

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
        leaveRef.current();
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

  // ───────────── snippets (D-295) ─────────────
  const insertSnippet = useCallback(
    (snippet: SnippetRow, range?: { start: number; end: number }) => {
      const cur = stateRef.current;
      if (!cur) return;
      const at = range ?? selection.current;
      const r = applySnippet(snippet, cur, at, { now: Date.now(), timeZone: mail.timeZone });
      setState((s) => ({ ...s, text: r.text, ...(r.subject ? { subject: r.subject } : {}) }));
      selection.current = { start: r.cursor, end: r.cursor };
      setCursorAt(r.cursor);
      setForcedSelection({ start: r.cursor, end: r.cursor });
      setFocus("body");
      markSnippetUsed({ snippetId: snippet._id as Id<"snippets"> }).catch(() => {});
      if (r.missing.length > 0)
        showToast({
          id: "snippet-vars",
          text: `Fill in ${r.missing.map((v) => `{{${v}}}`).join(", ")}.`,
        });
    },
    [mail.timeZone, setState, markSnippetUsed],
  );

  // ───────────── write with AI, rewrite (§12.1) ─────────────
  const aiProblem = useCallback((problem: string) => {
    showToast({
      id: "compose-ai",
      text: problem,
      tone: "error",
      actions: [{ label: "Open AI settings", run: () => router.push("/settings/ai") }],
    });
  }, []);

  const runAiEdit = useCallback(
    async (
      feature: "compose" | "rewrite",
      edit: AiEdit,
      vars: () => Promise<Record<string, unknown>>,
    ): Promise<void> => {
      let status: Awaited<ReturnType<typeof ai.check>>;
      try {
        status = await ai.check(feature);
      } catch (e) {
        aiProblem(errorText(e));
        return;
      }
      if (status !== "ready") {
        aiProblem(aiStatusText(status));
        return;
      }
      aiRun?.controller.abort();
      const controller = new AbortController();
      setAiRun({ edit, controller });
      setAiUndo(null);
      try {
        const ctx = await ai.context(controller.signal);
        const final = await streamTextFeature(feature, await vars(), ctx, (out) => {
          if (!controller.signal.aborted) setState((s) => ({ ...s, text: bodyWithAi(edit, out) }));
        });
        if (controller.signal.aborted) return;
        const body = bodyWithAi(edit, final.trim() ? final.replace(/\s+$/, "") : final);
        setState((s) => ({ ...s, text: body }));
        setAiUndo(edit.before);
      } catch (e) {
        if (controller.signal.aborted) return;
        setState((s) => ({ ...s, text: edit.before }));
        aiProblem(e instanceof AiFeatureError ? e.message : errorText(e));
      } finally {
        setAiRun((cur) => (cur?.controller === controller ? null : cur));
      }
    },
    [ai, aiRun, aiProblem, setState],
  );

  /** The conversation a reply answers, as a transcript from bodies already on this phone (else snippets). */
  const replyTranscript = useCallback(async () => {
    const s = stateRef.current;
    if (!s?.threadId || !convex) return null;
    const detail = await convex
      .query(api.mail.getThread, { threadId: s.threadId as Id<"threads"> })
      .catch(() => null);
    if (!detail) return null;
    const msgs = detail.messages.filter((m) => !m.isDraft);
    const persist = mail.prefs?.cacheBodiesOnDevice ?? true;
    const bodies = await Promise.all(msgs.map((m) => cachedBody(m._id as string, persist).catch(() => null)));
    return threadForAi(
      msgs.map((m) => ({ from: m.from ?? null, to: m.to, cc: m.cc, date: m.date, snippet: m.snippet })),
      (i) => bodies[i] ?? null,
      mail.timeZone,
    );
  }, [mail.prefs?.cacheBodiesOnDevice, mail.timeZone]);

  const write = useCallback(
    (instruction: string) => {
      const s = stateRef.current;
      if (!s) return;
      setSheet(null);
      void runAiEdit("compose", writeEdit(s.text), async () =>
        composeVars({
          instruction,
          userName: ai.userName,
          thread: await replyTranscript(),
          draft: s.text.trim() ? s.text : null,
          voice: ai.voice,
        }),
      );
    },
    [runAiEdit, ai.userName, ai.voice, replyTranscript],
  );

  const rewrite = useCallback(
    (style: RewriteStyle) => {
      const s = stateRef.current;
      if (!s) return;
      const range = rewriteRange(s.text, selection.current);
      setSheet(null);
      if (!range) return;
      void runAiEdit("rewrite", rewriteEdit(s.text, range), async () => ({
        text: range.text,
        style,
        voice: ai.voice,
      }));
    },
    [runAiEdit, ai.voice],
  );

  const stopAi = useCallback(() => {
    if (!aiRun) return;
    aiRun.controller.abort();
    setState((s) => ({ ...s, text: aiRun.edit.before }));
    setAiRun(null);
  }, [aiRun, setState]);

  const undoAi = useCallback(() => {
    if (aiUndo === null) return;
    const before = aiUndo;
    setState((s) => ({ ...s, text: before }));
    setAiUndo(null);
  }, [aiUndo, setState]);

  // ───────────── dictation (§12.1) ─────────────
  const insertDictation = useCallback(
    (text: string) => {
      const cur = stateRef.current;
      if (!cur) return;
      const r = insertTranscript(cur.text, selection.current, text);
      setState((s) => ({ ...s, text: r.text }));
      selection.current = { start: r.cursor, end: r.cursor };
      setCursorAt(r.cursor);
      setForcedSelection({ start: r.cursor, end: r.cursor });
      setFocus("body");
    },
    [setState],
  );

  useShortcuts("compose", {
    "compose.snippet": () => setSheet("snippets"),
    "compose.aiWrite": () => setSheet("ai"),
    "compose.rewrite": () => setSheet("ai"),
    "compose.dictate": () => setSheet("dictate"),
    "compose.attach": () => setAttachMenu(true),
    "compose.send": () => send(),
    "compose.sendLater": () => setLater(true),
    "compose.discard": discard,
    "compose.close": () => {
      if (aiRun) stopAi();
      else if (sheet) setSheet(null);
      else close();
    },
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

  // `;shortcut` before the cursor names a snippet: offer it above the keyboard (Tab on a hardware keyboard).
  const typed = focus === "body" && !aiRun ? snippetTrigger(state.text, cursorAt) : null;
  const typedSnippet = snippetForTrigger(snippets ?? [], typed);
  const trigger = typed && typedSnippet ? { snippet: typedSnippet, range: typed } : null;

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
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            minHeight: 52,
            paddingHorizontal: t.space[1],
            borderBottomWidth: StyleSheet.hairlineWidth,
            borderBottomColor: t.colors.borderSubtle,
          }}
        >
          <View style={{ width: 96, alignItems: "flex-start" }}>
            <HeaderButton label="Close" onPress={close} hint="Saves the draft" />
          </View>
          <View style={{ flex: 1, alignItems: "center" }}>
            <Txt variant="title" accessibilityRole="header" numberOfLines={1}>
              {title}
            </Txt>
            {saving || state.baseVersion > 0 ? (
              <Txt variant="caption" tone="muted">
                {saving ? "Saving…" : "Draft saved"}
              </Txt>
            ) : null}
          </View>
          <View style={{ width: 96, alignItems: "flex-end", paddingRight: t.space[1] }}>
            <Button
              title="Send"
              variant="primary"
              compact
              hint={hint("compose.send")}
              onPress={() => send()}
              disabled={problems.errors.length > 0}
              style={{ minHeight: 32 }}
            />
          </View>
        </View>
        <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ flexGrow: 1 }}>
          <RecipientField
            field="to"
            label="To"
            state={state}
            setState={setState}
            onFocus={() => setFocus("to")}
            autoFocus={focus === "to"}
          />
          {state.showCcBcc || !state.identityEmail ? (
            <>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`From ${state.identityEmail ?? "no address"}. Change`}
                onPress={() => setFromMenu(true)}
                disabled={accountIdentities.length < 2}
                style={{
                  flexDirection: "row",
                  alignItems: "center",
                  paddingHorizontal: t.space[2],
                  minHeight: 48,
                  borderBottomWidth: StyleSheet.hairlineWidth,
                  borderBottomColor: t.colors.borderSubtle,
                }}
              >
                <Txt variant="body" tone="muted" style={{ width: 52 }}>
                  From
                </Txt>
                <Txt variant="body" tone={state.identityEmail ? "text" : "error"} style={{ flex: 1 }}>
                  {state.identityEmail ?? "Connect a mailbox to send"}
                </Txt>
              </Pressable>
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
              accessibilityLabel="Cc, Bcc, From"
              onPress={() => setState((s) => ({ ...s, showCcBcc: true }))}
              style={{
                flexDirection: "row",
                alignItems: "center",
                paddingHorizontal: t.space[2],
                minHeight: 48,
                borderBottomWidth: StyleSheet.hairlineWidth,
                borderBottomColor: t.colors.borderSubtle,
              }}
            >
              <Txt variant="body" tone="muted" numberOfLines={1} style={{ flex: 1 }}>
                {accountIdentities.length > 1 ? `Cc, Bcc, From: ${state.identityEmail}` : "Cc, Bcc"}
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
            selectionColor={t.colors.dawn}
            cursorColor={t.colors.dawn}
            style={[
              t.text.bodyStrong,
              {
                fontSize: 17,
                color: t.colors.text,
                paddingHorizontal: t.space[2],
                minHeight: 48,
                borderBottomWidth: StyleSheet.hairlineWidth,
                borderBottomColor: t.colors.borderSubtle,
              },
              NO_OUTLINE,
            ]}
          />
          <TextInput
            accessibilityLabel="Message"
            value={state.text}
            onChangeText={(v) => {
              setAiUndo(null);
              setState((s) => ({ ...s, text: v }));
            }}
            onFocus={() => setFocus("body")}
            onSelectionChange={(e) => {
              selection.current = e.nativeEvent.selection;
              setCursorAt(e.nativeEvent.selection.end);
              if (forcedSelection) setForcedSelection(null);
            }}
            {...(forcedSelection ? { selection: forcedSelection } : {})}
            editable={!aiRun}
            autoFocus={focus === "body"}
            multiline
            textAlignVertical="top"
            placeholder={
              state.forwardOfMessageId
                ? "Add a note (the original message is attached)"
                : "Write your message"
            }
            placeholderTextColor={t.colors.muted}
            selectionColor={t.colors.dawn}
            cursorColor={t.colors.dawn}
            style={[
              t.text.reading,
              { flex: 1, minHeight: 240, color: t.colors.text, padding: t.space[2] },
              NO_OUTLINE,
            ]}
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
                    borderRadius: t.radius.md,
                    borderWidth: 1,
                    borderColor: a.status === "error" ? t.colors.accentText : t.colors.borderSubtle,
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
        {aiRun ? (
          <View
            accessibilityLiveRegion="polite"
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: t.space[1],
              paddingHorizontal: t.space[2],
              paddingVertical: t.space["0_5"],
              backgroundColor: t.colors.surface,
            }}
          >
            <Spinner />
            <Txt variant="meta" tone="secondary" style={{ flex: 1 }} numberOfLines={2}>
              {aiRun.edit.kind === "write" ? "Writing…" : "Rewriting…"}{" "}
              {ai.egress(aiRun.edit.kind === "write" ? "compose" : "rewrite") ?? ""}
            </Txt>
            <Button title="Stop" variant="ghost" compact onPress={stopAi} />
          </View>
        ) : aiUndo !== null ? (
          <View
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: t.space[1],
              paddingHorizontal: t.space[2],
              paddingVertical: t.space["0_5"],
              backgroundColor: t.colors.surface,
            }}
          >
            <Txt variant="meta" tone="secondary" style={{ flex: 1 }}>
              Written with AI. Read it before you send.
            </Txt>
            <Button title="Undo" variant="ghost" compact onPress={undoAi} />
            <Button title="Keep" variant="ghost" compact onPress={() => setAiUndo(null)} />
          </View>
        ) : trigger ? (
          <View
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: t.space[1],
              paddingHorizontal: t.space[2],
              paddingVertical: t.space["0_5"],
            }}
          >
            <Chip
              label={`Insert “${trigger.snippet.name}” (;${trigger.snippet.shortcut})`}
              onPress={() => insertSnippet(trigger.snippet, trigger.range)}
              selected
            />
          </View>
        ) : null}
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            minHeight: 44,
            paddingHorizontal: t.space[1],
            paddingBottom: insets.bottom,
            borderTopWidth: StyleSheet.hairlineWidth,
            borderTopColor: t.colors.borderSubtle,
            backgroundColor: t.colors.surface,
          }}
        >
          <HeaderButton label="Attach" icon="paperclip" onPress={() => setAttachMenu(true)} />
          <HeaderButton label="Send later" icon="clock" onPress={() => setLater(true)} />
          <HeaderButton label="Snippets" icon="bookmark" onPress={() => !aiRun && setSheet("snippets")} />
          <HeaderButton label="Dictate" icon="mic" onPress={() => !aiRun && setSheet("dictate")} />
          <Button
            title="Write with AI"
            compact
            hint={hint("compose.aiWrite")}
            disabled={!!aiRun}
            onPress={() => setSheet("ai")}
            style={{ minHeight: 32, marginLeft: t.space["0_5"] }}
          />
          <View style={{ flex: 1 }} />
          <HeaderButton label="Discard" icon="trash" onPress={discard} />
        </View>
      </KeyboardAvoidingView>

      <SheetModal visible={later} transparent onRequestClose={() => setLater(false)}>
        <Pressable
          accessibilityLabel="Close"
          style={{ flex: 1, backgroundColor: t.colors.scrim }}
          onPress={() => setLater(false)}
        />
        <View
          style={{
            backgroundColor: t.colors.background,
            borderTopLeftRadius: t.radius.xl,
            borderTopRightRadius: t.radius.xl,
            paddingTop: t.space[2],
            paddingBottom: insets.bottom + t.space[2],
          }}
        >
          <Txt variant="title" style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}>
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
      </SheetModal>

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

      {sheet === "ai" ? (
        <ComposeAiSheet
          isReply={state.kind === "reply" || state.kind === "replyAll"}
          canRewrite={rewriteRange(state.text, selection.current) !== null}
          rewriteScope={
            selection.current.end > selection.current.start ? "the selected text" : "your message"
          }
          egress={{ compose: ai.egress("compose"), rewrite: ai.egress("rewrite") }}
          onWrite={write}
          onRewrite={rewrite}
          onClose={() => setSheet(null)}
        />
      ) : null}
      {sheet === "snippets" ? (
        <SnippetSheet
          snippets={snippets}
          onPick={(sn) => {
            setSheet(null);
            insertSnippet(sn);
          }}
          onClose={() => setSheet(null)}
        />
      ) : null}
      {sheet === "dictate" ? (
        <DictationSheet ai={ai} onText={insertDictation} onClose={() => setSheet(null)} />
      ) : null}

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
