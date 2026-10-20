// Settings: devices and sessions (§4.2 `devices.*`, Better Auth sessions; each desktop has its own session,
// §14.5), and your data: export everything Akatsuki keeps about you as JSON, or delete the account
// (`me.deleteAccount` → cancel billing at apps/api → Better Auth `deleteUser`, §4.2).
import { Badge, Button, errorMessage, useToast } from "@akatsuki/ui";
import { useCallback, useEffect, useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { MeView, SessionInfo } from "../../backend/types.ts";
import { fullDate } from "../../lib/format.ts";
import { deviceId } from "../../lib/ids.ts";
import { Empty, Row, Section } from "../ui.tsx";

const KIND: Record<string, string> = { web: "Web", desktop: "Desktop", mobile: "Mobile" };

export function DevicesSection({ me }: { me: MeView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const devices = backend.useDevices();
  const { toast } = useToast();
  const tz = me.profile.timezone;
  const [sessions, setSessions] = useState<SessionInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    backend.auth
      .listSessions()
      .then(setSessions)
      .catch((e) => setError(errorMessage(e)));
  }, [backend.auth]);
  useEffect(load, [load]);
  const mine = deviceId();
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  return (
    <>
      <Section title="Devices" description="Every browser and app you use with Akatsuki.">
        {devices === undefined ? null : devices.length === 0 ? (
          <Empty>No devices yet.</Empty>
        ) : (
          <ul className="ak-settings__cards">
            {[...devices]
              .sort((a, b) => b.lastSeenAt - a.lastSeenAt)
              .map((d) => (
                <li key={d._id} className="ak-settings__card ak-split">
                  <div className="ak-split__text">
                    <span className="ak-settings__card-title">
                      {d.name} {d.deviceId === mine && <Badge tone="accent">This browser</Badge>}
                    </span>
                    <span className="ak-settings__hint">
                      {KIND[d.kind] ?? d.kind} · version {d.appVersion} · last seen{" "}
                      {fullDate(d.lastSeenAt, tz)}
                    </span>
                  </div>
                  {d.deviceId !== mine && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void m
                          .removeDevice({ deviceId: d.deviceId })
                          .then(() => toast({ title: `${d.name} removed. Sign it out under Sessions too.` }))
                          .catch(fail)
                      }
                    >
                      Remove
                    </Button>
                  )}
                </li>
              ))}
          </ul>
        )}
      </Section>
      <Section
        title="Sessions"
        description="Where you are signed in. Signing a session out takes effect within 15 minutes."
      >
        {error && <p className="ak-compose__error">{error}</p>}
        {sessions === null ? null : sessions.length === 0 ? (
          <Empty>No other sessions.</Empty>
        ) : (
          <ul className="ak-settings__cards">
            {sessions.map((s) => (
              <li key={s.id} className="ak-settings__card ak-split">
                <div className="ak-split__text">
                  <span className="ak-settings__card-title">
                    {s.userAgent ? shortAgent(s.userAgent) : "Unknown browser"}{" "}
                    {s.current && <Badge tone="accent">This session</Badge>}
                  </span>
                  <span className="ak-settings__hint">
                    Signed in {fullDate(s.createdAt, tz)} · expires {fullDate(s.expiresAt, tz)}
                  </span>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    s.current
                      ? void backend.auth.signOut()
                      : void backend.auth
                          .revokeSession(s.token)
                          .then(() => {
                            toast({ title: "Signed out that session." });
                            load();
                          })
                          .catch(fail)
                  }
                >
                  Sign out
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </>
  );
}

function shortAgent(ua: string): string {
  const browser = /Firefox\//.test(ua)
    ? "Firefox"
    : /Edg\//.test(ua)
      ? "Edge"
      : /Chrome\//.test(ua)
        ? "Chrome"
        : /Safari\//.test(ua)
          ? "Safari"
          : null;
  const os = /Mac OS X/.test(ua)
    ? "macOS"
    : /Windows/.test(ua)
      ? "Windows"
      : /Android/.test(ua)
        ? "Android"
        : /iPhone|iPad/.test(ua)
          ? "iOS"
          : /Linux/.test(ua)
            ? "Linux"
            : null;
  return browser && os ? `${browser} on ${os}` : ua.slice(0, 60);
}

/** Everything the client can read about the user, without message bodies or keys. */
export function buildExport(data: Record<string, unknown>): string {
  return JSON.stringify(
    {
      format: "akatsuki-export",
      version: 1,
      exportedAt: new Date().toISOString(),
      note: "Mail stays with your provider. Message bodies are never stored by Akatsuki; AI keys never leave your devices.",
      ...data,
    },
    null,
    2,
  );
}

export function DataSection({ me }: { me: MeView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const accounts = backend.useAccounts();
  const identities = backend.useIdentities();
  const labels = backend.useLabels();
  const splits = backend.useSplits();
  const rules = backend.useRules();
  const snippets = backend.useSnippets();
  const devices = backend.useDevices();
  const ai = backend.useAiSettings();
  const billing = backend.useBilling();
  const [confirm, setConfirm] = useState("");
  const [deleting, setDeleting] = useState(false);

  const download = () => {
    const json = buildExport({
      profile: { email: me.email, name: me.name, timezone: me.profile.timezone, prefs: me.profile.prefs },
      accounts: (accounts ?? []).map(({ _id, provider, email, displayName, status, createdAt }) => ({
        _id,
        provider,
        email,
        displayName,
        status,
        createdAt,
      })),
      identities,
      labels: (labels ?? []).map(({ _id, accountId, name, kind, role }) => ({
        _id,
        accountId,
        name,
        kind,
        role,
      })),
      splits,
      rules,
      snippets,
      devices,
      aiSettings: ai,
      billing: billing?.billing,
    });
    const url = URL.createObjectURL(new Blob([json], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `akatsuki-export-${new Date().toISOString().slice(0, 10)}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    toast({ title: "Export downloaded." });
  };

  const remove = async () => {
    setDeleting(true);
    try {
      // Cancel Pro first: the purge removes the billing row apps/api needs to find the Stripe customer.
      const st = billing?.billing.status;
      if (backend.api.configured && st && st !== "none" && st !== "canceled")
        await backend.api.cancel({ immediately: true }).catch(() => undefined);
      // Deletes the app rows and the Better Auth login, sessions included (D-393).
      await m.deleteAccount({ confirm: "DELETE" });
      await backend.auth.deleteUser().catch(() => undefined);
      await backend.auth.signOut();
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
      setDeleting(false);
    }
  };

  return (
    <>
      <Section
        title="Export"
        description="Your settings, mailboxes, labels, splits, rules, snippets and devices as one JSON file."
      >
        <Button onClick={download}>Download my data</Button>
      </Section>
      <Section
        title="Delete account"
        description="Stops syncing, deletes everything Akatsuki keeps about you (metadata, settings, devices), cancels Pro at once and signs you out. Your mail stays with your provider. This cannot be undone."
      >
        <Row label="Type DELETE to confirm" htmlFor="del-confirm">
          <input
            id="del-confirm"
            className="ak-input ak-input--mono"
            value={confirm}
            autoComplete="off"
            onChange={(e) => setConfirm(e.target.value)}
          />
        </Row>
        <div className="ak-settings__actions">
          <Button
            variant="danger"
            disabled={confirm !== "DELETE"}
            loading={deleting}
            onClick={() => void remove()}
          >
            Delete my account
          </Button>
        </div>
      </Section>
    </>
  );
}
