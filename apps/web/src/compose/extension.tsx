// Registers the composer with the mail shell (src/app/extensions.ts, D-268): `c`, `r`, `a`, `f`, the compose-
// scope keys while it has focus, `z` for undo-send inside the window, palette entries (continue a draft, the
// scheduled messages), a `/compose?to=&subject=&body=` deep link, and the thread footer's "Continue draft".
import { Button, Dialog, errorMessage, Kbd, type PaletteItem, useToast } from "@akatsuki/ui";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { notifyExtensions, registerExtension } from "../app/extensions.ts";
import { navigate } from "../app/router.tsx";
import { useBackend } from "../backend/context.tsx";
import type { MeView, ScheduledSendView, ThreadDetail, ThreadId } from "../backend/types.ts";
import { fullDate } from "../lib/format.ts";
import { deviceId, uuidv7 } from "../lib/ids.ts";
import { TimePicker } from "../mail/TimePicker.tsx";
import { Composer, composerCommands, draftStore } from "./Composer.tsx";
import { draftFor, type LocalDraft, newDraft } from "./model.ts";
import { type SendDeps, undoSend } from "./sendFlow.ts";
import { composer, useComposer } from "./store.ts";
import { htmlToText, parseAddressList, textToHtml } from "./text.ts";

/** Composer actions without keys, offered in the palette while the composer is open (D-289). */
const COMPOSER_PALETTE: [string, string][] = [
  ["composer:rewrite:shorter", "Rewrite: shorter"],
  ["composer:rewrite:friendlier", "Rewrite: friendlier"],
  ["composer:rewrite:formal", "Rewrite: more formal"],
  ["composer:rewrite:fix", "Rewrite: fix grammar"],
  ["composer:dictate", "Dictate"],
];

/** Send deps of the mounted overlay, for `z` (undo-send) from the shell. */
const sendDepsRef: { current: SendDeps | null } = { current: null };

// ───────────── drafts kept on this device (palette "Continue draft") ─────────────
let localDrafts: LocalDraft[] = [];
async function refreshLocalDrafts() {
  try {
    const all = (await draftStore().entries()).map(([, v]) => v);
    const open = composer.get().draft?.clientDraftId;
    const sending = new Set(composer.get().sends.map((s) => s.draft.clientDraftId));
    localDrafts = all
      .filter((d) => d.clientDraftId !== open && !sending.has(d.clientDraftId))
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, 8);
  } catch {
    localDrafts = [];
  }
  notifyExtensions();
}

// ───────────── the scheduled-messages dialog ─────────────
let scheduledOpen = false;
const scheduledListeners = new Set<() => void>();
function setScheduledOpen(v: boolean) {
  scheduledOpen = v;
  for (const l of [...scheduledListeners]) l();
}
const useScheduledOpen = () =>
  useSyncExternalStore(
    (cb) => {
      scheduledListeners.add(cb);
      return () => {
        scheduledListeners.delete(cb);
      };
    },
    () => scheduledOpen,
    () => scheduledOpen,
  );

function ComposeOverlay({ me }: { me: MeView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast, dismiss } = useToast();
  const state = useComposer();
  const accounts = backend.useAccounts();
  const identities = backend.useIdentities();
  const myEmails = useMemo(
    () =>
      new Set([
        ...(accounts ?? []).map((a) => a.email.toLowerCase()),
        ...(identities ?? []).map((i) => i.email.toLowerCase()),
      ]),
    [accounts, identities],
  );

  sendDepsRef.current = {
    worker: backend.worker,
    mutations: m,
    fetchDraft: backend.fetchDraft,
    toast,
    dismiss,
    local: draftStore(),
    deviceId: deviceId(),
    html: textToHtml,
    reopen: (d) => composer.open(d),
    timezone: me.profile.timezone,
  };

  // Replies wait for their thread; new mail waits for the accounts and identities.
  const pending = state.pending;
  const detail = backend.useThread((pending?.threadId ?? null) as ThreadId | null);
  useEffect(() => {
    if (!pending || detail === undefined || !accounts || !identities) return;
    if (detail === null) {
      composer.clearPending();
      toast({ title: "That conversation no longer exists.", tone: "error" });
      return;
    }
    const d = draftFor(detail, pending.mode, { accounts, identities, myEmails });
    composer.open(pending.body ? { ...d, body: pending.body } : d);
  }, [pending, detail, accounts, identities, myEmails, toast]);
  useEffect(() => {
    const p = state.pendingNew;
    if (!p || !accounts || !identities) return;
    if (accounts.length === 0) {
      composer.clearPending();
      toast({ title: "Connect a mailbox before you write.", tone: "neutral" });
      return;
    }
    composer.open(
      newDraft(
        { accounts, identities, myEmails },
        {
          to: p.to ? parseAddressList(p.to).valid : [],
          ...(p.subject && { subject: p.subject }),
          ...(p.body && { body: p.body }),
        },
      ),
    );
  }, [state.pendingNew, accounts, identities, myEmails, toast]);

  useEffect(() => {
    if (!state.draft) return;
    document.body.dataset.composing = "";
    return () => {
      delete document.body.dataset.composing;
    };
  }, [state.draft]);
  // The local-draft list follows the composer; `z` stops shadowing mail undo when a window closes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh when a draft opens or closes
  useEffect(() => {
    void refreshLocalDrafts();
  }, [state.draft?.clientDraftId]);
  useEffect(() => {
    const next = state.sends.reduce((n, s) => Math.min(n, s.undoUntil), Number.POSITIVE_INFINITY);
    if (!Number.isFinite(next)) return;
    const t = setTimeout(() => notifyExtensions(), Math.max(0, next - Date.now()) + 50);
    return () => clearTimeout(t);
  }, [state.sends]);

  return (
    <>
      {state.draft && accounts && identities && (
        <Composer
          key={state.session}
          initial={state.draft}
          me={me}
          accounts={accounts}
          identities={identities}
        />
      )}
      <ScheduledDialog timezone={me.profile.timezone} />
    </>
  );
}

