// Onboarding: connect a mailbox (ARCHITECTURE §6.7). Gmail and Outlook go through the worker's OAuth start
// (`POST /v1/connect/{google|microsoft}/start` → provider → worker callback → back here with
// `?status=pending&grant=` or `?status=error&code=`); this page then redeems the grant with
// `POST /v1/connect/complete`, which only the user who started the flow can do (D-395). IMAP posts the settings to
// `/v1/connect/imap`, which tests LOGIN and SMTP AUTH live; host autodetect fills the server fields (D-264, D-396).
// First Light (SPEC §3.2): no cards and no icons in pink circles; the providers are the landing's ruled specimen
// rows, chosen with 1, 2 and 3.
import { Button, errorCopy, Heading, Kbd } from "@akatsuki/ui";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { routeSlot } from "../app/extensions.ts";
import { navigate } from "../app/router.tsx";
import { useBackend } from "../backend/context.tsx";
import type { ImapDiscovery, ImapEndpoint } from "../backend/types.ts";
import { codeOf } from "../components/ErrorState.tsx";
import { IconArrowRight } from "../components/Icons.tsx";
import { echo } from "../mail/statusEcho.ts";

/** Where the worker sends the browser back after OAuth (must be on its returnTo allow-list, §6.7). */
export function returnToUrl(): string {
  return `${window.location.origin}/onboarding`;
}

export function Onboarding({ step, state }: { step?: "imap"; state: "checkout" | "connect" | "done" }) {
  if (state === "checkout") {
    const Checkout = routeSlot("checkout");
    return Checkout ? <Checkout /> : <CheckoutUnavailable />;
  }
  return step === "imap" ? <ImapForm /> : <ConnectChooser />;
}

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <main className="ak-onboarding">
      <a className="ak-auth__dot" href="https://akatsuki.vc">
        <span className="ak-visually-hidden">Akatsuki</span>
      </a>
      <div className="ak-onboarding__inner">{children}</div>
    </main>
  );
}

function CheckoutUnavailable() {
  const backend = useBackend();
  return (
    <Frame>
      <Heading level={1} variant="display" className="ak-onboarding__title">
        Start your trial.
      </Heading>
      <p className="ak-onboarding__lede">
        Hosted Akatsuki needs a Pro subscription (14 days free). Checkout is not available in this build yet.
      </p>
      <Button variant="ghost" onClick={() => void backend.auth.signOut()}>
        Sign out
      </Button>
    </Frame>
  );
}

type ConnectResult = { status: "pending"; grant: string } | { status: "error"; code: string };

/** Reads and clears `?status=…` left by the worker's OAuth callback (the grant leaves the URL at once). */
function useConnectResult(): ConnectResult | null {
  const [result] = useState((): ConnectResult | null => {
    const p = new URLSearchParams(window.location.search);
    const status = p.get("status");
    const grant = p.get("grant");
    if (status === "pending" && grant) return { status, grant };
    if (status === "error") return { status, code: p.get("code") ?? "internal" };
    return null;
  });
  useEffect(() => {
    if (result) window.history.replaceState(null, "", window.location.pathname);
  }, [result]);
  return result;
}

/** Why a grant could not be redeemed, in words that also make sense to someone sent a stranger's link. */
function completeError(code: string): string {
  if (code === "invalid_argument")
    return "This connection was started from another Akatsuki sign-in, so it was not added. Start again from your own account.";
  if (code === "not_found") return "This connection expired. Start again.";
  return errorCopy(code).title;
}

