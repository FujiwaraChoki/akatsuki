// Settings: connected mailboxes (name, history window, reconnect, remove) and identities (send-as addresses
// with their name, reply-to and signature). Signatures are edited as plain text and stored as simple HTML;
// Convex sanitizes it on write and the worker adds it to outgoing mail (§3.3, §7.3).
import { Badge, Button, errorMessage, useToast } from "@akatsuki/ui";
import { useState } from "react";
import { navigate } from "../../app/router.tsx";
import { useBackend } from "../../backend/context.tsx";
import type { IdentityView, MailAccountView } from "../../backend/types.ts";
import { htmlToText, textToHtml } from "../../compose/text.ts";
import { fullDate } from "../../lib/format.ts";
import { Empty, Row, Section, Switch } from "../ui.tsx";

const PROVIDER: Record<MailAccountView["provider"], string> = {
  gmail: "Gmail",
  graph: "Outlook",
  imap: "IMAP",
};
const STATUS: Record<MailAccountView["status"], string> = {
  connecting: "Connecting",
  initial_sync: "Syncing",
  live: "Up to date",
  degraded: "Sync delayed",
  reauth_required: "Needs reconnecting",
  paused: "Paused",
  removing: "Removing",
};
const DAY = 86_400_000;

export function AccountsSection() {
  const backend = useBackend();
  const accounts = backend.useAccounts();
  return (
    <Section
      title="Accounts"
      description="Akatsuki syncs headers and short previews. Message bodies stay with your provider and on your devices."
      actions={
        <Button variant="primary" onClick={() => navigate("/onboarding")}>
          Connect a mailbox
        </Button>
      }
    >
      {accounts === undefined ? null : accounts.length === 0 ? (
        <Empty>No mailbox connected yet.</Empty>
      ) : (
        <ul className="ak-settings__cards">
          {accounts.map((a) => (
            <AccountCard key={a._id} account={a} />
          ))}
        </ul>
      )}
    </Section>
  );
}

function AccountCard({ account: a }: { account: MailAccountView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const me = backend.useMe();
  const { toast } = useToast();
  const [name, setName] = useState(a.displayName ?? "");
  const [confirm, setConfirm] = useState(false);
  const tz = me?.profile.timezone ?? "UTC";
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  const horizon =
    a.backfillHorizonAt === 0 ? "all" : a.createdAt - a.backfillHorizonAt >= 300 * DAY ? "1y" : "90d";
  const reconnect = async () => {
    const provider = a.provider === "gmail" ? "google" : a.provider === "graph" ? "microsoft" : null;
    if (!provider) return navigate("/onboarding/imap");
    try {
      window.location.assign(
        await backend.worker.connectStart(provider, {
          returnTo: `${window.location.origin}/onboarding`,
          loginHint: a.email,
          accountId: a._id,
        }),
      );
    } catch (e) {
      fail(e);
    }
  };
  return (
    <li className="ak-settings__card" data-status={a.status}>
      <div className="ak-settings__card-head">
        <div>
          <span className="ak-settings__card-title">{a.email}</span>
          <span className="ak-settings__hint">
            {PROVIDER[a.provider]} · {STATUS[a.status]}
            {a.lastSyncAt ? ` · synced ${fullDate(a.lastSyncAt, tz)}` : ""}
          </span>
        </div>
        {a.status === "reauth_required" && (
          <Button variant="primary" size="sm" onClick={() => void reconnect()}>
            Reconnect
          </Button>
        )}
      </div>
      <Row label="Display name" htmlFor={`name-${a._id}`} hint="Shown in the sidebar and the From menu.">
        <input
          id={`name-${a._id}`}
          className="ak-input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() =>
            name !== (a.displayName ?? "") &&
            void m.updateAccount({ accountId: a._id, displayName: name.trim() }).catch(fail)
          }
        />
      </Row>
      <Row label="History" hint="How far back Akatsuki syncs. Older mail stays searchable at your provider.">
        <select
          className="ak-input ak-input--select"
          aria-label={`History for ${a.email}`}
          value={horizon}
          onChange={(e) => {
            const v = e.target.value;
            const horizonAt = v === "all" ? 0 : a.createdAt - (v === "1y" ? 365 : 90) * DAY;
            void m.setBackfillHorizon({ accountId: a._id, horizonAt }).catch(fail);
          }}
        >
          <option value="90d">90 days</option>
          <option value="1y">1 year</option>
          <option value="all">Everything</option>
        </select>
      </Row>
      <div className="ak-settings__actions">
        {confirm ? (
          <>
            <span className="ak-settings__hint">
              Removing deletes this mailbox's metadata from Akatsuki. Your mail stays with{" "}
              {PROVIDER[a.provider]}.
            </span>
            <Button
              variant="danger"
              size="sm"
              onClick={() =>
                void m
                  .removeAccount({ accountId: a._id })
                  .then(() => toast({ title: `Removing ${a.email}.` }))
                  .catch(fail)
              }
            >
              Remove {a.email}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setConfirm(false)}>
              Keep it
            </Button>
          </>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            disabled={a.status === "removing"}
            onClick={() => setConfirm(true)}
          >
            Remove mailbox
          </Button>
        )}
      </div>
    </li>
  );
}

