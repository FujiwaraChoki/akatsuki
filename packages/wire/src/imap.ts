// IMAP/SMTP host autodetect for the "Connect IMAP" forms (ARCHITECTURE §6.7 step 5, D-396, D-407). The worker
// owns discovery (`GET /v1/connect/imap/discover?email=`: built-in table, the domain's autoconfig file, the
// Thunderbird ISPDB, all through its SSRF-guarded client). When it knows nothing (404) or cannot be reached, a
// client falls back to the conventional `imap.<domain>` / `smtp.<domain>` guess, which the user can edit.
// Moved here from apps/web/src/onboarding/imapDiscovery.ts so web and mobile share one implementation.
import type { ImapDiscovery, TlsMode } from "./worker.ts";

export type ImapEndpointLike = { host: string; port: number; tls: TlsMode };

/** Where a pre-filled form came from: the worker's answer, or the local guess. */
export type ImapDiscoveryResult = Omit<ImapDiscovery, "source"> & {
  source: ImapDiscovery["source"] | "guess";
};

/** The lowercased domain of a plausible public address (same rules as `email_domain` in crates/providers). */
export function emailDomain(email: string): string | null {
  const e = email.trim();
  const at = e.lastIndexOf("@");
  if (at <= 0) return null;
  const d = e
    .slice(at + 1)
    .replace(/\.+$/, "")
    .toLowerCase();
  if (!d || d.length > 253 || !d.includes(".")) return null;
  if (/^[0-9.]+$/.test(d) || d.includes(":") || d.startsWith("[")) return null; // IP literals (D-407)
  if (d === "localhost" || /\.(localhost|local|internal)$/.test(d)) return null;
  const ok = d
    .split(".")
    .every(
      (l) =>
        l.length > 0 && l.length <= 63 && /^[a-z0-9-]+$/.test(l) && !l.startsWith("-") && !l.endsWith("-"),
    );
  return ok ? d : null;
}

/** Last resort: `imap.<domain>:993` (implicit TLS) and `smtp.<domain>:587` (STARTTLS), marked as a guess. */
export function guessImap(email: string): ImapDiscoveryResult | null {
  const d = emailDomain(email);
  if (!d) return null;
  return {
    imap: { host: `imap.${d}`, port: 993, tls: "implicit" },
    smtp: { host: `smtp.${d}`, port: 587, tls: "starttls" },
    username: email.trim(),
    source: "guess",
    appPasswordRequired: false,
  };
}

function isEndpoint(v: unknown): v is ImapEndpointLike {
  const e = v as ImapEndpointLike;
  return (
    typeof e === "object" &&
    e !== null &&
    typeof e.host === "string" &&
    e.host.length > 0 &&
    e.host.length <= 253 &&
    Number.isInteger(e.port) &&
    e.port > 0 &&
    e.port < 65536 &&
    (e.tls === "implicit" || e.tls === "starttls")
  );
}

/** Validates the worker's JSON (`Discovered` in crates/providers, camelCase); null when it is not usable. */
export function parseImapDiscovery(v: unknown): ImapDiscovery | null {
  const d = v as Partial<ImapDiscovery> | null;
  if (typeof d !== "object" || d === null || !isEndpoint(d.imap) || !isEndpoint(d.smtp)) return null;
  const source = d.source === "autoconfig" || d.source === "ispdb" ? d.source : "well_known";
  return {
    imap: { host: d.imap.host, port: d.imap.port, tls: d.imap.tls },
    smtp: { host: d.smtp.host, port: d.smtp.port, tls: d.smtp.tls },
    username: typeof d.username === "string" ? d.username : "",
    source,
    appPasswordRequired: d.appPasswordRequired === true,
  };
}

/**
 * Worker first (`ask` performs the request and resolves to its JSON, or throws), then the guess. Never throws;
 * null only when the address has no usable domain.
 */
export async function discoverImapSettings(
  email: string,
  ask: (() => Promise<unknown>) | null,
): Promise<ImapDiscoveryResult | null> {
  if (!emailDomain(email)) return null;
  if (ask) {
    try {
      const found = parseImapDiscovery(await ask());
      if (found) return { ...found, username: found.username || email.trim() };
    } catch {
      // Nothing known (404), offline, rate limited or not entitled: fall back to the guess.
    }
  }
  return guessImap(email);
}

/** The usual TLS mode of a mail port (993/465 implicit, 143/587/25/2525 STARTTLS), for a port the user typed. */
export function tlsForPort(port: number, kind: "imap" | "smtp"): TlsMode {
  if (kind === "imap") return port === 143 ? "starttls" : "implicit";
  return port === 465 ? "implicit" : "starttls";
}
