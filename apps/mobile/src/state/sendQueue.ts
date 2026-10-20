// Send with undo (ARCHITECTURE §10.2, §10.3). The composer closes at once and a toast shows "Sending… Undo".
// `POST /v1/send` saves the final draft at the provider and schedules the send in Convex; Undo calls
// `sends.cancel` directly on Convex (which leaves a tombstone if the worker has not scheduled yet) and reopens the
// composer from the local copy. Send later uses the same path with `sendAt`.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import type { DraftContent } from "@akatsuki/wire";
import { router } from "expo-router";
import { convex, requireWorker } from "../lib/clients";
import { kvSet } from "../lib/db";
import { errorText } from "../lib/errors";
import * as haptics from "../lib/haptics";
import { newClientId } from "../lib/ids";
import {
  type ComposeState,
  canUndo,
  draftContent,
  type SendEvent,
  type SendPhase,
  sendReducer,
  sendStatus,
} from "../vm/compose";
import { describeTime } from "../vm/snooze";
import { dismissToast, showToast } from "./toasts";

export const composeKey = (clientDraftId: string) => `compose:${clientDraftId}`;

/** `DraftContent` for the worker (ids in the compose state are plain strings from Convex rows). */
export function toDraftContent(state: ComposeState): DraftContent {
  const { inReplyToMessageId, forwardOfMessageId, ...d } = draftContent(state);
  return {
    ...d,
    accountId: d.accountId as Id<"mailAccounts">,
    ...(inReplyToMessageId ? { inReplyToMessageId: inReplyToMessageId as Id<"messages"> } : {}),
    ...(forwardOfMessageId ? { forwardOfMessageId: forwardOfMessageId as Id<"messages"> } : {}),
  };
}

/** Keeps the local copy the composer reopens from (§10.3: "the composer reopens from the device's local copy"). */
export async function saveLocalCompose(state: ComposeState): Promise<void> {
  await kvSet(composeKey(state.clientDraftId), state);
}

export function reopenCompose(clientDraftId: string): void {
  router.push({ pathname: "/compose", params: { restore: clientDraftId } });
}

export interface SendOptions {
  deviceId: string;
  undoSeconds: number;
  timeZone: string;
  /** Send later. */
  sendAt?: number;
}

export function startSend(state: ComposeState, opts: SendOptions): void {
  const clientSendId = newClientId();
  const toastId = `send:${clientSendId}`;
  const later = opts.sendAt !== undefined;
  let phase: SendPhase = { kind: "idle" };
  let ticker: ReturnType<typeof setInterval> | null = null;
  const describe = (at: number) => describeTime(at, Date.now(), opts.timeZone);

  const render = () => {
    const now = Date.now();
    const text = sendStatus(phase, now, describe);
    const undoable = canUndo(phase, now);
    const terminal =
      phase.kind === "sent" ||
      phase.kind === "cancelled" ||
      phase.kind === "too_late" ||
      phase.kind === "failed";
    showToast({
      id: toastId,
      text,
      tone: phase.kind === "failed" ? "error" : "default",
      durationMs: terminal ? (phase.kind === "failed" ? 0 : 3000) : 0,
      actions: undoable
        ? [{ label: "Undo", run: undo }]
        : phase.kind === "failed"
          ? [
              {
                label: "Open draft",
                run: () => {
                  dismissToast(toastId);
                  reopenCompose(state.clientDraftId);
                },
              },
            ]
          : [],
    });
    if (terminal && ticker) {
      clearInterval(ticker);
      ticker = null;
    }
  };

  const dispatch = (e: SendEvent) => {
    phase = sendReducer(phase, e);
    render();
  };

  function undo() {
    if (!canUndo(phase, Date.now())) return;
    dispatch({ type: "undo" });
    haptics.tick();
    if (!convex) return;
    convex
      .mutation(api.sends.cancel, { clientSendId })
      .then(({ result }) => {
        dispatch({ type: "cancel_result", result });
        if (result === "cancelled") reopenCompose(state.clientDraftId);
      })
      .catch((err) => dispatch({ type: "error", message: errorText(err) }));
  }

  dispatch({ type: "submit", clientSendId, later });
  const draft = toDraftContent(state);
  saveLocalCompose(state)
    .catch(() => {})
    .then(() =>
      requireWorker().send({
        clientSendId,
        clientDraftId: state.clientDraftId,
        baseVersion: state.baseVersion,
        draft,
        ...(later ? { sendAt: opts.sendAt } : {}),
        undoSeconds: opts.undoSeconds,
        deviceId: opts.deviceId,
      }),
    )
    .then((accepted) => {
      dispatch({ type: "accepted", sendAt: accepted.sendAt, state: accepted.state });
      if (phase.kind === "scheduled" && !later) {
        ticker = setInterval(() => {
          if (phase.kind === "scheduled" && Date.now() >= phase.sendAt) dispatch({ type: "elapsed" });
          else render();
        }, 1000);
      }
      if (phase.kind === "sent" || (phase.kind === "scheduled" && later)) haptics.success();
    })
    .catch((err) => {
      haptics.warning();
      dispatch({ type: "error", message: `Not sent: ${errorText(err)}` });
    });
}
