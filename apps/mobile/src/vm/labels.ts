// Labels management on mobile (ARCHITECTURE §4.2 `labels.create` / `labels.rename` / `labels.remove`, D-417
// follow-up): the labels the user made, per mailbox (Gmail labels, Outlook categories, IMAP keywords). System
// labels, role folders and provider folders are not edited here, the same rule as web's `editableLabels`. Each
// change is an op the worker applies at the provider (§5); a failure arrives as an op-failure toast.

export interface LabelLike {
  _id: string;
  accountId: string;
  name: string;
  labelKey: string;
  kind?: string;
  role?: string;
  exclusive: boolean;
  hidden?: boolean;
  pendingDelete?: boolean;
}

/** `labels.create` / `labels.rename` accept at most 225 characters (Gmail's limit). */
export const LABEL_NAME_MAX = 225;

/** Labels the user can rename or delete: not system labels, roles or folders; not hidden or being deleted. */
export function editableLabels<T extends LabelLike>(labels: readonly T[]): T[] {
  return labels
    .filter(
      (l) =>
        l.labelKey.startsWith("l:") &&
        l.kind !== "system" &&
        !l.role &&
        !l.exclusive &&
        !l.hidden &&
        !l.pendingDelete,
    )
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Folders the user made (not system or role folders). They can be renamed here, but not deleted: deleting a
 * folder at an IMAP or Outlook provider deletes the mail in it, so that stays a deliberate act at the provider.
 * On IMAP servers without keywords a new label is created as a folder, so it shows up here.
 */
export function renamableFolders<T extends LabelLike>(labels: readonly T[]): T[] {
  return labels
    .filter(
      (l) =>
        l.labelKey.startsWith("l:") &&
        l.kind !== "system" &&
        !l.role &&
        l.exclusive &&
        !l.hidden &&
        !l.pendingDelete &&
        !l.name.toLowerCase().startsWith("akatsuki/"),
    )
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Editable labels grouped by mailbox, in the order of `accounts`; mailboxes being removed are left out. */
export function labelsByAccount<
  T extends LabelLike,
  A extends { _id: string; email: string; status?: string },
>(labels: readonly T[], accounts: readonly A[]): { account: A; labels: T[]; folders: T[] }[] {
  return accounts
    .filter((a) => a.status !== "removing")
    .map((account) => {
      const mine = labels.filter((l) => l.accountId === account._id);
      return { account, labels: editableLabels(mine), folders: renamableFolders(mine) };
    });
}

/** Why a new or renamed label name cannot be saved, or null. */
export function labelNameProblem(
  name: string,
  siblings: readonly Pick<LabelLike, "_id" | "name">[],
  selfId?: string,
): string | null {
  const n = name.trim();
  if (!n) return "Give it a name.";
  if (n.length > LABEL_NAME_MAX) return "Label names are at most 225 characters.";
  if (siblings.some((l) => l._id !== selfId && l.name.trim().toLowerCase() === n.toLowerCase()))
    return "This mailbox already has a label with that name.";
  return null;
}

/** The confirmation shown before `labels.remove`. */
export function deleteLabelMessage(name: string): { title: string; body: string } {
  return {
    title: `Delete “${name}”?`,
    body: "It is removed from every conversation, here and at your provider. The mail itself stays.",
  };
}