function ScheduledDialog({ timezone }: { timezone: string }) {
  const open = useScheduledOpen();
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const sends = backend.useSends(open ? ["scheduled", "failed", "unknown"] : undefined);
  const [reschedule, setReschedule] = useState<ScheduledSendView | null>(null);
  const rows = (open ? (sends ?? []) : []).filter((s) => s.state !== "cancelled" && s.draftId);
  const act = (p: Promise<unknown>, done: string) =>
    p.then(() => toast({ title: done })).catch((e) => toast({ title: errorMessage(e), tone: "error" }));
  return (
    <>
      <Dialog
        open={open && !reschedule}
        onOpenChange={(o) => !o && setScheduledOpen(false)}
        title="Scheduled messages"
        size="md"
      >
        {sends === undefined ? (
          <p className="ak-settings__hint">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="ak-settings__hint">Nothing is scheduled. Use Send later in the composer.</p>
        ) : (
          <ul className="ak-scheduled">
            {rows.map((s) => (
              <li key={s._id} className="ak-scheduled__row" data-state={s.state}>
                <div className="ak-scheduled__what">
                  <span className="ak-scheduled__subject">{s.preview.subject || "(no subject)"}</span>
                  <span className="ak-scheduled__to">
                    to {s.preview.to.map((a) => a.name ?? a.email).join(", ") || "nobody"}
                  </span>
                </div>
                {s.state === "scheduled" && (
                  <>
                    <span className="ak-scheduled__when">{fullDate(s.sendAt, timezone)}</span>
                    <Button size="sm" onClick={() => setReschedule(s)}>
                      Change time
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void act(
                          m.cancelSend({ clientSendId: s.clientSendId }),
                          "Cancelled. The message stays in Drafts.",
                        )
                      }
                    >
                      Cancel
                    </Button>
                  </>
                )}
                {s.state === "failed" && (
                  <span className="ak-scheduled__error">
                    Not sent:{" "}
                    {s.error?.code === "stale_schedule"
                      ? "it was more than a day overdue."
                      : "the provider refused it."}{" "}
                    It is still in Drafts.
                  </span>
                )}
                {s.state === "unknown" && (
                  <>
                    <span className="ak-scheduled__error">We could not confirm that this was sent.</span>
                    <Button
                      size="sm"
                      onClick={() =>
                        void act(
                          m.resolveUnknownSend({ clientSendId: s.clientSendId, resolution: "was_sent" }),
                          "Marked as sent.",
                        )
                      }
                    >
                      It was sent
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void act(
                          m.resolveUnknownSend({ clientSendId: s.clientSendId, resolution: "discard" }),
                          "Kept as a draft.",
                        )
                      }
                    >
                      It was not sent
                    </Button>
                  </>
                )}
              </li>
            ))}
          </ul>
        )}
      </Dialog>
      <TimePicker
        open={reschedule !== null}
        mode="send"
        count={1}
        timezone={timezone}
        onOpenChange={(o) => !o && setReschedule(null)}
        onPick={(at) => {
          const s = reschedule;
          setReschedule(null);
          if (s)
            void act(
              m.rescheduleSend({ clientSendId: s.clientSendId, sendAt: at }),
              `Rescheduled for ${fullDate(at, timezone)}.`,
            );
        }}
      />
    </>
  );
}