function ConnectChooser() {
  const backend = useBackend();
  const m = backend.useMutations();
  const result = useConnectResult();
  const [busy, setBusy] = useState<null | "google" | "microsoft" | "skip" | "finishing">(
    result?.status === "pending" ? "finishing" : null,
  );
  const [error, setError] = useState<string | null>(
    result?.status === "error" ? errorCopy(result.code).title : null,
  );
  const handled = useRef(false);

  useEffect(() => {
    if (result?.status !== "pending" || handled.current) return;
    handled.current = true;
    backend.worker
      .connectComplete(result.grant)
      .then(async (r) => {
        // The status bar says it (SPEC §3.9: no persistent "connected" toast).
        echo(r.reconnected ? "Mailbox reconnected" : "Mailbox connected · syncing");
        await m.updateProfile({ onboarding: "done" }).catch(() => undefined);
        navigate("/inbox", { replace: true });
      })
      .catch((e) => {
        setError(completeError(codeOf(e)));
        setBusy(null);
      });
  }, [result, m, backend.worker]);

  const start = async (provider: "google" | "microsoft") => {
    setBusy(provider);
    setError(null);
    try {
      const url = await backend.worker.connectStart(provider, { returnTo: returnToUrl() });
      window.location.assign(url);
    } catch (e) {
      setError(errorCopy(codeOf(e)).title);
      setBusy(null);
    }
  };

  const skip = async () => {
    setBusy("skip");
    await m.updateProfile({ onboarding: "done" }).catch(() => undefined);
    navigate("/inbox", { replace: true });
  };

  // 1, 2, 3 choose a provider (SPEC §3.2).
  const choose = useRef<(n: number) => void>(() => undefined);
  choose.current = (n) => {
    if (busy) return;
    if (n === 1) void start("google");
    else if (n === 2) void start("microsoft");
    else if (n === 3) navigate("/onboarding/imap");
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || !/^[123]$/.test(e.key)) return;
      const t = e.target as HTMLElement | null;
      if (t && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
      e.preventDefault();
      choose.current(Number(e.key));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <Frame>
      <Heading level={1} variant="display" className="ak-onboarding__title">
        Bring your inbox.
      </Heading>
      <p className="ak-onboarding__lede">
        Akatsuki syncs headers and short previews. Message bodies stay with your provider and on this device.
      </p>
      <ul className="ak-connect">
        <ConnectCard
          title="Gmail"
          detail="Google Workspace and gmail.com"
          n={1}
          busy={busy === "google"}
          onClick={() => start("google")}
        />
        <ConnectCard
          title="Outlook"
          detail="Microsoft 365, outlook.com and hotmail.com"
          n={2}
          busy={busy === "microsoft"}
          onClick={() => start("microsoft")}
        />
        <ConnectCard
          title="Other (IMAP)"
          detail="iCloud, Fastmail, Yahoo, your own server"
          n={3}
          onClick={() => navigate("/onboarding/imap")}
        />
      </ul>
      {busy === "finishing" && (
        <p className="ak-onboarding__lede" role="status">
          Adding your mailbox…
        </p>
      )}
      {error && (
        <p className="ak-onboarding__error" role="alert">
          {error}
        </p>
      )}
      <button
        type="button"
        className="ak-auth__text ak-onboarding__skip"
        onClick={skip}
        disabled={busy === "skip"}
      >
        I'll do this later
      </button>
    </Frame>
  );
}

function ConnectCard({
  title,
  detail,
  n,
  busy,
  onClick,
}: {
  title: string;
  detail: string;
  n: number;
  busy?: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        className="ak-connect__card"
        onClick={onClick}
        aria-busy={busy || undefined}
        disabled={busy}
      >
        <span className="ak-connect__text">
          <span className="ak-connect__title">{title}</span>
          <span className="ak-connect__detail">{detail}</span>
        </span>
        <Kbd keys={String(n)} className="ak-connect__key" />
        <IconArrowRight className="ak-connect__arrow" />
      </button>
    </li>
  );
}

function EndpointFields({
  legend,
  value,
  onChange,
}: {
  legend: string;
  value: ImapEndpoint;
  onChange: (v: ImapEndpoint) => void;
}) {
  const id = useId();
  return (
    <fieldset className="ak-endpoint">
      <legend className="ak-field__label">{legend}</legend>
      <div className="ak-endpoint__row">
        <label htmlFor={`${id}-host`} className="ak-visually-hidden">
          {legend} host
        </label>
        <input
          id={`${id}-host`}
          className="ak-input"
          value={value.host}
          onChange={(e) => onChange({ ...value, host: e.target.value.trim() })}
          placeholder="imap.example.com"
          autoComplete="off"
          spellCheck={false}
        />
        <label htmlFor={`${id}-port`} className="ak-visually-hidden">
          {legend} port
        </label>
        <input
          id={`${id}-port`}
          className="ak-input ak-input--port"
          inputMode="numeric"
          value={String(value.port)}
          onChange={(e) => onChange({ ...value, port: Number(e.target.value.replace(/\D/g, "")) || 0 })}
        />
        <label htmlFor={`${id}-tls`} className="ak-visually-hidden">
          {legend} security
        </label>
        <select
          id={`${id}-tls`}
          className="ak-input ak-input--select"
          value={value.tls}
          onChange={(e) => onChange({ ...value, tls: e.target.value as ImapEndpoint["tls"] })}
        >
          <option value="implicit">SSL/TLS</option>
          <option value="starttls">STARTTLS</option>
        </select>
      </div>
    </fieldset>
  );
}

