// Send, undo-send and send-later (ARCHITECTURE §10.2–10.3).
//
// 1. "Sending…  Undo" shows at once; the composer is already closed.
// 2. `POST /v1/send` carries the final content, so the worker saves the provider draft synchronously before
//    `worker.scheduleSend` (the text waits in the provider's Drafts folder, never in Convex).
// 3. Undo calls `sends.cancel({clientSendId})` directly on Convex: `cancelled` reopens the composer from this
//    device's copy (also when Undo beat `/v1/send`: Convex keeps a tombstone that `scheduleSend` honours);
//    `too_late` says so.
// 4. After the window the toast says "Sent.", the device copy is deleted, and a "remind me if no reply"
//    follow-up becomes an `if_no_reply` reminder on the thread (D-283).
import type { ToastApi } from "@akatsuki/ui";
import { errorMessage } from "@akatsuki/ui";
import type { SendAccepted } from "@akatsuki/wire";
import type { Backend, Mutations, ThreadId, WorkerApi } from "../backend/types.ts";
import { fullDate } from "../lib/format.ts";
import { uuidv7 } from "../lib/ids.ts";
import type { KvStore } from "../lib/kv.ts";
import { FOLLOW_UP_MS, type LocalDraft, toDraftContent } from "./model.ts";
import { composer, type PendingSend } from "./store.ts";

export interface SendDeps {
  worker: Pick<WorkerApi, "send">;
  mutations: Pick<Mutations, "cancelSend" | "setReminder">;
  fetchDraft: Backend["fetchDraft"];
  toast: ToastApi["toast"];
  dismiss: ToastApi["dismiss"];
  local: KvStore<LocalDraft>;
  deviceId: string;
  html: (text: string) => string;
  /** Opens the composer again with this draft (after Undo or a failed send). */
  reopen: (d: LocalDraft) => void;
  timezone: string;
  now?: () => number;
  /** Follow-up lookup for new mail: tries × intervalMs until the sent message has a thread. */
  followUpPoll?: { tries: number; intervalMs: number };
}

/** Undo after scheduling a later send stays available this long. */
export const LATER_UNDO_MS = 10_000;

const undone = new Set<string>();
const inflight = new Map<string, Promise<SendAccepted | null>>();

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Starts a send. Returns the pending send (also kept in the composer store while it can be undone). */
export function sendDraft(
  draft: LocalDraft,
  opts: { undoSeconds: number; sendAt?: number },
  deps: SendDeps,
): PendingSend {
  const now = deps.now?.() ?? Date.now();
  const content = toDraftContent(draft, deps.html);
  if (!content) throw new Error("draft has no account");
  const clientSendId = uuidv7(now);
  const later = opts.sendAt !== undefined;
  const undoSeconds = Math.max(0, opts.undoSeconds);
  const sendAt = later ? opts.sendAt! : now + undoSeconds * 1000;
  const undoUntil = later ? now + LATER_UNDO_MS : sendAt;
  const pending: PendingSend = { clientSendId, draft, kind: later ? "later" : "undo", sendAt, undoUntil };
  const canUndo = undoUntil > now;
  if (canUndo) composer.addSend(pending);
  const toastId = `send:${clientSendId}`;
  deps.toast({
    id: toastId,
    title: later ? `Scheduled for ${fullDate(sendAt, deps.timezone)}.` : canUndo ? "Sending…" : "Sent.",
    ...(canUndo && {
      action: { label: "Undo", shortcut: "undo", onAction: () => void undoSend(clientSendId, deps) },
    }),
    durationMs: canUndo ? undoUntil - now : 3000,
  });

  const request = deps.worker
    .send({
      clientSendId,
      clientDraftId: draft.clientDraftId,
      baseVersion: draft.version,
      draft: content,
      ...(later && { sendAt }),
      undoSeconds,
      deviceId: deps.deviceId,
    })
    .then((accepted) => accepted)
    .catch((e) => {
      composer.removeSend(clientSendId);
      deps.dismiss(toastId);
      if (!undone.has(clientSendId))
        deps.toast({
          id: toastId,
          tone: "error",
          title: `Couldn't send: ${errorMessage(e)}`,
          description: "Your message is still a draft on this device.",
          action: { label: "Open draft", onAction: () => deps.reopen(draft) },
          durationMs: null,
        });
      return null;
    });
  inflight.set(clientSendId, request);

  void request.then(async (accepted) => {
    if (!accepted || accepted.state === "cancelled" || undone.has(clientSendId)) return;
    const wait = Math.max(0, undoUntil - (deps.now?.() ?? Date.now()));
    if (wait > 0) await sleep(wait);
    inflight.delete(clientSendId);
    if (undone.has(clientSendId)) return;
    composer.removeSend(clientSendId);
    if (!later) deps.toast({ id: toastId, title: "Sent.", durationMs: 2500 });
    await deps.local.delete(draft.clientDraftId).catch(() => undefined);
    if (draft.followUp) await followUp(draft, sendAt + FOLLOW_UP_MS[draft.followUp], deps);
  });
  return pending;
}

/** Undo for a pending send. Returns true when the message was stopped. */
export async function undoSend(clientSendId: string, deps: SendDeps): Promise<boolean> {
  const p = composer.get().sends.find((s) => s.clientSendId === clientSendId);
  if (!p) return false;
  undone.add(clientSendId);
  composer.removeSend(clientSendId);
  deps.dismiss(`send:${clientSendId}`);
  let result: "cancelled" | "too_late";
  try {
    result = (await deps.mutations.cancelSend({ clientSendId })).result;
  } catch (e) {
    undone.delete(clientSendId);
    deps.toast({ title: `Couldn't undo: ${errorMessage(e)}`, tone: "error" });
    return false;
  }
  if (result === "too_late") {
    undone.delete(clientSendId);
    deps.toast({ title: "Too late to undo. The message was sent." });
    return false;
  }
  // The worker may still be saving the final draft; wait for /v1/send so the reopened copy has its version.
  const running = inflight.get(clientSendId);
  if (running) await Promise.race([running, sleep(5000)]);
  inflight.delete(clientSendId);
  const meta = await deps.fetchDraft(p.draft.clientDraftId).catch(() => null);
  deps.reopen({ ...p.draft, version: meta?.version ?? p.draft.version });
  deps.toast({
    title: p.kind === "later" ? "Scheduled send cancelled." : "Sending cancelled.",
    description: "Your draft is open again.",
  });
  return true;
}

/** "Remind me if no reply": an `if_no_reply` reminder on the thread once it is known (§4.2 setReminder). */
async function followUp(draft: LocalDraft, at: number, deps: SendDeps): Promise<void> {
  let threadId = draft.threadId as ThreadId | undefined;
  const poll = deps.followUpPoll ?? { tries: 10, intervalMs: 3000 };
  for (let i = 0; !threadId && i < poll.tries; i++) {
    await sleep(poll.intervalMs);
    threadId = (await deps.fetchDraft(draft.clientDraftId).catch(() => null))?.threadId;
  }
  if (!threadId) {
    deps.toast({
      title: "Couldn't set the follow-up reminder yet.",
      description: "Open the conversation in Sent and press Shift H.",
    });
    return;
  }
  try {
    await deps.mutations.setReminder({ threadId, at, condition: "if_no_reply" });
    deps.toast({ title: `Follow-up set for ${fullDate(at, deps.timezone)} if nobody replies.` });
  } catch (e) {
    deps.toast({ title: `Couldn't set the follow-up reminder: ${errorMessage(e)}`, tone: "error" });
  }
}

/** Test seam. */
export function resetSendFlow(): void {
  undone.clear();
  inflight.clear();
}