/** Under an open thread: Reply, Reply all, Forward, and "Continue draft" when this conversation has one. */
function DraftFooter({ thread }: { thread: ThreadDetail }) {
  const backend = useBackend();
  const { toast } = useToast();
  const draft = [...thread.messages].reverse().find((x) => x.isDraft);
  const [busy, setBusy] = useState(false);
  const id = thread.thread._id;
  // The reply box (SPEC §3.1.5): a 40px hairline box at the foot of the pane, "Reply…" and its key; reply-all and
  // forward show their keys when it is hovered or focused. r opens the inline composer in its place.
  const replyBar = (
    <fieldset className="ak-thread__reply">
      <legend className="ak-visually-hidden">Respond</legend>
      <button
        type="button"
        className="ak-replybox"
        aria-keyshortcuts="r"
        onClick={() => composer.requestReply(id, "reply")}
      >
        <span className="ak-visually-hidden">Reply</span>
        <span className="ak-replybox__text" aria-hidden="true">
          Reply…
        </span>
        <Kbd shortcut="thread.reply" />
      </button>
      <span className="ak-replybox__more">
        <button
          type="button"
          className="ak-replybox__alt"
          onClick={() => composer.requestReply(id, "reply_all")}
        >
          Reply all <Kbd shortcut="thread.replyAll" />
        </button>
        <button
          type="button"
          className="ak-replybox__alt"
          onClick={() => composer.requestReply(id, "forward")}
        >
          Forward <Kbd shortcut="thread.forward" />
        </button>
      </span>
    </fieldset>
  );
  if (!draft) return replyBar;
  const open = async () => {
    setBusy(true);
    try {
      const local = (await draftStore().entries())
        .map(([, v]) => v)
        .find((d) => d.threadId === thread.thread._id);
      if (local) return composer.open(local);
      const clientDraftId = uuidv7();
      const r = await backend.worker.adoptDraft(clientDraftId, draft._id);
      composer.open({
        ...draftFor(thread, "reply", { accounts: [], identities: [], myEmails: new Set() }),
        clientDraftId,
        accountId: r.draft.accountId,
        identityEmail: r.draft.identityEmail,
        to: r.draft.to,
        cc: r.draft.cc,
        bcc: r.draft.bcc,
        showCc: r.draft.cc.length > 0,
        subject: r.draft.subject,
        body: r.draft.html ? htmlToText(r.draft.html) : r.draft.text,
        quoted: null,
        quoteOf: undefined,
        ...(r.draft.inReplyToMessageId && { inReplyToMessageId: r.draft.inReplyToMessageId }),
        version: r.version,
      });
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div className="ak-thread__draft">
        <span>You have a draft in this conversation.</span>
        <Button size="sm" variant="secondary" loading={busy} onClick={() => void open()}>
          Continue draft
        </Button>
      </div>
      {replyBar}
    </>
  );
}

/** `/compose?to=&subject=&body=`: opens the composer over the inbox. */
function ComposeRoute() {
  useEffect(() => {
    const p = new URLSearchParams(window.location.search);
    composer.requestNew({
      ...(p.get("to") && { to: p.get("to")! }),
      ...(p.get("subject") && { subject: p.get("subject")! }),
      ...(p.get("body") && { body: p.get("body")! }),
    });
    navigate("/inbox", { replace: true });
  }, []);
  return null;
}

export function registerCompose(): void {
  registerExtension({
    id: "compose",
    routes: { compose: ComposeRoute },
    overlay: ComposeOverlay,
    threadFooter: DraftFooter,
    keyboardScopes: () => {
      const s = composer.get();
      return s.draft && s.focused ? ["compose"] : null;
    },
    commands: (ctx) => {
      const s = composer.get();
      const out: Record<string, () => void> = {
        "compose.new": () =>
          s.draft
            ? queueMicrotask(() => document.getElementById("ak-compose-body")?.focus())
            : composer.requestNew(),
      };
      const target = (ctx.openThread?.thread._id ?? ctx.targets[0]) as ThreadId | undefined;
      if (target && ctx.targets.length <= 1) {
        out["thread.reply"] = () => composer.requestReply(target, "reply");
        out["thread.replyAll"] = () => composer.requestReply(target, "reply_all");
        out["thread.forward"] = () => composer.requestReply(target, "forward");
      }
      // Resolved when the key is pressed: the composer's handlers close over its latest state.
      if (s.draft && composerCommands.current)
        for (const id of Object.keys(composerCommands.current))
          out[id] = () => composerCommands.current?.[id]?.();
      const pendingSend = composer.undoable();
      if (pendingSend && sendDepsRef.current) {
        const deps = sendDepsRef.current;
        out.undo = () => void undoSend(pendingSend.clientSendId, deps);
      }
      return out;
    },
    paletteItems: () => {
      const items: PaletteItem[] = [
        {
          id: "compose:scheduled",
          label: "Show scheduled messages",
          section: "Compose",
          keywords: ["send later", "outbox", "queue"],
          run: () => setScheduledOpen(true),
        },
      ];
      if (composer.get().draft)
        for (const [id, label] of COMPOSER_PALETTE)
          if (composerCommands.current?.[id])
            items.push({
              id,
              label,
              section: "Compose",
              keywords: ["ai", "rewrite", "voice"],
              run: () => composerCommands.current?.[id]?.(),
            });
      for (const d of localDrafts)
        items.push({
          id: `compose:draft:${d.clientDraftId}`,
          label: `Continue draft: ${d.subject || d.to[0]?.email || "(no subject)"}`,
          section: "Compose",
          keywords: ["draft", "resume"],
          run: () => composer.open(d),
        });
      return items;
    },
  });
}
