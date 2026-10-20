// Thread actions with optimistic updates, undo and the durable outbox (ARCHITECTURE §5.1, §5.2, §16.1).
//
// One gesture = one call per (account, action) group, each with its own UUIDv7 `clientOpId` (an action differs
// per provider: archive is `modify` on Gmail and `move` on folder accounts), chunked at 500 threads. The
// gesture remembers its clientOpIds for 10 s so `z` (or the toast's Undo) reverses all of them.
import {
  type ActionName,
  ActionUnavailableError,
  actionsFor,
  applyAction,
  applyContextFor,
  diffKeys,
  isFolderProvider,
  type ThreadAction,
  type ThreadActions,
} from "@akatsuki/mail-model";
import { viewContains } from "@akatsuki/mail-model/optimistic";
import { actionVerb, describeOpFailure, errorMessage, UNDO_TOAST_MS, useToast } from "@akatsuki/ui";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { useBackend } from "../backend/context.tsx";
import type { LabelView, ListViewArgs, MailAccountView, ThreadId, ThreadRow } from "../backend/types.ts";
import { uuidv7 } from "../lib/ids.ts";
import { Outbox } from "../lib/outbox.ts";
import { rememberUndo, snapshotFor } from "./undoSnapshots.ts";

const MAX_PER_CALL = 500;

export interface Gesture {
  kind: "apply" | "snooze";
  clientOpIds: string[];
  threadIds: ThreadId[];
  at: number;
  /** Where to return on undo (the thread that auto-advanced away). */
  restore?: string;
}

export type BuildAction = (a: ThreadActions, row: ThreadRow) => ThreadAction;

const outboxes = new WeakMap<object, Outbox>();

export interface MailActionsDeps {
  accounts: readonly MailAccountView[];
  labels: readonly LabelView[];
}

