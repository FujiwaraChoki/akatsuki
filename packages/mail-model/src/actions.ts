// Client action helpers (ARCHITECTURE §5.1): the only way UIs build ThreadActions. The same gesture maps to
// different actions on label providers (Gmail) and folder providers (Graph, IMAP):
//
// | Helper                    | Gmail                                   | Graph / IMAP                          |
// |---------------------------|-----------------------------------------|---------------------------------------|
// | archive()                 | modify{remove:[sys:inbox]}              | move{to: l:<role "archive" folder>}   |
// | moveToInbox()             | modify{add:[sys:inbox]}                 | move{to: sys:inbox}                   |
// | markRead() / markUnread() | modify{remove/add:[sys:unread]}         | same                                  |
// | star() / unstar()         | modify{add/remove:[sys:starred]}        | same                                  |
// | spam() / notSpam()        | modify{add:[spam],remove:[inbox]} / inv | move{to: sys:spam} / move{to: inbox}  |
// | trash() / untrash()       | trash / untrash                         | trash / untrash                       |
// | deleteForever()           | unavailable (no scope)                  | delete_forever (caps.deleteForever)   |
// | addLabel(id)/removeLabel  | modify with l:<id>                      | categories/keywords: modify;          |
// |                           |                                         | folders: move{to: l:<id>} / inbox     |
// | snooze()                  | modify{add:[sys:snoozed],remove:[inbox]}| move{to: sys:snoozed}                 |
// | unsubscribe(method)       | unsubscribe                             | same                                  |
import { isFolderProvider } from "./applyOps.ts";
import { SYS } from "./labelKeys.ts";
import type { Provider, ThreadAction, UnsubscribeMethod } from "./types.ts";

/** The account fields the helpers need (a `mail.getThread().account` or `accounts.list` row fits). */
export interface ActionAccount {
  provider: Provider;
  caps?: { deleteForever?: boolean };
}

/** The label fields the helpers need (a `labels.list` row fits). */
export interface ActionLabel {
  _id: string;
  /** Folder semantics (Graph/IMAP folders). */
  exclusive: boolean;
  role?: string;
}

/** Thrown when an action does not exist for the account. `code` is a §15 error code. */
export class ActionUnavailableError extends Error {
  override name = "ActionUnavailableError";
  readonly code: "unsupported" | "invalid_action";
  constructor(code: "unsupported" | "invalid_action", message: string) {
    super(message);
    this.code = code;
  }
}

export type ActionName =
  | "archive"
  | "moveToInbox"
  | "markRead"
  | "markUnread"
  | "star"
  | "unstar"
  | "spam"
  | "notSpam"
  | "trash"
  | "untrash"
  | "deleteForever"
  | "snooze"
  | "addLabel"
  | "removeLabel"
  | "moveTo"
  | "unsubscribe";

export interface ThreadActions {
  readonly provider: Provider;
  /** Folder provider (Graph, IMAP): location changes are `move`s. */
  readonly folders: boolean;
  /** Whether `name` can be built for this account (e.g. archive needs an archive folder on Graph/IMAP). */
  available(name: ActionName): boolean;
  archive(): ThreadAction;
  moveToInbox(): ThreadAction;
  markRead(): ThreadAction;
  markUnread(): ThreadAction;
  /** `markRead()` when `isUnread`, else `markUnread()` (the `u` key). */
  toggleRead(isUnread: boolean): ThreadAction;
  star(): ThreadAction;
  unstar(): ThreadAction;
  /** `unstar()` when `isStarred`, else `star()` (the `s` key). */
  toggleStar(isStarred: boolean): ThreadAction;
  spam(): ThreadAction;
  notSpam(): ThreadAction;
  trash(): ThreadAction;
  untrash(): ThreadAction;
  deleteForever(): ThreadAction;
  /** The provider-side part of a snooze; `mail.snooze` builds the same action on the server. */
  snooze(): ThreadAction;
  addLabel(labelId: string): ThreadAction;
  removeLabel(labelId: string): ThreadAction;
  /** Move to a folder key (`sys:inbox`, `sys:spam`, `l:<folder>`). Folder providers only. */
  moveTo(folderKey: string): ThreadAction;
  unsubscribe(method: UnsubscribeMethod): ThreadAction;
}

const modify = (add: string[], remove: string[]): ThreadAction => ({ kind: "modify", add, remove });
const move = (to: string): ThreadAction => ({ kind: "move", to });

/**
 * Action helpers for one account. `labels` are that account's labels (needed on folder providers to find
 * the archive folder and to tell folders from categories/keywords).
 */
export function actionsFor(account: ActionAccount, labels: readonly ActionLabel[] = []): ThreadActions {
  const folders = isFolderProvider(account.provider);
  const archiveFolder = labels.find((l) => l.role === "archive");
  const byId = new Map(labels.map((l) => [l._id, l]));
  const canDeleteForever = account.caps?.deleteForever ?? folders;
  const isFolder = (labelId: string) => folders && byId.get(labelId)?.exclusive === true;

  const helpers: ThreadActions = {
    provider: account.provider,
    folders,
    available(name) {
      if (name === "archive") return !folders || archiveFolder !== undefined;
      if (name === "deleteForever") return canDeleteForever;
      if (name === "moveTo") return folders;
      return true;
    },
    archive() {
      if (!folders) return modify([], [SYS.inbox]);
      if (!archiveFolder)
        throw new ActionUnavailableError("invalid_action", "This account has no archive folder yet.");
      return move(`l:${archiveFolder._id}`);
    },
    moveToInbox: () => (folders ? move(SYS.inbox) : modify([SYS.inbox], [])),
    markRead: () => modify([], [SYS.unread]),
    markUnread: () => modify([SYS.unread], []),
    toggleRead: (isUnread) => (isUnread ? modify([], [SYS.unread]) : modify([SYS.unread], [])),
    star: () => modify([SYS.starred], []),
    unstar: () => modify([], [SYS.starred]),
    toggleStar: (isStarred) => (isStarred ? modify([], [SYS.starred]) : modify([SYS.starred], [])),
    spam: () => (folders ? move(SYS.spam) : modify([SYS.spam], [SYS.inbox])),
    notSpam: () => (folders ? move(SYS.inbox) : modify([SYS.inbox], [SYS.spam])),
    trash: () => ({ kind: "trash" }),
    untrash: () => ({ kind: "untrash" }),
    deleteForever() {
      if (!canDeleteForever)
        throw new ActionUnavailableError(
          "unsupported",
          "Permanent delete is not available for this account.",
        );
      return { kind: "delete_forever" };
    },
    snooze: () => (folders ? move(SYS.snoozed) : modify([SYS.snoozed], [SYS.inbox])),
    addLabel: (labelId) => (isFolder(labelId) ? move(`l:${labelId}`) : modify([`l:${labelId}`], [])),
    removeLabel: (labelId) => (isFolder(labelId) ? move(SYS.inbox) : modify([], [`l:${labelId}`])),
    moveTo(folderKey) {
      if (!folders)
        throw new ActionUnavailableError("invalid_action", "Move is only for folder-based accounts.");
      return move(folderKey);
    },
    unsubscribe: (method) => ({ kind: "unsubscribe", method }),
  };
  return helpers;
}
