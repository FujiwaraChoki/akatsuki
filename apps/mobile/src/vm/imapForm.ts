// The "Connect IMAP" sheet (ARCHITECTURE §6.7 step 5, §7.2 `POST /v1/connect/imap`). Typing the address fills in
// the servers from the worker's discovery (`GET /v1/connect/imap/discover`, shared helpers in packages/wire) or,
// when it knows nothing, the conventional guess; fields the user has edited are never overwritten. Pure state.
import type { ImapConnect, ImapDiscoveryResult, TlsMode } from "@akatsuki/wire";
import { emailDomain, tlsForPort } from "@akatsuki/wire";

export interface ImapForm {
  email: string;
  username: string;
  password: string;
  imapHost: string;
  imapPort: string;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: string;
  smtpTls: TlsMode;
  /** Fields the user typed into (autodetect leaves them alone). */
  touched: ReadonlySet<ImapField>;
  /** Where the server fields came from, for the note under them. */
  source: ImapDiscoveryResult["source"] | null;
  /** The provider wants an app password (Gmail, iCloud, Yahoo, Fastmail …). */
  appPasswordRequired: boolean;
  /** The domain the current suggestion is for (a new domain asks again). */
  detectedFor: string | null;
}

export type ImapField = "username" | "imapHost" | "imapPort" | "smtpHost" | "smtpPort";

export const EMPTY_IMAP_FORM: ImapForm = {
  email: "",
  username: "",
  password: "",
  imapHost: "",
  imapPort: "993",
  imapTls: "implicit",
  smtpHost: "",
  smtpPort: "465",
  smtpTls: "implicit",
  touched: new Set(),
  source: null,
  appPasswordRequired: false,
  detectedFor: null,
};

/** A field the user edits by hand; ports also update their TLS mode. */
export function editImapField(f: ImapForm, field: ImapField | "email" | "password", value: string): ImapForm {
  if (field === "email" || field === "password") return { ...f, [field]: value };
  const touched = new Set(f.touched);
  touched.add(field);
  const next: ImapForm = { ...f, [field]: value, touched };
  if (field === "imapPort") next.imapTls = tlsForPort(Number(value), "imap");
  if (field === "smtpPort") next.smtpTls = tlsForPort(Number(value), "smtp");
  return next;
}

/** Switches a server between SSL/TLS and STARTTLS (for providers on unusual ports); counts as an edit. */
export function toggleTls(f: ImapForm, kind: "imap" | "smtp"): ImapForm {
  const touched = new Set(f.touched);
  touched.add(kind === "imap" ? "imapPort" : "smtpPort");
  return kind === "imap"
    ? { ...f, imapTls: f.imapTls === "implicit" ? "starttls" : "implicit", touched }
    : { ...f, smtpTls: f.smtpTls === "implicit" ? "starttls" : "implicit", touched };
}

/** The domain to autodetect for, when it differs from the one already detected. */
export function domainToDetect(f: Pick<ImapForm, "email" | "detectedFor">): string | null {
  const d = emailDomain(f.email);
  return d && d !== f.detectedFor ? d : null;
}

/** Applies a discovery result to the fields the user has not edited. */
export function applyDiscovery(f: ImapForm, d: ImapDiscoveryResult, domain: string): ImapForm {
  const keep = (k: ImapField) => f.touched.has(k);
  return {
    ...f,
    username: keep("username") ? f.username : d.username || f.email.trim(),
    imapHost: keep("imapHost") ? f.imapHost : d.imap.host,
    imapPort: keep("imapPort") ? f.imapPort : String(d.imap.port),
    imapTls: keep("imapPort") ? f.imapTls : d.imap.tls,
    smtpHost: keep("smtpHost") ? f.smtpHost : d.smtp.host,
    smtpPort: keep("smtpPort") ? f.smtpPort : String(d.smtp.port),
    smtpTls: keep("smtpPort") ? f.smtpTls : d.smtp.tls,
    source: d.source,
    appPasswordRequired: d.appPasswordRequired,
    detectedFor: domain,
  };
}

/** The note under the server fields. */
export function discoveryNote(f: ImapForm): string | null {
  if (!f.source) return null;
  const base =
    f.source === "guess"
      ? "These servers are a guess from your domain. Check them with your provider if connecting fails."
      : "Servers filled in for your provider. You can change them.";
  return f.appPasswordRequired
    ? `${base} This provider needs an app password, not your normal password.`
    : base;
}

// A hostname or IPv4 address; the worker decides which addresses it may reach (public only, D-398).
const HOST_RE = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/i;

function port(s: string): number | null {
  const n = Number(s.trim());
  return Number.isInteger(n) && n > 0 && n < 65536 ? n : null;
}

/** Problems that keep Connect disabled, in display order (the worker checks the rest live). */
export function imapProblems(f: ImapForm): string[] {
  const out: string[] = [];
  if (!emailDomain(f.email)) out.push("Enter your email address.");
  if (!f.password) out.push("Enter the password.");
  if (!HOST_RE.test(f.imapHost.trim())) out.push("Enter the IMAP server.");
  if (port(f.imapPort) === null) out.push("The IMAP port is a number from 1 to 65535.");
  if (!HOST_RE.test(f.smtpHost.trim())) out.push("Enter the SMTP server.");
  if (port(f.smtpPort) === null) out.push("The SMTP port is a number from 1 to 65535.");
  return out;
}

/** The `POST /v1/connect/imap` body. */
export function imapConnectBody(f: ImapForm): ImapConnect {
  const email = f.email.trim().toLowerCase();
  return {
    email,
    username: f.username.trim() || email,
    password: f.password,
    imap: { host: f.imapHost.trim().toLowerCase(), port: port(f.imapPort) ?? 993, tls: f.imapTls },
    smtp: { host: f.smtpHost.trim().toLowerCase(), port: port(f.smtpPort) ?? 465, tls: f.smtpTls },
  };
}

export function tlsLabel(tls: TlsMode): string {
  return tls === "implicit" ? "SSL/TLS" : "STARTTLS";
}