export function IdentitiesSection() {
  const backend = useBackend();
  const identities = backend.useIdentities();
  const accounts = backend.useAccounts() ?? [];
  return (
    <Section
      title="Identities and signatures"
      description="The addresses you send from. Gmail send-as aliases and Outlook aliases appear here after sync."
    >
      {identities === undefined ? null : identities.length === 0 ? (
        <Empty>Connect a mailbox to set up how you send.</Empty>
      ) : (
        <ul className="ak-settings__cards">
          {identities.map((i) => (
            <IdentityCard
              key={i._id}
              identity={i}
              accountEmail={accounts.find((a) => a._id === i.accountId)?.email}
            />
          ))}
        </ul>
      )}
    </Section>
  );
}

const SIGNATURE_MAX = 10 * 1024;

function IdentityCard({ identity: i, accountEmail }: { identity: IdentityView; accountEmail?: string }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const [name, setName] = useState(i.name ?? "");
  const [replyTo, setReplyTo] = useState(i.replyTo ?? "");
  const [signature, setSignature] = useState(i.signatureHtml ? htmlToText(i.signatureHtml) : "");
  const [saving, setSaving] = useState(false);
  const html = signature.trim() ? textToHtml(signature) : "";
  const dirty =
    name !== (i.name ?? "") ||
    replyTo !== (i.replyTo ?? "") ||
    html !== (i.signatureHtml ? textToHtml(htmlToText(i.signatureHtml)) : "");
  const save = async () => {
    if (html.length > SIGNATURE_MAX)
      return toast({ title: "Signatures are limited to 10 KB.", tone: "error" });
    setSaving(true);
    try {
      await m.updateIdentity({
        identityId: i._id,
        name: name.trim(),
        replyTo: replyTo.trim(),
        signatureHtml: html,
      });
      toast({ title: "Identity saved." });
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
    } finally {
      setSaving(false);
    }
  };
  return (
    <li className="ak-settings__card">
      <div className="ak-settings__card-head">
        <div>
          <span className="ak-settings__card-title">{i.email}</span>
          {accountEmail && accountEmail !== i.email && (
            <span className="ak-settings__hint">via {accountEmail}</span>
          )}
        </div>
        {i.isDefault ? (
          <Badge tone="accent">Default</Badge>
        ) : (
          <Row label="Default">
            <Switch
              label={`Send from ${i.email} by default`}
              checked={false}
              onChange={() => void m.updateIdentity({ identityId: i._id, isDefault: true })}
            />
          </Row>
        )}
      </div>
      <Row label="Name" htmlFor={`idn-${i._id}`}>
        <input
          id={`idn-${i._id}`}
          className="ak-input"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </Row>
      <Row label="Reply-To" htmlFor={`rt-${i._id}`} hint="Optional. Replies go here instead.">
        <input
          id={`rt-${i._id}`}
          className="ak-input"
          type="email"
          value={replyTo}
          onChange={(e) => setReplyTo(e.target.value)}
        />
      </Row>
      <label className="ak-field__label" htmlFor={`sig-${i._id}`}>
        Signature
      </label>
      <textarea
        id={`sig-${i._id}`}
        className="ak-textarea"
        rows={4}
        value={signature}
        onChange={(e) => setSignature(e.target.value)}
      />
      <div className="ak-settings__actions">
        <Button variant="primary" size="sm" disabled={!dirty} loading={saving} onClick={() => void save()}>
          Save
        </Button>
      </div>
    </li>
  );
}