const SOURCE_NOTE: Record<ImapDiscovery["source"], string> = {
  well_known: "Settings found for this provider.",
  autoconfig: "Settings found in your domain's configuration.",
  ispdb: "Settings found in the Thunderbird provider database.",
  guess: "We guessed these settings. Check them with your provider.",
};

function ImapForm() {
  const backend = useBackend();
  const m = backend.useMutations();
  const ids = { email: useId(), user: useId(), pass: useId() };
  const [email, setEmail] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [found, setFound] = useState<ImapDiscovery | null>(null);
  const [imap, setImap] = useState<ImapEndpoint>({ host: "", port: 993, tls: "implicit" });
  const [smtp, setSmtp] = useState<ImapEndpoint>({ host: "", port: 587, tls: "starttls" });
  const [showServers, setShowServers] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lastLookup = useRef("");

  const detect = async () => {
    const addr = email.trim();
    if (!addr.includes("@") || addr === lastLookup.current) return;
    lastLookup.current = addr;
    setDetecting(true);
    const d = await backend.worker.discoverImap(addr);
    setDetecting(false);
    if (lastLookup.current !== addr) return;
    setFound(d);
    if (d) {
      setImap(d.imap);
      setSmtp(d.smtp);
      setUsername((u) => u || d.username || addr);
      if (d.source === "guess") setShowServers(true);
    } else setShowServers(true);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    if (!email.includes("@") || !password || !imap.host || !smtp.host || !imap.port || !smtp.port) {
      setError("Fill in your address, password and both servers.");
      setShowServers(true);
      return;
    }
    setBusy(true);
    try {
      await backend.worker.connectImap({
        email: email.trim(),
        username: username.trim() || email.trim(),
        password,
        imap,
        smtp,
      });
      setPassword("");
      echo("Mailbox connected · syncing");
      await m.updateProfile({ onboarding: "done" }).catch(() => undefined);
      navigate("/inbox", { replace: true });
    } catch (err) {
      setError(errorCopy(codeOf(err)).title);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Frame>
      <Heading level={1} variant="display" className="ak-onboarding__title ak-onboarding__title--step">
        Any mailbox, one inbox.
      </Heading>
      <form className="ak-imap" onSubmit={submit} noValidate>
        <label htmlFor={ids.email} className="ak-field__label">
          Email address
        </label>
        <input
          id={ids.email}
          className="ak-input"
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          onBlur={detect}
          placeholder="you@example.com"
          // biome-ignore lint/a11y/noAutofocus: first field of a dedicated form page
          autoFocus
        />
        <label htmlFor={ids.pass} className="ak-field__label">
          Password
        </label>
        <input
          id={ids.pass}
          className="ak-input"
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onFocus={detect}
        />
        <p className="ak-imap__status" aria-live="polite">
          {detecting
            ? "Looking up your server settings…"
            : found
              ? `${SOURCE_NOTE[found.source]}${found.appPasswordRequired ? " This provider needs an app password, not your normal one." : ""}`
              : ""}
        </p>
        <button
          type="button"
          className="ak-imap__toggle"
          aria-expanded={showServers}
          onClick={() => setShowServers((s) => !s)}
        >
          {showServers ? "Hide server settings" : "Server settings"}
        </button>
        {showServers && (
          <div className="ak-imap__servers">
            <label htmlFor={ids.user} className="ak-field__label">
              Username
            </label>
            <input
              id={ids.user}
              className="ak-input"
              autoComplete="username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder={email || "you@example.com"}
            />
            <EndpointFields legend="Incoming (IMAP)" value={imap} onChange={setImap} />
            <EndpointFields legend="Outgoing (SMTP)" value={smtp} onChange={setSmtp} />
          </div>
        )}
        {error && (
          <p className="ak-onboarding__error" role="alert">
            {error}
          </p>
        )}
        <div className="ak-auth__actions">
          <Button type="submit" variant="brand" size="lg" loading={busy}>
            Connect
          </Button>
          <button type="button" className="ak-auth__text" onClick={() => navigate("/onboarding")}>
            Back
          </button>
        </div>
        <p className="ak-imap__privacy">
          Your password is sent once over TLS to the Akatsuki sync service, which tests it and stores it
          encrypted.
        </p>
      </form>
    </Frame>
  );
}
