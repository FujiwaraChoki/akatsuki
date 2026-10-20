// The composer: a fast overlay over the mail shell for new mail, replies, reply-all and forwards
// (ARCHITECTURE §10). Plain-text editing (D-280); autosave to the device at once and to the provider draft
// every 2 s (§10.1); send with undo, send later (§10.2–10.3); attachments through the worker's encrypted spool
// (§10.4); snippets with variables; contact autocomplete; identities and signatures; follow-up reminders;
// opt-in read receipts; AI write, rewrite and dictation with their egress labels (§12).
//
// Keys (packages/shortcuts, compose scope): Ctrl/⌘ Enter send, Ctrl/⌘ Shift L send later, Ctrl/⌘ ; snippet,
// Ctrl/⌘ Shift A attach, Ctrl/⌘ Shift C / B cc / bcc, Ctrl/⌘ J write with AI, Ctrl/⌘ Shift , discard, Escape
// close. Every one of them is also in the command palette while the composer is open.
//
// First Light (SPEC §3.4): compose takes the reader slot (the list stays usable beside it); a reply expands at the
// foot of the thread, full reader width. One anatomy: a 13/500 title, 36px hairline field rows, the body in the
// reading size with a dawn caret, then an ink Send, Send later and Write with AI. Rewrite and dictation sit in the
// Write-with-AI popover; follow-up, read receipt, snippets and discard in More.
import { type AnchorRect, Button, errorMessage, IconButton, Kbd, Spinner, useToast } from "@akatsuki/ui";
import {
  type ChangeEvent,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { dictationSupported, useDictation } from "../ai/dictation.ts";
import { EgressNote } from "../ai/Egress.tsx";
import { loadAi } from "../ai/module.ts";
import { ensureBodies, threadForAi, visibleText } from "../ai/thread.ts";
import { type UseAi, useAi, useAiStream } from "../ai/useAi.ts";
import { voiceFor } from "../ai/voice.ts";
import { useBackend, useBodies } from "../backend/context.tsx";
import type { IdentityView, MailAccountView, MessageMeta, MeView } from "../backend/types.ts";
import { IconChevronDown, IconClose, IconDots, IconPaperclip } from "../components/Icons.tsx";
import { fileSize } from "../lib/format.ts";
import { deviceId, uuidv7 } from "../lib/ids.ts";
import { defaultStore, type KvStore } from "../lib/kv.ts";
import { ListPicker, type PickerItem } from "../mail/ListPicker.tsx";
import { TimePicker } from "../mail/TimePicker.tsx";
import {
  attachmentBytes,
  type ComposeAttachment,
  type FollowUp,
  forwardBlock,
  hasContent,
  type LocalDraft,
  MAX_ATTACHMENT_BYTES,
  replyQuote,
  sendProblem,
} from "./model.ts";
import { RecipientField } from "./RecipientField.tsx";
import { DraftSaver, type SaveStatus } from "./saver.ts";
import { type SendDeps, sendDraft } from "./sendFlow.ts";
import { composer, useComposer } from "./store.ts";
import { type Addr, builtinVariables, expandVariables, htmlToText, textToHtml } from "./text.ts";

let localStore: KvStore<LocalDraft> | null = null;
/** The device's compose store (IndexedDB `drafts`, §10.1 step 4). */
export function draftStore(): KvStore<LocalDraft> {
  localStore ??= defaultStore<LocalDraft>("drafts");
  return localStore;
}
/** Test seam. */
export function setDraftStore(s: KvStore<LocalDraft> | null): void {
  localStore = s;
}

/** Handlers of the open composer, read by the compose extension's `commands()`. */
export const composerCommands: { current: Record<string, () => void> | null } = { current: null };

const MODE_TITLE: Record<LocalDraft["mode"], string> = {
  new: "New message",
  reply: "Reply",
  reply_all: "Reply all",
  forward: "Forward",
};

const STATUS_TEXT: Record<SaveStatus, string> = {
  idle: "",
  pending: "Draft",
  saving: "Saving…",
  saved: "Draft saved",
  error: "Saved on this device only",
  conflict: "Changed on another device",
};

const FOLLOW_UPS: { id: FollowUp | "off"; label: string }[] = [
  { id: "off", label: "No follow-up" },
  { id: "1d", label: "Remind me if no reply in 1 day" },
  { id: "3d", label: "Remind me if no reply in 3 days" },
  { id: "1w", label: "Remind me if no reply in 1 week" },
];

export interface ComposerProps {
  initial: LocalDraft;
  me: MeView;
  accounts: readonly MailAccountView[];
  identities: readonly IdentityView[];
}

export function Composer({ initial, me, accounts, identities }: ComposerProps) {
  const backend = useBackend();
  const loader = useBodies();
  const m = backend.useMutations();
  const { toast, dismiss } = useToast();
  const ai = useAi();
  const state = useComposer();
  const prefs = me.profile.prefs;
  const tz = me.profile.timezone;
  const [d, setD] = useState<LocalDraft>(initial);
  const [status, setStatus] = useState<SaveStatus>(initial.version > 0 ? "saved" : "idle");
  const [problem, setProblem] = useState<string | null>(null);
  const [showQuote, setShowQuote] = useState(false);
  const [picker, setPicker] = useState<null | "snippet" | "later">(null);
  const [aiBar, setAiBar] = useState<null | "write">(null);
  const [panel, setPanel] = useState<null | "rewrite" | "more">(null);
  const laterRef = useRef<HTMLButtonElement>(null);
  const [laterAnchor, setLaterAnchor] = useState<AnchorRect | null>(null);
  const [instruction, setInstruction] = useState("");
  const [aiUndo, setAiUndo] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [, setFocusTick] = useState(0);
  const rootRef = useRef<HTMLElement>(null);
  const dock = useReplyDock(d.mode, rootRef);
  const toRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const ccRef = useRef<HTMLInputElement>(null);
  const bccRef = useRef<HTMLInputElement>(null);
  // The field a command acts on takes focus once it is rendered (a palette command closes the palette in the same
  // update, so this runs after the palette gave focus back).
  const focusNext = useRef<"cc" | "bcc" | "body" | null>(null);
  useEffect(() => {
    const which = focusNext.current;
    if (!which) return;
    const el = which === "cc" ? ccRef.current : which === "bcc" ? bccRef.current : bodyRef.current;
    if (!el) return;
    focusNext.current = null;
    el.focus({ preventScroll: true });
  });
  const fileRef = useRef<HTMLInputElement>(null);
  const aiInputRef = useRef<HTMLInputElement>(null);
  const stream = useAiStream(ai);
  const aiTarget = useRef<{ kind: "write" | "rewrite"; start: number; end: number; before: string } | null>(
    null,
  );

  // One saver per composer session (the overlay remounts the composer for another draft).
  const [saver] = useState(
    () =>
      new DraftSaver(initial, {
        worker: backend.worker,
        local: draftStore(),
        deviceId: deviceId(),
        html: textToHtml,
        onChange: (s) => {
          setStatus(s.status);
          setD((cur) =>
            cur.version === s.draft.version
              ? cur
              : { ...cur, version: s.draft.version, savedAt: s.draft.savedAt },
          );
        },
      }),
  );
  useEffect(() => () => saver.dispose(), [saver]);

  // Untouched replies and forwards (r then Escape) leave nothing behind; a pre-filled body counts as content.
  const touched = useRef(initial.body.trim() !== "" || initial.version > 0);
  const edit = useCallback(
    (
      patch: Partial<LocalDraft> | ((cur: LocalDraft) => Partial<LocalDraft>),
      opts: { system?: boolean } = {},
    ) => {
      if (!opts.system) touched.current = true;
      setD((cur) => {
        const next = {
          ...cur,
          ...(typeof patch === "function" ? patch(cur) : patch),
          version: saver.current().version,
        };
        if (touched.current) saver.update(next);
        else saver.sync(next);
        composer.update(next);
        return next;
      });
      setProblem(null);
    },
    [saver],
  );

  // Focus: recipients for new mail and forwards, the body for replies.
  useEffect(() => {
    const t = setTimeout(() => {
      if (initial.mode === "reply" || initial.mode === "reply_all" || initial.to.length > 0) {
        const el = bodyRef.current;
        el?.focus();
        el?.setSelectionRange(0, 0);
      } else toRef.current?.focus();
    }, 0);
    return () => clearTimeout(t);
  }, [initial.mode, initial.to.length]);

  // The reply quote / forwarded message, once the original body is on the device.
  const thread = backend.useThread((d.threadId ?? null) as never);
  const quoteMeta: MessageMeta | undefined = d.quoteOf
    ? thread?.messages.find((x) => x._id === d.quoteOf)
    : undefined;
  const [, bodyTick] = useState(0);
  useEffect(() => {
    const off = loader.subscribe(() => bodyTick((n) => n + 1));
    return () => {
      off();
    };
  }, [loader]);
  useEffect(() => {
    if (thread && d.quoteOf && d.quoted === null) void ensureBodies(loader, thread);
  }, [thread, d.quoteOf, d.quoted, loader]);
  const quoteBody = quoteMeta ? loader.cache.peek(quoteMeta._id) : undefined;
  useEffect(() => {
    if (!quoteMeta || d.quoted !== null || !quoteBody) return;
    const text = visibleText(quoteBody, quoteMeta);
    if (d.mode === "forward") {
      const parts: ComposeAttachment[] = quoteBody.attachments
        .filter((a) => !a.inline)
        .map((a) => ({
          key: `fwd-${a.partId}`,
          filename: a.filename,
          size: a.size,
          mimeType: a.mimeType,
          status: "ready",
          fromMessageId: quoteMeta._id,
          partId: a.partId,
        }));
      edit(
        (cur) => ({
          quoted: forwardBlock(quoteMeta, text, tz),
          attachments: [...parts, ...cur.attachments],
        }),
        { system: true },
      );
    } else edit({ quoted: replyQuote(quoteMeta, text, tz) }, { system: true });
  }, [quoteMeta, quoteBody, d.quoted, d.mode, tz, edit]);

  // ───────────── identity ─────────────
  const fromOptions = useMemo(() => {
    const out: { value: string; label: string; accountId: string; email: string }[] = [];
    for (const a of accounts) {
      if (d.mode !== "new" && a._id !== d.accountId) continue;
      const ids = identities.filter((i) => i.accountId === a._id);
      const emails = ids.length
        ? ids.map((i) => ({ email: i.email, name: i.name }))
        : [{ email: a.email, name: a.displayName }];
      for (const e of emails)
        out.push({
          value: `${a._id}|${e.email}`,
          label: e.name ? `${e.name} <${e.email}>` : e.email,
          accountId: a._id,
          email: e.email,
        });
    }
    return out;
  }, [accounts, identities, d.mode, d.accountId]);
  const identity = identities.find((i) => i.accountId === d.accountId && i.email === d.identityEmail);

  // ───────────── attachments (§10.4) ─────────────
  const addFiles = useCallback(
    (files: readonly File[]) => {
      let total = attachmentBytes(d);
      for (const file of files) {
        if (total + file.size > MAX_ATTACHMENT_BYTES) {
          toast({
            title: `${file.name} is too large. Attachments are limited to 25 MB per message.`,
            tone: "error",
          });
          continue;
        }
        total += file.size;
        const key = uuidv7();
        const att: ComposeAttachment = {
          key,
          filename: file.name,
          size: file.size,
          mimeType: file.type || "application/octet-stream",
          status: "uploading",
        };
        edit((cur) => ({ attachments: [...cur.attachments, att] }));
        backend.worker
          .upload(file, file.name)
          .then((r) =>
            edit((cur) => ({
              attachments: cur.attachments.map((a) =>
                a.key === key ? { ...a, status: "ready", uploadId: r.uploadId } : a,
              ),
            })),
          )
          .catch((e) =>
            edit((cur) => ({
              attachments: cur.attachments.map((a) =>
                a.key === key ? { ...a, status: "error", error: errorMessage(e) } : a,
              ),
            })),
          );
      }
    },
    [d, edit, backend.worker, toast],
  );

  // ───────────── send, schedule, discard, close ─────────────
  const sendDeps = (): SendDeps => ({
    worker: backend.worker,
    mutations: m,
    fetchDraft: backend.fetchDraft,
    toast,
    dismiss,
    local: draftStore(),
    deviceId: deviceId(),
    html: textToHtml,
    reopen: (draft) => composer.open(draft),
    timezone: tz,
  });

  const send = async (sendAt?: number) => {
    const p = sendProblem(d);
    if (p) {
      setProblem(p);
      if (p.startsWith("Add")) toRef.current?.focus();
      return;
    }
    stream.cancel();
    composer.close();
    focusList();
    // A save in flight bumps the version; send on top of it so the worker's compare-and-set accepts it.
    const latest = { ...d, version: (await saver.settle()).version };
    saver.dispose();
    await draftStore()
      .put(latest.clientDraftId, latest)
      .catch(() => undefined);
    sendDraft(
      latest,
      { undoSeconds: prefs.undoSendSeconds, ...(sendAt !== undefined && { sendAt }) },
      sendDeps(),
    );
  };

  const close = async () => {
    stream.cancel();
    composer.close();
    focusList();
    if (!touched.current || !hasContent(d)) {
      saver.dispose();
      await draftStore()
        .delete(d.clientDraftId)
        .catch(() => undefined);
      if (saver.current().version > 0)
        void m.discardDraft({ clientOpId: uuidv7(), clientDraftId: d.clientDraftId }).catch(() => undefined);
      return;
    }
    await saver.flush().catch(() => undefined);
    const offline = saver.status === "error";
    saver.dispose();
    toast(
      offline
        ? {
            title: "Draft kept on this device.",
            description: "It could not reach your mail provider. Reopen it from the command palette.",
          }
        : { title: "Draft saved.", durationMs: 2000 },
    );
  };

  const discard = async () => {
    stream.cancel();
    saver.dispose();
    composer.close();
    focusList();
    await draftStore()
      .delete(d.clientDraftId)
      .catch(() => undefined);
    if (saver.current().version > 0)
      await m.discardDraft({ clientOpId: uuidv7(), clientDraftId: d.clientDraftId }).catch(() => undefined);
    toast({ title: "Draft discarded.", durationMs: 2500 });
  };

  // Another device saved this draft meanwhile: load its copy (adopt) or keep this one (§10.1 step 3).
  const loadTheirs = async () => {
    try {
      const meta = await backend.fetchDraft(d.clientDraftId);
      const t = meta?.threadId ? await backend.fetchThread(meta.threadId) : null;
      const msg = t?.messages.find((x) => x.isDraft && x.rfcMessageId === meta?.rfcMessageId);
      if (!msg) throw new Error("not_found");
      const r = await backend.worker.adoptDraft(d.clientDraftId, msg._id);
      const next: LocalDraft = {
        ...d,
        to: r.draft.to,
        cc: r.draft.cc,
        bcc: r.draft.bcc,
        subject: r.draft.subject,
        body: r.draft.html ? htmlToText(r.draft.html) : r.draft.text,
        quoted: null,
        quoteOf: undefined,
        version: r.version,
      };
      saver.replace(next);
      setD(next);
    } catch {
      toast({ title: "Couldn't load the other copy. Keep this one, or discard it.", tone: "error" });
    }
  };

  // ───────────── snippets ─────────────
  const snippets = backend.useSnippets() ?? [];
  const insertAtCursor = useCallback(
    (text: string, replace?: { start: number; end: number }) => {
      const el = bodyRef.current;
      edit((cur) => {
        const start = replace?.start ?? el?.selectionStart ?? cur.body.length;
        const end = replace?.end ?? el?.selectionEnd ?? cur.body.length;
        const body = cur.body.slice(0, start) + text + cur.body.slice(end);
        queueMicrotask(() => {
          el?.focus();
          el?.setSelectionRange(start + text.length, start + text.length);
        });
        return { body };
      });
    },
    [edit],
  );
  const applySnippet = (snippetId: string, replace?: { start: number; end: number }) => {
    const s = snippets.find((x) => x._id === snippetId);
    if (!s) return;
    const { text, missing } = expandVariables(htmlToText(s.bodyHtml), builtinVariables(d.to, Date.now(), tz));
    if (s.subject && !d.subject.trim()) edit({ subject: s.subject });
    insertAtCursor(text, replace);
    void m.markSnippetUsed({ snippetId: s._id }).catch(() => undefined);
    if (missing.length) toast({ title: `Fill in ${missing.map((v) => `{{${v}}}`).join(", ")}.` });
  };
  const snippetItems: PickerItem[] = snippets.map((s) => ({
    id: s._id,
    label: s.name,
    ...(s.shortcut && { detail: `;${s.shortcut}` }),
  }));

  // `;shortcut` then Tab expands a snippet in place.
  const onBodyKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Tab" || e.shiftKey) return;
    const el = e.currentTarget;
    const before = el.value.slice(0, el.selectionStart);
    const hit = /;([a-z0-9_-]{1,32})$/i.exec(before);
    const s = hit && snippets.find((x) => x.shortcut?.toLowerCase() === hit[1]!.toLowerCase());
    if (!hit || !s) return;
    e.preventDefault();
    applySnippet(s._id, { start: el.selectionStart - hit[0].length, end: el.selectionStart });
  };

  // ───────────── AI: write, rewrite, dictation (§12) ─────────────
  const runWrite = async () => {
    const text = instruction.trim();
    if (!text) return;
    const detail = d.threadId ? await backend.fetchThread(d.threadId as never) : null;
    if (detail) await ensureBodies(loader, detail);
    const mod = ai.ai ?? (await loadAi());
    const vars = mod.composeVars({
      instruction: text,
      userName: me.name ?? me.email,
      thread: detail ? threadForAi(detail, loader, tz) : null,
      draft: d.body.trim() ? d.body : null,
      voice: voiceFor(ai.settings),
    });
    aiTarget.current = { kind: "write", start: 0, end: d.body.length, before: d.body };
    void stream.start("compose", vars);
  };
  const runRewrite = (style: "shorter" | "friendlier" | "formal" | "fix") => {
    const el = bodyRef.current;
    const hasSel = el && el.selectionEnd > el.selectionStart;
    const start = hasSel ? el.selectionStart : 0;
    const end = hasSel ? el.selectionEnd : d.body.length;
    const text = d.body.slice(start, end);
    if (!text.trim()) return;
    aiTarget.current = { kind: "rewrite", start, end, before: d.body };
    void stream.start("rewrite", { text, style, voice: voiceFor(ai.settings) });
  };
  // From the palette: the body is what the rewrite changes, so it has the keyboard afterwards.
  const rewriteFromCommand = (style: "shorter" | "friendlier" | "formal" | "fix") => {
    focusNext.current = "body";
    runRewrite(style);
    setFocusTick((n) => n + 1);
  };
  // Stream into the body as it arrives; offer Undo once done.
  useEffect(() => {
    const t = aiTarget.current;
    if (!t || (stream.state.status !== "running" && stream.state.status !== "done")) return;
    const out = stream.state.text;
    const body = t.before.slice(0, t.start) + out + t.before.slice(t.end);
    setD((cur) => (cur.body === body ? cur : { ...cur, body }));
    if (stream.state.status === "done") {
      aiTarget.current = null;
      edit({ body });
      setAiUndo(t.before);
      setAiBar(null);
      setInstruction("");
    }
  }, [stream.state, edit]);
  useEffect(() => {
    if (stream.state.status === "error") aiTarget.current = null;
  }, [stream.state.status]);
  const aiBusy = stream.state.status === "running";
  const dictation = useDictation(ai, (text) => insertAtCursor(text.endsWith(" ") ? text : `${text} `));

  // ───────────── keyboard ─────────────
  const handlers: Record<string, () => void> = {
    "compose.send": () => void send(),
    "compose.sendLater": () => openLater(),
    "compose.discard": () => void discard(),
    "compose.close": () => {
      if (picker) return setPicker(null);
      if (panel) return setPanel(null);
      if (aiBusy) return stream.cancel();
      if (aiBar) return setAiBar(null);
      void close();
    },
    "compose.snippet": () => setPicker("snippet"),
    "compose.attach": () => fileRef.current?.click(),
    "compose.cc": () => {
      focusNext.current = "cc";
      edit({ showCc: true });
    },
    "compose.bcc": () => {
      focusNext.current = "bcc";
      edit({ showBcc: true });
    },
    "compose.aiWrite": () => {
      setAiBar("write");
      queueMicrotask(() => aiInputRef.current?.focus());
    },
    // Palette-only (D-289): rewrite the selection or the whole body, dictation.
    "composer:rewrite:shorter": () => rewriteFromCommand("shorter"),
    "composer:rewrite:friendlier": () => rewriteFromCommand("friendlier"),
    "composer:rewrite:formal": () => rewriteFromCommand("formal"),
    "composer:rewrite:fix": () => rewriteFromCommand("fix"),
    ...(dictationSupported() && { "composer:dictate": dictation.toggle }),
  };
  composerCommands.current = handlers;
  useEffect(
    () => () => {
      composerCommands.current = null;
    },
    [],
  );

  // One-shot requests from outside (palette, instant replies).
  const lastReq = useRef(0);
  useEffect(() => {
    const r = state.request;
    if (!r || r.seq === lastReq.current) return;
    lastReq.current = r.seq;
    if (r.kind === "snippet") setPicker("snippet");
    else if (r.kind === "send_later") setPicker("later");
    else if (r.kind === "ai_write") handlers["compose.aiWrite"]!();
    else if (r.kind === "insert" && r.text) insertAtCursor(r.text);
  });

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (e.dataTransfer.files.length) addFiles([...e.dataTransfer.files]);
  };

  const onFiles = (e: ChangeEvent<HTMLInputElement>) => {
    if (e.target.files?.length) addFiles([...e.target.files]);
    e.target.value = "";
  };

  const statusText = status === "pending" && saver.current().version === 0 ? "" : STATUS_TEXT[status];

  function openLater() {
    const r = laterRef.current?.getBoundingClientRect();
    setLaterAnchor(r ? { top: r.top, bottom: r.bottom, left: r.left, right: r.right } : null);
    setPicker("later");
  }

  return (
    <section
      ref={rootRef}
      className="ak-compose"
      data-docked={dock?.kind}
      style={
        dock
          ? ({
              "--ak-dock-left": `${dock.left}px`,
              "--ak-dock-top": `${dock.top}px`,
              "--ak-dock-width": `${dock.width}px`,
              "--ak-dock-height": `${dock.height}px`,
            } as CSSProperties)
          : undefined
      }
      role="dialog"
      aria-modal="false"
      aria-label={MODE_TITLE[d.mode]}
      data-dragging={dragging || undefined}
      onFocus={() => composer.setFocused(true)}
      onBlur={(e) => {
        if (!rootRef.current?.contains(e.relatedTarget as Node | null)) composer.setFocused(false);
      }}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      <header className="ak-compose__header">
        <h2 className="ak-compose__title">{MODE_TITLE[d.mode]}</h2>
        <span className="ak-compose__status" aria-live="polite" data-status={status}>
          {statusText}
          {status === "error" && (
            <button type="button" className="ak-link" onClick={() => void saver.retry()}>
              Retry
            </button>
          )}
        </span>
        <button type="button" className="ak-compose__close" aria-label="Close" onClick={() => void close()}>
          <Kbd shortcut="compose.close" />
        </button>
      </header>

      {status === "conflict" && (
        <div className="ak-banner ak-banner--warn ak-compose__conflict" role="alert">
          <span>This draft was changed on another device.</span>
          <Button size="sm" onClick={() => void loadTheirs()}>
            Load that copy
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void saver.keepMine()}>
            Keep this one
          </Button>
        </div>
      )}

      <div className="ak-compose__fields" data-from={fromOptions.length > 1 || undefined}>
        {fromOptions.length > 1 && (
          <div className="ak-compose__row">
            <label className="ak-compose__label" htmlFor="ak-compose-from">
              From
            </label>
            <select
              id="ak-compose-from"
              className="ak-input ak-input--select ak-compose__from"
              value={`${d.accountId}|${d.identityEmail}`}
              onChange={(e) => {
                const o = fromOptions.find((x) => x.value === e.target.value);
                if (o) edit({ accountId: o.accountId as never, identityEmail: o.email });
              }}
            >
              {fromOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
        )}
        <RecipientField label="To" value={d.to} onChange={(to) => edit({ to })} inputRef={toRef} />
        {d.showCc ? (
          <RecipientField label="Cc" value={d.cc} onChange={(cc) => edit({ cc })} inputRef={ccRef} />
        ) : null}
        {d.showBcc ? (
          <RecipientField label="Bcc" value={d.bcc} onChange={(bcc) => edit({ bcc })} inputRef={bccRef} />
        ) : null}
        {(!d.showCc || !d.showBcc) && (
          <div className="ak-compose__more">
            {!d.showCc && (
              <button type="button" className="ak-compose__ccbcc" onClick={handlers["compose.cc"]}>
                Cc
              </button>
            )}
            {!d.showBcc && (
              <button type="button" className="ak-compose__ccbcc" onClick={handlers["compose.bcc"]}>
                Bcc
              </button>
            )}
          </div>
        )}
        <div className="ak-compose__row">
          <label className="ak-compose__label" htmlFor="ak-compose-subject">
            Subject
          </label>
          <input
            id="ak-compose-subject"
            className="ak-compose__subject"
            value={d.subject}
            onChange={(e) => edit({ subject: e.target.value })}
            autoComplete="off"
          />
        </div>
      </div>

      <div className="ak-compose__body">
        <label htmlFor="ak-compose-body" className="ak-visually-hidden">
          Message
        </label>
        <textarea
          id="ak-compose-body"
          ref={bodyRef}
          className="ak-compose__text"
          value={d.body}
          readOnly={aiBusy}
          aria-busy={aiBusy || undefined}
          placeholder={d.mode === "new" ? "Write your message" : ""}
          onChange={(e) => edit({ body: e.target.value })}
          onKeyDown={onBodyKeyDown}
          onPaste={(e) => {
            const files = [...e.clipboardData.files];
            if (files.length) {
              e.preventDefault();
              addFiles(files);
            }
          }}
        />
        {identity?.signatureHtml && (
          <div className="ak-compose__signature" title="Signature">
            {htmlToText(identity.signatureHtml)}
          </div>
        )}
        {d.quoteOf && (
          <div className="ak-compose__quote">
            {d.quoted === null ? (
              <span className="ak-compose__hint">
                <Spinner size="sm" label="Loading the original message" /> Loading the original message…
              </span>
            ) : (
              <>
                <button
                  type="button"
                  className="ak-quoted-toggle"
                  aria-expanded={showQuote}
                  onClick={() => setShowQuote((s) => !s)}
                >
                  {showQuote ? "Hide" : "Show"} {d.mode === "forward" ? "forwarded message" : "quoted text"}
                </button>
                {showQuote && <pre className="ak-compose__quoted">{d.quoted}</pre>}
              </>
            )}
          </div>
        )}
      </div>

      {d.attachments.length > 0 && (
        <ul className="ak-compose__files" aria-label="Attachments">
          {d.attachments.map((a) => (
            <li key={a.key} className="ak-compose__file" data-status={a.status}>
              <IconPaperclip />
              <span className="ak-attachment__name">{a.filename}</span>
              <span className="ak-attachment__size">
                {a.status === "uploading" ? "Uploading…" : a.status === "error" ? a.error : fileSize(a.size)}
              </span>
              <button
                type="button"
                className="ak-chip__remove"
                aria-label={`Remove ${a.filename}`}
                onClick={() =>
                  edit((cur) => ({ attachments: cur.attachments.filter((x) => x.key !== a.key) }))
                }
              >
                <IconClose />
              </button>
            </li>
          ))}
        </ul>
      )}

      {problem && (
        <p className="ak-compose__error" role="alert">
          {problem}
        </p>
      )}
      {stream.state.status === "error" && (
        <p className="ak-compose__error" role="alert">
          {errorMessage(stream.state.error)}
        </p>
      )}
      {dictation.state.status === "error" && (
        <p className="ak-compose__error" role="alert">
          {errorMessage(dictation.state.error)}
        </p>
      )}

      {aiBar === "write" && (
        <div className="ak-compose__ai">
          <label className="ak-visually-hidden" htmlFor="ak-compose-ai">
            Write with AI
          </label>
          <div className="ak-compose__ai-row">
            <input
              id="ak-compose-ai"
              ref={aiInputRef}
              className="ak-compose__ai-input"
              placeholder="Tell it what to write"
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
                  e.preventDefault();
                  void runWrite();
                }
              }}
              disabled={aiBusy}
            />
            {aiBusy ? (
              <Button size="sm" onClick={stream.cancel}>
                Stop
              </Button>
            ) : (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void runWrite()}
                disabled={ai.status("compose") !== "ready"}
              >
                Write
              </Button>
            )}
          </div>
          <EgressNote ai={ai} feature="compose" />
        </div>
      )}

      {panel === "rewrite" && (
        <fieldset className="ak-compose__panel">
          <legend className="ak-visually-hidden">Rewrite and dictate</legend>
          <RewriteButtons ai={ai} disabled={aiBusy || !d.body.trim()} onRewrite={runRewrite} />
          {dictationSupported() && (
            <button
              type="button"
              className="ak-compose__panel-item"
              aria-pressed={dictation.state.status === "recording"}
              onClick={dictation.toggle}
              disabled={ai.status("dictation") !== "ready" || dictation.state.status === "transcribing"}
            >
              {dictation.state.status === "recording" && <span className="ak-rec" aria-hidden="true" />}
              {dictation.state.status === "recording" ? "Listening… · Stop dictation" : "Dictate"}
            </button>
          )}
          <EgressNote ai={ai} feature="rewrite" />
        </fieldset>
      )}
      {panel === "more" && (
        <fieldset className="ak-compose__panel ak-compose__panel--more">
          <legend className="ak-visually-hidden">More</legend>
          <label className="ak-compose__followup">
            <span>Remind me if no reply</span>
            <select
              className="ak-input ak-input--select"
              aria-label="Follow-up reminder"
              value={d.followUp ?? "off"}
              onChange={(e) =>
                edit({ followUp: e.target.value === "off" ? null : (e.target.value as FollowUp) })
              }
            >
              {FOLLOW_UPS.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.label}
                </option>
              ))}
            </select>
          </label>
          {prefs.readReceipts.enabled && (
            <label
              className="ak-check ak-compose__receipt"
              title="Unreliable: Apple Mail Privacy Protection opens every message."
            >
              <input
                type="checkbox"
                checked={d.readReceipt}
                onChange={(e) => edit({ readReceipt: e.target.checked })}
              />
              Read receipt
            </label>
          )}
          <button
            type="button"
            className="ak-compose__panel-item"
            onClick={() => {
              setPanel(null);
              setPicker("snippet");
            }}
          >
            Insert snippet <Kbd shortcut="compose.snippet" />
          </button>
          <button
            type="button"
            className="ak-compose__panel-item ak-compose__panel-item--danger"
            onClick={() => void discard()}
          >
            Discard draft <Kbd shortcut="compose.discard" />
          </button>
        </fieldset>
      )}

      <footer className="ak-compose__footer">
        <Button variant="ink" shortcut="compose.send" onClick={() => void send()}>
          Send
        </Button>
        <Button ref={laterRef} variant="secondary" shortcut="compose.sendLater" onClick={openLater}>
          Send later
        </Button>
        <Button
          variant="ghost"
          shortcut="compose.aiWrite"
          className="ak-compose__write"
          onClick={handlers["compose.aiWrite"]}
        >
          Write with AI
        </Button>
        <button
          type="button"
          className="ak-compose__toggle"
          aria-label="Rewrite and dictate"
          aria-expanded={panel === "rewrite"}
          onClick={() => setPanel((p) => (p === "rewrite" ? null : "rewrite"))}
        >
          <IconChevronDown />
        </button>
        {aiUndo !== null && !aiBusy && (
          <button
            type="button"
            className="ak-textbutton ak-compose__undo"
            onClick={() => {
              edit({ body: aiUndo });
              setAiUndo(null);
            }}
          >
            Undo AI change
          </button>
        )}
        <span className="ak-compose__spacer" />
        <IconButton
          label="Attach files"
          icon={<IconPaperclip />}
          shortcut="compose.attach"
          onClick={() => fileRef.current?.click()}
        />
        <IconButton
          label="More"
          icon={<IconDots />}
          aria-expanded={panel === "more"}
          pressed={panel === "more"}
          onClick={() => setPanel((p) => (p === "more" ? null : "more"))}
        />
        <input ref={fileRef} type="file" multiple hidden onChange={onFiles} aria-label="Attach files" />
      </footer>
      {(aiBusy || dictation.state.status !== "idle") && (
        <EgressNote
          className="ak-compose__where"
          ai={ai}
          feature={
            dictation.state.status !== "idle"
              ? "dictation"
              : aiTarget.current?.kind === "rewrite"
                ? "rewrite"
                : "compose"
          }
        />
      )}

      <ListPicker
        open={picker === "snippet"}
        width={400}
        anchor={laterAnchor}
        title="Insert snippet"
        placeholder="Snippet"
        items={snippetItems}
        emptyText="No snippets yet. Create them in Settings, Snippets."
        onOpenChange={(o) => {
          if (!o) {
            setPicker(null);
            queueMicrotask(() => bodyRef.current?.focus());
          }
        }}
        onPick={(item) => applySnippet(item.id)}
      />
      <TimePicker
        open={picker === "later"}
        anchor={laterAnchor}
        mode="send"
        count={1}
        timezone={tz}
        onOpenChange={(o) => !o && setPicker(null)}
        onPick={(at) => void send(at)}
      />
    </section>
  );
}

