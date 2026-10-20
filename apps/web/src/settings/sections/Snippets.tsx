// Settings: snippets (§3.2 `snippets`: user-authored templates, ≤ 20 KB, sanitized on write) and per-sender
// preferences (VIP, remote images, block). In the composer a snippet is inserted with Ctrl/⌘ ; or by typing
// `;shortcut` then Tab; `{{first_name}}`, `{{name}}`, `{{email}}`, `{{day}}` and `{{date}}` fill in from the first
// recipient and today, other variables stay for you to fill.
import { Button, errorMessage, useToast } from "@akatsuki/ui";
import { useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { SnippetView } from "../../backend/types.ts";
import { htmlToText, isEmail, textToHtml, variablesIn } from "../../compose/text.ts";
import { blockSender } from "../../senders/block.ts";
import { Empty, Row, Section, Switch } from "../ui.tsx";

const BODY_MAX = 20 * 1024;

export function SnippetsSection() {
  const backend = useBackend();
  const snippets = backend.useSnippets();
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const sorted = [...(snippets ?? [])].sort(
    (a, b) => b.useCount - a.useCount || a.name.localeCompare(b.name),
  );
  return (
    <Section
      title="Snippets"
      description="Reusable text. Insert with Ctrl or ⌘ ; in the composer, or type ;shortcut and press Tab. Variables like {{first_name}} fill in for you."
      actions={
        <Button variant="primary" onClick={() => setEditing("new")}>
          New snippet
        </Button>
      }
    >
      {editing === "new" && <SnippetForm onDone={() => setEditing(null)} />}
      {snippets === undefined ? null : sorted.length === 0 ? (
        <Empty>No snippets yet.</Empty>
      ) : (
        <ul className="ak-settings__cards">
          {sorted.map((s) =>
            editing === s._id ? (
              <li key={s._id}>
                <SnippetForm snippet={s} onDone={() => setEditing(null)} />
              </li>
            ) : (
              <li key={s._id} className="ak-settings__card ak-split">
                <div className="ak-split__text">
                  <span className="ak-settings__card-title">
                    {s.name}
                    {s.shortcut && <code className="ak-settings__code"> ;{s.shortcut}</code>}
                  </span>
                  <span className="ak-settings__hint">{htmlToText(s.bodyHtml).slice(0, 140)}</span>
                </div>
                <Button size="sm" onClick={() => setEditing(s._id)}>
                  Edit
                </Button>
              </li>
            ),
          )}
        </ul>
      )}
    </Section>
  );
}

function SnippetForm({ snippet, onDone }: { snippet?: SnippetView; onDone: () => void }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const [name, setName] = useState(snippet?.name ?? "");
  const [shortcut, setShortcut] = useState(snippet?.shortcut ?? "");
  const [subject, setSubject] = useState(snippet?.subject ?? "");
  const [body, setBody] = useState(snippet ? htmlToText(snippet.bodyHtml) : "");
  const [error, setError] = useState<string | null>(null);
  const vars = variablesIn(body);
  const save = async () => {
    const html = textToHtml(body);
    if (!name.trim()) return setError("Give it a name.");
    if (!body.trim()) return setError("Write the snippet text.");
    if (shortcut && !/^[a-z0-9_-]{1,32}$/i.test(shortcut))
      return setError("Shortcuts use letters, digits, - and _ only.");
    if (html.length > BODY_MAX) return setError("Snippets are limited to 20 KB.");
    try {
      await m.saveSnippet({
        ...(snippet && { snippetId: snippet._id }),
        name: name.trim(),
        ...(shortcut.trim() && { shortcut: shortcut.trim().toLowerCase() }),
        ...(subject.trim() && { subject: subject.trim() }),
        bodyHtml: html,
        variables: vars,
      });
      toast({ title: `Snippet “${name.trim()}” saved.` });
      onDone();
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return (
    <form
      className="ak-settings__card ak-rule-form"
      aria-label={snippet ? `Edit snippet ${snippet.name}` : "New snippet"}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <Row label="Name" htmlFor="snp-name">
        <input
          id="snp-name"
          className="ak-input"
          value={name}
          // biome-ignore lint/a11y/noAutofocus: the first field of a form the user just opened
          autoFocus
          onChange={(e) => setName(e.target.value)}
        />
      </Row>
      <Row label="Shortcut" htmlFor="snp-short" hint="Type ;shortcut then Tab in a message.">
        <input
          id="snp-short"
          className="ak-input"
          value={shortcut}
          onChange={(e) => setShortcut(e.target.value)}
        />
      </Row>
      <Row label="Subject" htmlFor="snp-subject" hint="Optional; used when the message has no subject yet.">
        <input
          id="snp-subject"
          className="ak-input"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
        />
      </Row>
      <label className="ak-field__label" htmlFor="snp-body">
        Text
      </label>
      <textarea
        id="snp-body"
        className="ak-textarea"
        rows={6}
        value={body}
        onChange={(e) => setBody(e.target.value)}
      />
      {vars.length > 0 && (
        <p className="ak-settings__hint">Variables: {vars.map((v) => `{{${v}}}`).join(", ")}</p>
      )}
      {error && (
        <p className="ak-compose__error" role="alert">
          {error}
        </p>
      )}
      <div className="ak-settings__actions">
        <Button type="submit" variant="primary">
          Save snippet
        </Button>
        {snippet && (
          <Button
            variant="ghost"
            onClick={() =>
              void m
                .removeSnippet({ snippetId: snippet._id })
                .then(onDone)
                .catch((e) => setError(errorMessage(e)))
            }
          >
            Delete
          </Button>
        )}
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

const SENDER_RE = /^(@[^\s@]+\.[^\s@]+)$/;

export function SendersSection() {
  const backend = useBackend();
  const m = backend.useMutations();
  const rules = backend.useRules();
  const { toast } = useToast();
  const [input, setInput] = useState("");
  const sender = input.trim().toLowerCase();
  const valid = isEmail(sender) || SENDER_RE.test(sender);
  const pref = backend.useSenderPrefFull(valid ? sender : null);
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  const blockedRules = (rules ?? []).filter((r) => r.name.startsWith("Blocked: "));
  return (
    <Section
      title="Senders"
      description="VIPs land in the VIP split and can be used in rules. Images from trusted senders load automatically. Blocked senders go straight to Trash."
    >
      <Row label="Address or domain" htmlFor="sender-q" hint="For example mara@example.com or @example.com.">
        <input
          id="sender-q"
          className="ak-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          aria-invalid={input && !valid ? true : undefined}
        />
      </Row>
      {valid && pref !== undefined && (
        <div className="ak-settings__card">
          <Row label="VIP">
            <Switch
              label="VIP"
              checked={pref?.vip ?? false}
              onChange={(vip) => void m.setSenderPref({ sender, vip }).catch(fail)}
            />
          </Row>
          <Row label="Always load images">
            <Switch
              label="Always load images"
              checked={pref?.allowRemoteImages ?? false}
              onChange={(allowRemoteImages) =>
                void m.setSenderPref({ sender, allowRemoteImages }).catch(fail)
              }
            />
          </Row>
          <Row label="Blocked" hint="New mail from this sender goes to Trash.">
            <Switch
              label="Blocked"
              checked={pref?.blocked ?? false}
              onChange={(blocked) =>
                void blockSender(sender, blocked, { m, rules: rules ?? [] })
                  .then(() => toast({ title: blocked ? `Blocked ${sender}.` : `Unblocked ${sender}.` }))
                  .catch(fail)
              }
            />
          </Row>
        </div>
      )}
      {blockedRules.length > 0 && (
        <>
          <h3 className="ak-ai__sub">Blocked</h3>
          <ul className="ak-settings__cards">
            {blockedRules.map((r) => {
              const who = r.name.slice("Blocked: ".length);
              return (
                <li key={r._id} className="ak-settings__card ak-split">
                  <span className="ak-split__text">{who}</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      void blockSender(who, false, { m, rules: rules ?? [] })
                        .then(() => toast({ title: `Unblocked ${who}.` }))
                        .catch(fail)
                    }
                  >
                    Unblock
                  </Button>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </Section>
  );
}
