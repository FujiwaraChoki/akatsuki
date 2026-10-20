// Settings: labels (§4.2 labels.create / labels.rename / labels.remove). Gmail labels, Outlook categories and IMAP
// keywords the user made; system labels and provider folders are listed by the sidebar but not edited here. Each
// change is an op the worker applies to the provider (§5); a failure rolls back with a toast.
import { Button, errorMessage, useToast } from "@akatsuki/ui";
import { useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { AccountId, LabelView, MailAccountView } from "../../backend/types.ts";
import { uuidv7 } from "../../lib/ids.ts";
import { visibleLabels } from "../../mail/Sidebar.tsx";
import { Empty, Row, Section } from "../ui.tsx";

/** Labels the user can rename or delete: not system labels, roles or folders. */
export function editableLabels(labels: readonly LabelView[]): LabelView[] {
  return visibleLabels(labels).filter((l) => !l.role && !l.exclusive);
}

export function LabelsSection() {
  const backend = useBackend();
  const accounts = backend.useAccounts() ?? [];
  const labels = backend.useLabels();
  const [creating, setCreating] = useState(false);
  const live = accounts.filter((a) => a.status !== "removing");
  return (
    <Section
      title="Labels"
      description="Labels you made, per mailbox. Outlook calls them categories and IMAP servers keywords. Changes go to your provider too. You can also create a label from the label picker (l) by typing a new name."
      actions={
        live.length > 0 && (
          <Button variant="primary" onClick={() => setCreating(true)}>
            New label
          </Button>
        )
      }
    >
      {creating && <NewLabelForm accounts={live} onDone={() => setCreating(false)} />}
      {labels === undefined ? null : live.length === 0 ? (
        <Empty>Connect a mailbox to use labels.</Empty>
      ) : (
        live.map((a) => {
          const mine = editableLabels(labels.filter((l) => l.accountId === a._id));
          return (
            <div key={a._id} className="ak-settings__group">
              {live.length > 1 && <h3 className="ak-settings__group-title">{a.email}</h3>}
              {mine.length === 0 ? (
                <Empty>No labels yet.</Empty>
              ) : (
                <ul className="ak-settings__cards" aria-label={`Labels of ${a.email}`}>
                  {mine.map((l) => (
                    <LabelRow key={l._id} label={l} />
                  ))}
                </ul>
              )}
            </div>
          );
        })
      )}
    </Section>
  );
}

function NewLabelForm({ accounts, onDone }: { accounts: MailAccountView[]; onDone: () => void }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const [name, setName] = useState("");
  const [accountId, setAccountId] = useState<AccountId>(accounts[0]!._id);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    const n = name.trim();
    if (!n) return setError("Give it a name.");
    try {
      await m.createLabel({ clientOpId: uuidv7(), accountId, name: n });
      toast({ title: `Label “${n}” created.` });
      onDone();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <form
      className="ak-settings__card ak-rule-form"
      aria-label="New label"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <Row label="Name" htmlFor="lbl-new-name">
        <input
          id="lbl-new-name"
          className="ak-input"
          value={name}
          maxLength={225}
          // biome-ignore lint/a11y/noAutofocus: the first field of a form the user just opened
          autoFocus
          onChange={(e) => {
            setName(e.target.value);
            setError(null);
          }}
        />
      </Row>
      {accounts.length > 1 && (
        <Row label="Mailbox" htmlFor="lbl-new-account">
          <select
            id="lbl-new-account"
            className="ak-input ak-input--select"
            value={accountId}
            onChange={(e) => setAccountId(e.target.value as AccountId)}
          >
            {accounts.map((a) => (
              <option key={a._id} value={a._id}>
                {a.email}
              </option>
            ))}
          </select>
        </Row>
      )}
      {error && (
        <p className="ak-compose__error" role="alert">
          {error}
        </p>
      )}
      <div className="ak-settings__actions">
        <Button variant="primary" type="submit">
          Create label
        </Button>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function LabelRow({ label }: { label: LabelView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const [mode, setMode] = useState<"view" | "rename" | "delete">("view");
  const [name, setName] = useState(label.name);
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  const rename = () => {
    const n = name.trim();
    if (!n || n === label.name) return setMode("view");
    setMode("view");
    void m
      .renameLabel({ clientOpId: uuidv7(), labelId: label._id, name: n })
      .then(() => toast({ title: `Renamed to “${n}”.` }))
      .catch(fail);
  };
  if (mode === "rename")
    return (
      <li className="ak-settings__card">
        <form
          className="ak-split"
          aria-label={`Rename ${label.name}`}
          onSubmit={(e) => {
            e.preventDefault();
            rename();
          }}
        >
          <input
            className="ak-input"
            aria-label={`New name for ${label.name}`}
            value={name}
            maxLength={225}
            // biome-ignore lint/a11y/noAutofocus: the field the user just asked to edit
            autoFocus
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setName(label.name);
                setMode("view");
              }
            }}
          />
          <Button size="sm" variant="primary" type="submit">
            Save
          </Button>
        </form>
      </li>
    );
  return (
    <li className="ak-settings__card ak-split">
      <div className="ak-split__text">
        <span className="ak-settings__card-title">{label.name}</span>
        {mode === "delete" && (
          <span className="ak-settings__hint">
            Deleting removes it from every conversation, here and at your provider. The mail stays.
          </span>
        )}
      </div>
      {mode === "delete" ? (
        <>
          <Button
            size="sm"
            variant="danger"
            onClick={() =>
              void m
                .removeLabel({ clientOpId: uuidv7(), labelId: label._id })
                .then(() => toast({ title: `Deleted “${label.name}”.` }))
                .catch(fail)
            }
          >
            Delete {label.name}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setMode("view")}>
            Keep it
          </Button>
        </>
      ) : (
        <>
          <Button size="sm" onClick={() => setMode("rename")} aria-label={`Rename ${label.name}`}>
            Rename
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setMode("delete")}
            aria-label={`Delete ${label.name}`}
          >
            Delete
          </Button>
        </>
      )}
    </li>
  );
}