function RewriteButtons({
  ai,
  disabled,
  onRewrite,
}: {
  ai: UseAi;
  disabled: boolean;
  onRewrite: (style: "shorter" | "friendlier" | "formal" | "fix") => void;
}) {
  const ready = ai.status("rewrite") === "ready";
  return (
    <fieldset className="ak-compose__rewrite">
      <legend className="ak-visually-hidden">Rewrite</legend>
      {(
        [
          ["shorter", "Shorter"],
          ["friendlier", "Friendlier"],
          ["formal", "More formal"],
          ["fix", "Fix grammar"],
        ] as const
      ).map(([style, label]) => (
        <button
          key={style}
          type="button"
          className="ak-compose__panel-item"
          disabled={disabled || !ready}
          onClick={() => onRewrite(style)}
        >
          {label}
        </button>
      ))}
    </fieldset>
  );
}

function focusList() {
  queueMicrotask(() => {
    const el = document.querySelector<HTMLElement>(".ak-listpane [role='listbox']");
    el?.focus();
  });
}

export type { Addr };

/** Above phone width the composer docks into the reading pane (app.css); below it is a full-screen sheet. */
const DOCK_QUERY = "(min-width: 769px)";

interface Dock {
  /** `slot`: new mail takes the whole reader slot; `bottom`: a reply expands at the foot of the thread. */
  kind: "slot" | "bottom";
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Where the composer sits (SPEC §3.4). New mail takes the reader slot (the list stays usable on the left); replies,
 * reply-all and forwards expand at the bottom of the open thread, full reader width, and the pane gets that much
 * bottom padding and scrolls to the end, so the message being answered stays readable above it. With no reading
 * pane on screen (two panes, no thread open), new mail is a sheet on the right (CSS). The composer's height is
 * published as `--ak-compose-h` so toasts can keep clear of it on phones.
 */
function useReplyDock(mode: LocalDraft["mode"], rootRef: { current: HTMLElement | null }) {
  const [dock, setDock] = useState<Dock | null>(null);
  useLayoutEffect(() => {
    if (typeof matchMedia !== "function") return;
    const mq = matchMedia(DOCK_QUERY);
    const update = () => {
      const pane = document.querySelector<HTMLElement>(".ak-reading");
      const r = pane?.getBoundingClientRect();
      const next: Dock | null =
        mq.matches && r && r.width > 0
          ? {
              kind: mode === "new" ? "slot" : "bottom",
              left: Math.round(r.left),
              top: Math.round(r.top),
              width: Math.round(r.width),
              height: Math.round(r.height),
            }
          : null;
      setDock((cur) =>
        cur?.kind === next?.kind &&
        cur?.left === next?.left &&
        cur?.top === next?.top &&
        cur?.width === next?.width &&
        cur?.height === next?.height
          ? cur
          : next,
      );
    };
    update();
    const pane = document.querySelector<HTMLElement>(".ak-reading");
    const ro = typeof ResizeObserver === "function" ? new ResizeObserver(update) : null;
    if (pane) ro?.observe(pane);
    mq.addEventListener?.("change", update);
    window.addEventListener("resize", update);
    return () => {
      ro?.disconnect();
      mq.removeEventListener?.("change", update);
      window.removeEventListener("resize", update);
    };
  }, [mode]);
  const docked = dock?.kind === "bottom";
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const html = document.documentElement;
    const pane = docked ? document.querySelector<HTMLElement>(".ak-reading") : null;
    const foot = root.querySelector<HTMLElement>(".ak-compose__footer");
    const publish = () => {
      const h = `${Math.round(root.offsetHeight)}px`;
      html.style.setProperty("--ak-compose-h", h);
      if (foot) html.style.setProperty("--ak-compose-foot-h", `${Math.round(foot.offsetHeight)}px`);
      pane?.style.setProperty("--ak-dock-h", h);
    };
    publish();
    const ro = typeof ResizeObserver === "function" ? new ResizeObserver(publish) : null;
    ro?.observe(root);
    if (foot) ro?.observe(foot);
    if (pane) {
      pane.dataset.docked = "";
      // The newest message ends up just above the composer.
      requestAnimationFrame(() => pane.scrollTo({ top: pane.scrollHeight }));
    }
    return () => {
      ro?.disconnect();
      html.style.removeProperty("--ak-compose-h");
      html.style.removeProperty("--ak-compose-foot-h");
      if (pane) {
        delete pane.dataset.docked;
        pane.style.removeProperty("--ak-dock-h");
      }
    };
  }, [docked, rootRef]);
  return dock;
}
