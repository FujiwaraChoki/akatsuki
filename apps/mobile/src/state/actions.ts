// Thread actions (ARCHITECTURE §2 "life of one action", §5.2, §16.1): build the action with packages/mail-model's
// helpers, write the call to the durable outbox, run `mail.apply` with the optimistic update so the row moves at
// once, and offer Undo for 10 seconds. Undo is instant too: the rows are remembered and put back optimistically
// (vm/undoOptimistic.ts). A snooze is undone with `mail.unsnooze`, because undoing its ops alone would leave the
// timer active. Failures reported later by the worker arrive through `ops.failures` (components/Toasts).

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import {
  ActionUnavailableError,
  isFolderProvider,
  type ThreadAction,
  type ThreadActions,
} from "@akatsuki/mail-model";
import { applyOptimisticUpdate, snoozeOptimisticUpdate } from "@akatsuki/mail-model/optimistic";
import { useMutation } from "convex/react";
import { useCallback, useMemo } from "react";
import { sqliteOutboxStorage } from "../lib/db";
import { errorText } from "../lib/errors";
import * as haptics from "../lib/haptics";
import { newClientId } from "../lib/ids";
import { createOutbox } from "../vm/outbox";
import { describeTime } from "../vm/snooze";
import { rememberForUndo, undoOptimisticUpdate } from "../vm/undoOptimistic";
import { useMail } from "./mail";
import { dismissToast, showToast } from "./toasts";

export const outbox = createOutbox(sqliteOutboxStorage);

export const UNDO_MS = 10_000;

export type ThreadRef = { _id: string; accountId: string };

/** What a gesture does, as a function of the account's action helpers. */
export type ActionBuilder = (a: ThreadActions) => ThreadAction;

export interface ActionSpec {
  /** Past-tense toast text, e.g. "Archived". */
  done: string;
  build: ActionBuilder;
}

export const ACTIONS = {
  archive: { done: "Archived", build: (a) => a.archive() },
  moveToInbox: { done: "Moved to Inbox", build: (a) => a.moveToInbox() },
  trash: { done: "Moved to Trash", build: (a) => a.trash() },
  untrash: { done: "Restored", build: (a) => a.untrash() },
  spam: { done: "Reported as spam", build: (a) => a.spam() },
  notSpam: { done: "Moved to Inbox", build: (a) => a.notSpam() },
  markRead: { done: "Marked as read", build: (a) => a.markRead() },
  markUnread: { done: "Marked as unread", build: (a) => a.markUnread() },
  star: { done: "Starred", build: (a) => a.star() },
  unstar: { done: "Unstarred", build: (a) => a.unstar() },
} satisfies Record<string, ActionSpec>;

export type ActionName = keyof typeof ACTIONS;

/** The newest undoable gesture, for the `z` key. */
let lastUndo: { id: string; run: () => void; at: number } | null = null;
/** `mail.unsnooze` clientOpId → the snooze it undoes (read by the unsnooze optimistic update). */
const unsnoozeOf = new Map<string, string>();

const DEFAULT_SNOOZE: ThreadAction = { kind: "modify", add: ["sys:snoozed"], remove: ["sys:inbox"] };