export function useMailActions({ accounts, labels }: MailActionsDeps) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast, dismiss } = useToast();
  const last = useRef<Gesture | null>(null);
  const opVerbs = useRef(new Map<string, string>());

  const outbox = useMemo(() => {
    let o = outboxes.get(backend);
    if (!o) {
      o = new Outbox();
      outboxes.set(backend, o);
    }
    return o;
  }, [backend]);

  // Replay anything a previous session could not send (§16.1). clientOpId dedupe makes this safe.
  useEffect(() => {
    void outbox.replay(m);
  }, [outbox, m]);

  const accountOf = useCallback((id: string) => accounts.find((a) => a._id === id), [accounts]);
  const labelsOf = useCallback((id: string) => labels.filter((l) => l.accountId === id), [labels]);
  const contextFor = useCallback(
    (accountId: string) => {
      const a = accountOf(accountId);
      const exclusive = labelsOf(accountId)
        .filter((l) => l.exclusive && l.kind !== "system")
        .map((l) => l._id);
      return applyContextFor(a?.provider ?? "gmail", exclusive);
    },
    [accountOf, labelsOf],
  );
  const helpersFor = useCallback(
    (accountId: string): ThreadActions | null => {
      const a = accountOf(accountId);
      return a ? actionsFor(a, labelsOf(accountId)) : null;
    },
    [accountOf, labelsOf],
  );

  /** Whether `name` is possible for every row (e.g. archive needs an archive folder on IMAP). */
  const available = useCallback(
    (name: ActionName, rows: readonly ThreadRow[]) =>
      rows.length > 0 && rows.every((r) => helpersFor(r.accountId)?.available(name) ?? false),
    [helpersFor],
  );

  /** Predicts which rows leave `view` (for cursor movement and auto-advance), before anything is sent. */
  const leaving = useCallback(
    (rows: readonly ThreadRow[], build: BuildAction, view: ListViewArgs | null): Set<string> => {
      const out = new Set<string>();
      if (!view) return out;
      for (const r of rows) {
        const h = helpersFor(r.accountId);
        if (!h) continue;
        try {
          const after = applyAction(
            { keys: r.labelKeys, deleted: false },
            build(h, r),
            contextFor(r.accountId),
          );
          if (
            !viewContains(view, after, {
              accountId: r.accountId,
              split: after.keys.includes("sys:inbox") ? r.split : undefined,
            })
          )
            out.add(r._id);
        } catch {
          /* unavailable: nothing moves */
        }
      }
      return out;
    },
    [helpersFor, contextFor],
  );

  const showUndo = useCallback(
    (g: Gesture, title: string) => {
      last.current = g;
      toast({
        id: "undo",
        title,
        durationMs: UNDO_TOAST_MS,
        action: { label: "Undo", shortcut: "undo", onAction: () => void undoRef.current?.() },
      });
    },
    [toast],
  );

  const fail = useCallback((err: unknown) => toast({ title: errorMessage(err), tone: "error" }), [toast]);

  /**
   * Applies `build` to each row (grouped per account and resulting action) and shows an Undo toast with
   * `title(n)`. Returns the gesture, or null when nothing was sent.
   */
  const perform = useCallback(
    (
      rows: readonly ThreadRow[],
      build: BuildAction,
      title: ((n: number) => string) | null,
      opts: { restore?: string } = {},
    ): Gesture | null => {
      const groups = new Map<string, { action: ThreadAction; rows: ThreadRow[] }>();
      for (const r of rows) {
        const h = helpersFor(r.accountId);
        if (!h) continue;
        let action: ThreadAction;
        try {
          action = build(h, r);
        } catch (e) {
          if (e instanceof ActionUnavailableError) {
            toast({ title: e.message, tone: "neutral" });
            continue;
          }
          throw e;
        }
        const key = `${r.accountId}|${JSON.stringify(action)}`;
        const g = groups.get(key);
        if (g) g.rows.push(r);
        else groups.set(key, { action, rows: [r] });
      }
      if (groups.size === 0) return null;
      const gesture: Gesture = {
        kind: "apply",
        clientOpIds: [],
        threadIds: rows.map((r) => r._id),
        at: Date.now(),
        ...opts,
      };
      for (const { action, rows: groupRows } of groups.values()) {
        for (let i = 0; i < groupRows.length; i += MAX_PER_CALL) {
          const chunk = groupRows.slice(i, i + MAX_PER_CALL);
          const clientOpId = uuidv7();
          gesture.clientOpIds.push(clientOpId);
          rememberUndo(
            clientOpId,
            snapshotFor(
              chunk,
              action,
              (id) => accountOf(id)?.provider ?? "gmail",
              contextFor,
              (keys, a, ctx) => applyAction({ keys, deleted: false }, a, ctx).keys,
              diffKeys,
            ),
          );
          const args = { clientOpId, threadIds: chunk.map((r) => r._id), action };
          outbox
            .run("apply", clientOpId, args, () => m.apply(args))
            .then((r) => {
              const opIds = (r as { opIds?: string[] } | undefined)?.opIds ?? [];
              for (const id of opIds) opVerbs.current.set(id, actionVerb(action));
            })
            .catch(fail);
        }
      }
      if (title) showUndo(gesture, title(rows.length));
      return gesture;
    },
    [helpersFor, accountOf, contextFor, outbox, m, fail, showUndo, toast],
  );

  const snooze = useCallback(
    (
      rows: readonly ThreadRow[],
      until: number,
      returnUnread: boolean,
      title: string,
      opts: { restore?: string } = {},
    ) => {
      if (rows.length === 0) return null;
      const gesture: Gesture = {
        kind: "snooze",
        clientOpIds: [],
        threadIds: rows.map((r) => r._id),
        at: Date.now(),
        ...opts,
      };
      for (let i = 0; i < rows.length; i += MAX_PER_CALL) {
        const clientOpId = uuidv7();
        gesture.clientOpIds.push(clientOpId);
        const args = {
          clientOpId,
          threadIds: rows.slice(i, i + MAX_PER_CALL).map((r) => r._id),
          until,
          returnUnread,
        };
        outbox.run("snooze", clientOpId, args, () => m.snooze(args)).catch(fail);
      }
      showUndo(gesture, title);
      return gesture;
    },
    [outbox, m, fail, showUndo],
  );

  const undoRef = useRef<(() => Promise<Gesture | null>) | null>(null);
  /** Reverses the last gesture if it is less than 10 s old. Returns it (for navigation), or null. */
  const undoLast = useCallback(async (): Promise<Gesture | null> => {
    const g = last.current;
    if (!g || Date.now() - g.at > UNDO_TOAST_MS) return null;
    last.current = null;
    dismiss("undo");
    try {
      if (g.kind === "snooze") {
        const clientOpId = uuidv7();
        const args = { clientOpId, threadIds: g.threadIds };
        await outbox.run("unsnooze", clientOpId, args, () => m.unsnooze(args));
      } else
        await Promise.all(
          g.clientOpIds.map((undoClientOpId) => {
            const clientOpId = uuidv7();
            const args = { clientOpId, undoClientOpId };
            return outbox.run("undo", clientOpId, args, () => m.undo(args));
          }),
        );
      toast({ title: "Undone.", durationMs: 2000 });
    } catch (e) {
      fail(e);
    }
    return g;
  }, [dismiss, outbox, m, toast, fail]);
  undoRef.current = undoLast;

  // Op failures (§5.7): one toast per opId with Retry and Dismiss. The thread already rolled back.
  const failures = backend.useOpFailures();
  const shown = useRef(new Set<string>());
  useEffect(() => {
    for (const f of failures ?? []) {
      if (shown.current.has(f._id)) continue;
      shown.current.add(f._id);
      const verb =
        opVerbs.current.get(f._id) ??
        (f.payloadKind === "thread" ? "update" : f.payloadKind.replace(/_/g, " "));
      toast({
        id: f._id,
        tone: "error",
        title: describeOpFailure({ verb, subject: f.subject, account: f.accountEmail, code: f.error.code }),
        action:
          f.payloadKind !== "send"
            ? {
                label: "Retry",
                onAction: () => void m.retryOp({ opId: f._id, clientOpId: uuidv7() }).catch(fail),
              }
            : undefined,
        secondaryAction: {
          label: "Dismiss",
          onAction: () => void m.ackFailure({ opIds: [f._id] }).catch(() => undefined),
        },
        onDismiss: () => void m.ackFailure({ opIds: [f._id] }).catch(() => undefined),
      });
    }
  }, [failures, toast, m, fail]);

  const setReminder = useCallback(
    (row: ThreadRow, at: number) =>
      m
        .setReminder({ threadId: row._id, at, condition: "if_no_reply" })
        .then(() => toast({ title: "Reminder set. It comes back if nobody replies." }))
        .catch(fail),
    [m, toast, fail],
  );

  return {
    perform,
    snooze,
    undoLast,
    leaving,
    available,
    helpersFor,
    setReminder,
    contextFor,
    isFolder: (id: string) => isFolderProvider(accountOf(id)?.provider ?? "gmail"),
  };
}

export type MailActions = ReturnType<typeof useMailActions>;
