// Composer state shared by the overlay, the shortcuts (c, r, a, f, z) and the palette: which draft is open,
// whether the composer owns the keyboard, replies waiting for their thread, and sends inside their undo window.
import { useSyncExternalStore } from "react";
import { notifyExtensions } from "../app/extensions.ts";
import type { ThreadId } from "../backend/types.ts";
import type { LocalDraft } from "./model.ts";

export interface PendingSend {
  clientSendId: string;
  draft: LocalDraft;
  kind: "undo" | "later";
  sendAt: number;
  /** Undo is offered until this instant (the undo window, or 10 s after scheduling a later send). */
  undoUntil: number;
}

export interface ComposerState {
  draft: LocalDraft | null;
  /** Bumped when another draft replaces the open one (the editor remounts). */
  session: number;
  /** Focus is inside the composer: its keys (compose scope) win over the mail list's. */
  focused: boolean;
  /** A reply, reply-all or forward waiting for its thread to load. */
  pending: { threadId: ThreadId; mode: "reply" | "reply_all" | "forward"; body?: string } | null;
  /** A new message waiting for the accounts to load (deep link, `c` on a cold start). */
  pendingNew: { to?: string; subject?: string; body?: string } | null;
  sends: PendingSend[];
  /** One-shot requests to the open composer (insert text from an AI reply, open the snippet picker). */
  request: { kind: "snippet" | "ai_write" | "send_later" | "insert"; text?: string; seq: number } | null;
}

let state: ComposerState = {
  draft: null,
  session: 0,
  focused: false,
  pending: null,
  pendingNew: null,
  sends: [],
  request: null,
};
const listeners = new Set<() => void>();
let seq = 0;

function set(patch: Partial<ComposerState>) {
  state = { ...state, ...patch };
  for (const l of [...listeners]) l();
  // The mail shell re-reads extension commands (compose shortcuts, scopes) on change.
  notifyExtensions();
}

export const composer = {
  get: () => state,
  subscribe(cb: () => void) {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  },
  open(draft: LocalDraft) {
    set({ draft, session: state.session + 1, pending: null, pendingNew: null, request: null });
  },
  /** Keeps the open draft object in sync with the editor (no remount). */
  update(draft: LocalDraft) {
    if (state.draft?.clientDraftId === draft.clientDraftId) state = { ...state, draft };
  },
  close() {
    set({ draft: null, focused: false, request: null });
  },
  requestReply(threadId: ThreadId, mode: "reply" | "reply_all" | "forward", body?: string) {
    set({ pending: { threadId, mode, ...(body !== undefined && { body }) } });
  },
  requestNew(p: { to?: string; subject?: string; body?: string } = {}) {
    set({ pendingNew: p });
  },
  clearPending() {
    set({ pending: null, pendingNew: null });
  },
  setFocused(focused: boolean) {
    if (state.focused !== focused) set({ focused });
  },
  ask(kind: NonNullable<ComposerState["request"]>["kind"], text?: string) {
    set({ request: { kind, seq: ++seq, ...(text !== undefined && { text }) } });
  },
  addSend(s: PendingSend) {
    set({ sends: [...state.sends.filter((x) => x.clientSendId !== s.clientSendId), s] });
  },
  removeSend(clientSendId: string) {
    if (state.sends.some((s) => s.clientSendId === clientSendId))
      set({ sends: state.sends.filter((s) => s.clientSendId !== clientSendId) });
  },
  /** The newest send that can still be undone. */
  undoable(now = Date.now()): PendingSend | null {
    const live = state.sends.filter((s) => s.undoUntil > now);
    return live.length ? live[live.length - 1]! : null;
  },
  /** Test seam. */
  reset() {
    state = {
      draft: null,
      session: 0,
      focused: false,
      pending: null,
      pendingNew: null,
      sends: [],
      request: null,
    };
    for (const l of [...listeners]) l();
  },
};

export function useComposer(): ComposerState {
  return useSyncExternalStore(composer.subscribe, composer.get, composer.get);
}