export function useThreadActions() {
  const mail = useMail();
  const isFolderAccount = useCallback(
    (accountId: string) => isFolderProvider(mail.accountById.get(accountId)?.provider ?? "gmail"),
    [mail.accountById],
  );
  const snoozeActionFor = useCallback(
    (accountId: string) => mail.actionsFor(accountId)?.snooze() ?? DEFAULT_SNOOZE,
    [mail],
  );
  const updates = useMemo(() => {
    const apply = applyOptimisticUpdate({ contextFor: mail.contextFor });
    const snooze = snoozeOptimisticUpdate({ contextFor: mail.contextFor, snoozeActionFor });
    const undo = undoOptimisticUpdate({ contextFor: mail.contextFor, isFolderAccount });
    return { apply, snooze, undo };
  }, [mail.contextFor, snoozeActionFor, isFolderAccount]);

  const applyMutation = useMutation(api.mail.apply).withOptimisticUpdate((store, args) => {
    rememberForUndo(store, args.clientOpId, args.threadIds, args.action);
    updates.apply(store, args);
  });
  const snoozeMutation = useMutation(api.mail.snooze).withOptimisticUpdate((store, args) => {
    rememberForUndo(store, args.clientOpId, args.threadIds, (row) => snoozeActionFor(row.accountId));
    updates.snooze(store, args);
  });
  const undoMutation = useMutation(api.mail.undo).withOptimisticUpdate((store, args) =>
    updates.undo(store, args),
  );
  const unsnoozeMutation = useMutation(api.mail.unsnooze).withOptimisticUpdate((store, args) => {
    const of = unsnoozeOf.get(args.clientOpId);
    if (of) updates.undo(store, { undoClientOpId: of });
  });
  const reminderMutation = useMutation(api.mail.setReminder);

  const fail = useCallback((key: string) => {
    return (err: unknown) => {
      haptics.warning();
      showToast({ id: `err:${key}`, text: errorText(err), tone: "error" });
    };
  }, []);

  /** Shows "<text> · Undo" for 10 s; `undo` runs at most once (toast button or `z`). */
  const offerUndo = useCallback((text: string, undo: () => void) => {
    const id = `undo:${newClientId()}`;
    let done = false;
    const run = () => {
      if (done) return;
      done = true;
      dismissToast(id);
      if (lastUndo?.id === id) lastUndo = null;
      undo();
      haptics.tick();
    };
    showToast({ id, text, durationMs: UNDO_MS, actions: [{ label: "Undo", run }] });
    lastUndo = { id, run, at: Date.now() };
  }, []);

  const undoOps = useCallback(
    (clientOpIds: readonly string[]) => () => {
      for (const undoClientOpId of clientOpIds) {
        const clientOpId = newClientId();
        outbox
          .run("mail.undo", clientOpId, { clientOpId, undoClientOpId }, () =>
            undoMutation({ clientOpId, undoClientOpId }),
          )
          .catch(fail(clientOpId));
      }
    },
    [undoMutation, fail],
  );

  /** Runs one named action on threads (grouped per account: actions differ between Gmail and folder accounts). */
  const run = useCallback(
    async (name: ActionName, threads: readonly ThreadRef[], opts: { undo?: boolean } = {}) => {
      const spec: ActionSpec = ACTIONS[name];
      const byAccount = new Map<string, string[]>();
      for (const t of threads) byAccount.set(t.accountId, [...(byAccount.get(t.accountId) ?? []), t._id]);
      const clientOpIds: string[] = [];
      for (const [accountId, ids] of byAccount) {
        const helpers = mail.actionsFor(accountId);
        if (!helpers) continue;
        let action: ThreadAction;
        try {
          action = spec.build(helpers);
        } catch (err) {
          const text = err instanceof ActionUnavailableError ? err.message : errorText(err);
          showToast({ id: `unavail:${name}`, text, tone: "error" });
          return;
        }
        for (let i = 0; i < ids.length; i += 500) {
          const clientOpId = newClientId();
          const args = { clientOpId, threadIds: ids.slice(i, i + 500) as Id<"threads">[], action };
          clientOpIds.push(clientOpId);
          outbox.run("mail.apply", clientOpId, args, () => applyMutation(args)).catch(fail(clientOpId));
        }
      }
      if (clientOpIds.length === 0) return;
      haptics.success();
      if (opts.undo !== false) offerUndo(spec.done, undoOps(clientOpIds));
    },
    [mail, applyMutation, offerUndo, undoOps, fail],
  );

  const snooze = useCallback(
    (threads: readonly ThreadRef[], until: number) => {
      const clientOpId = newClientId();
      const threadIds = threads.map((t) => t._id) as Id<"threads">[];
      const args = { clientOpId, threadIds, until, returnUnread: false };
      outbox.run("mail.snooze", clientOpId, args, () => snoozeMutation(args)).catch(fail(clientOpId));
      haptics.success();
      offerUndo(`Snoozed until ${describeTime(until, Date.now(), mail.timeZone)}`, () => {
        const undoId = newClientId();
        unsnoozeOf.set(undoId, clientOpId);
        outbox
          .run("mail.unsnooze", undoId, { clientOpId: undoId, threadIds }, () =>
            unsnoozeMutation({ clientOpId: undoId, threadIds }),
          )
          .catch(fail(undoId))
          .finally(() => unsnoozeOf.delete(undoId));
      });
    },
    [snoozeMutation, unsnoozeMutation, offerUndo, fail, mail.timeZone],
  );

  const remind = useCallback(
    (thread: ThreadRef, at: number) => {
      reminderMutation({ threadId: thread._id as Id<"threads">, at, condition: "if_no_reply" })
        .then(() =>
          showToast({
            id: `remind:${thread._id}`,
            text: `Reminder ${describeTime(at, Date.now(), mail.timeZone)} if no reply`,
          }),
        )
        .catch(fail(`remind:${thread._id}`));
      haptics.success();
    },
    [reminderMutation, mail.timeZone, fail],
  );

  const single = useCallback(
    (thread: ThreadRef, action: ThreadAction, done: string | null) => {
      const clientOpId = newClientId();
      const args = { clientOpId, threadIds: [thread._id] as Id<"threads">[], action };
      outbox.run("mail.apply", clientOpId, args, () => applyMutation(args)).catch(fail(clientOpId));
      haptics.tick();
      if (done) offerUndo(done, undoOps([clientOpId]));
    },
    [applyMutation, offerUndo, undoOps, fail],
  );

  const labelAction = useCallback(
    (thread: ThreadRef, labelId: string, add: boolean) => {
      const helpers = mail.actionsFor(thread.accountId);
      if (!helpers) return;
      single(
        thread,
        add ? helpers.addLabel(labelId) : helpers.removeLabel(labelId),
        add ? "Label added" : "Label removed",
      );
    },
    [mail, single],
  );

  const moveTo = useCallback(
    (thread: ThreadRef, folderKey: string, title: string) => {
      const helpers = mail.actionsFor(thread.accountId);
      if (!helpers?.available("moveTo")) return;
      single(thread, helpers.moveTo(folderKey), `Moved to ${title}`);
    },
    [mail, single],
  );

  const unsubscribe = useCallback(
    (thread: ThreadRef, method: "one_click_post" | "mailto") => {
      const helpers = mail.actionsFor(thread.accountId);
      if (!helpers) return;
      single(thread, helpers.unsubscribe(method), null);
      showToast({ id: `unsub:${thread._id}`, text: "Unsubscribe requested" });
    },
    [mail, single],
  );

  /**
   * Applies an action built per account to many threads (bulk label and move). Threads whose account cannot do
   * it (`build` returns null) are skipped; one Undo covers every call.
   */
  const applyEach = useCallback(
    (
      threads: readonly ThreadRef[],
      build: (h: ThreadActions, accountId: string) => ThreadAction | null,
      done: string,
    ) => {
      const byAccount = new Map<string, string[]>();
      for (const t of threads) byAccount.set(t.accountId, [...(byAccount.get(t.accountId) ?? []), t._id]);
      const clientOpIds: string[] = [];
      for (const [accountId, ids] of byAccount) {
        const helpers = mail.actionsFor(accountId);
        let action: ThreadAction | null = null;
        try {
          action = helpers ? build(helpers, accountId) : null;
        } catch {
          action = null;
        }
        if (!action) continue;
        for (let i = 0; i < ids.length; i += 500) {
          const clientOpId = newClientId();
          const args = { clientOpId, threadIds: ids.slice(i, i + 500) as Id<"threads">[], action };
          clientOpIds.push(clientOpId);
          outbox.run("mail.apply", clientOpId, args, () => applyMutation(args)).catch(fail(clientOpId));
        }
      }
      if (clientOpIds.length === 0) return 0;
      haptics.success();
      offerUndo(done, undoOps(clientOpIds));
      return clientOpIds.length;
    },
    [mail, applyMutation, offerUndo, undoOps, fail],
  );

  /** Adds a label (of one mailbox) to the threads of that mailbox. */
  const labelMany = useCallback(
    (threads: readonly ThreadRef[], labelId: string, labelAccountId: string) =>
      applyEach(
        threads.filter((t) => t.accountId === labelAccountId),
        (h) => h.addLabel(labelId),
        threads.length === 1 ? "Label added" : "Labels added",
      ),
    [applyEach],
  );

  /** Moves threads to a folder key (`sys:inbox` works for every account; other folders for their own). */
  const moveMany = useCallback(
    (threads: readonly ThreadRef[], folderKey: string, title: string, folderAccountId?: string) =>
      applyEach(
        folderAccountId ? threads.filter((t) => t.accountId === folderAccountId) : threads,
        (h) =>
          h.available("moveTo") ? h.moveTo(folderKey) : folderKey === "sys:inbox" ? h.moveToInbox() : null,
        `Moved to ${title}`,
      ),
    [applyEach],
  );

  /** The `z` key: undo the newest gesture that is still inside its window. */
  const undoLast = useCallback(() => {
    const last = lastUndo;
    if (!last || Date.now() - last.at > UNDO_MS) return false;
    last.run();
    return true;
  }, []);

  return { run, snooze, remind, labelAction, moveTo, labelMany, moveMany, unsubscribe, undoLast };
}
